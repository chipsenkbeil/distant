use std::{fmt, io};

use crate::net::common::Request;
use log::*;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::client::{Channel, ChannelExt};
use crate::constants::CLIENT_SEARCHER_CAPACITY;
use crate::protocol::{self, SearchId, SearchQuery, SearchQueryMatch};

/// Represents a searcher for files, directories, and symlinks on the filesystem
pub struct Searcher {
    channel: Channel,
    id: SearchId,
    query: SearchQuery,
    task: JoinHandle<()>,
    rx: mpsc::Receiver<SearchQueryMatch>,
}

impl fmt::Debug for Searcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Searcher")
            .field("id", &self.id)
            .field("query", &self.query)
            .finish()
    }
}

impl Searcher {
    /// Creates a searcher for some query
    pub async fn search(mut channel: Channel, query: SearchQuery) -> io::Result<Self> {
        trace!("Searching using {query:?}",);

        // Submit our run request and get back a mailbox for responses
        let mut mailbox = channel
            .mail(Request::new(protocol::Msg::Single(
                protocol::Request::Search {
                    query: query.clone(),
                },
            )))
            .await?;

        let (tx, rx) = mpsc::channel(CLIENT_SEARCHER_CAPACITY);

        // Wait to get the confirmation of watch as either ok or error
        let mut queue: Vec<SearchQueryMatch> = Vec::new();
        let mut search_id = None;
        while let Some(res) = mailbox.next().await {
            for data in res.payload.into_vec() {
                match data {
                    // If we get results before the started indicator, queue them up
                    protocol::Response::SearchResults { matches, .. } => {
                        queue.extend(matches);
                    }

                    // Once we get the started indicator, mark as ready to go
                    protocol::Response::SearchStarted { id } => {
                        trace!("[Query {id}] Searcher has started");
                        search_id = Some(id);
                    }

                    // If we get an explicit error, convert and return it
                    protocol::Response::Error(x) => return Err(io::Error::from(x)),

                    // Otherwise, we got something unexpected, and report as such
                    x => return Err(io::Error::other(format!("Unexpected response: {x:?}"))),
                }
            }

            // Exit if we got the confirmation
            // NOTE: Doing this later because we want to make sure the entire payload is processed
            //       first before exiting the loop
            if search_id.is_some() {
                break;
            }
        }

        let search_id = match search_id {
            // Send out any of our queued changes that we got prior to the acknowledgement
            Some(id) => {
                trace!("[Query {id}] Forwarding {} queued matches", queue.len());
                for r#match in queue.drain(..) {
                    if tx.send(r#match).await.is_err() {
                        return Err(io::Error::other(format!(
                            "[Query {id}] Queue search match dropped"
                        )));
                    }
                }
                id
            }

            // If we never received an acknowledgement of search before the mailbox closed,
            // fail with a missing confirmation error
            None => {
                return Err(io::Error::other(
                    "Search query missing started confirmation",
                ))
            }
        };

        // Spawn a task that continues to look for search result events and the conclusion of the
        // search, discarding anything else that it gets
        let task = tokio::spawn({
            async move {
                while let Some(res) = mailbox.next().await {
                    let mut done = false;

                    for data in res.payload.into_vec() {
                        match data {
                            protocol::Response::SearchResults { matches, .. } => {
                                // If we can't queue up a match anymore, we've
                                // been closed and therefore want to quit
                                if tx.is_closed() {
                                    break;
                                }

                                // Otherwise, send over the matches
                                for r#match in matches {
                                    if let Err(x) = tx.send(r#match).await {
                                        error!(
                                            "[Query {search_id}] Searcher failed to send match {:?}",
                                            x.0
                                        );
                                        break;
                                    }
                                }
                            }

                            // Received completion indicator, so close out
                            protocol::Response::SearchDone { .. } => {
                                trace!("[Query {search_id}] Searcher has finished");
                                done = true;
                                break;
                            }

                            _ => continue,
                        }
                    }

                    if done {
                        break;
                    }
                }
            }
        });

        Ok(Self {
            id: search_id,
            query,
            channel,
            task,
            rx,
        })
    }

    /// Returns a reference to the query this searcher is running
    pub fn query(&self) -> &SearchQuery {
        &self.query
    }

    /// Returns true if the searcher is still actively searching
    pub fn is_active(&self) -> bool {
        !self.task.is_finished()
    }

    /// Returns the next match detected by the searcher, or none if the searcher has concluded
    pub async fn next(&mut self) -> Option<SearchQueryMatch> {
        self.rx.recv().await
    }

    /// Cancels the search being performed by the watcher
    pub async fn cancel(&mut self) -> io::Result<()> {
        trace!("[Query {}] Cancelling search", self.id);
        self.channel.cancel_search(self.id).await?;

        // Kill our task that processes inbound matches if we have successfully stopped searching
        self.task.abort();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Tests for Searcher: setup error handling (missing confirmation, error response,
    //! unexpected response), result queuing before started confirmation, is_active lifecycle,
    //! and iteration via next().

    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::net::common::{FramedTransport, InmemoryTransport, Response};
    use test_log::test;
    use tokio::sync::Mutex;

    use super::*;
    use crate::protocol::{
        SearchQueryCondition, SearchQueryMatchData, SearchQueryOptions, SearchQueryPathMatch,
        SearchQuerySubmatch, SearchQueryTarget,
    };
    use crate::Client;

    fn make_session() -> (FramedTransport<InmemoryTransport>, Client) {
        let (t1, t2) = FramedTransport::pair(100);
        (t1, Client::spawn_inmemory(t2, Default::default()))
    }

    #[test(tokio::test)]
    async fn searcher_should_have_query_reflect_ongoing_query() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        // Create a task for searcher as we need to handle the request and a response
        // in a separate async block
        let search_task = {
            let test_query = test_query.clone();
            tokio::spawn(async move { Searcher::search(session.clone_channel(), test_query).await })
        };

        // Wait until we get the request from the session
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();

        // Send back an acknowledgement that a search was started
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::SearchStarted { id: rand::random() },
            ))
            .await
            .unwrap();

        // Get the searcher and verify the query
        let searcher = search_task.await.unwrap().unwrap();
        assert_eq!(searcher.query(), &test_query);
    }

    #[test(tokio::test)]
    async fn searcher_should_support_getting_next_match() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        // Create a task for searcher as we need to handle the request and a response
        // in a separate async block
        let search_task =
            tokio::spawn(
                async move { Searcher::search(session.clone_channel(), test_query).await },
            );

        // Wait until we get the request from the session
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();

        // Send back an acknowledgement that a searcher was created
        let id = rand::random::<SearchId>();
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                protocol::Response::SearchStarted { id },
            ))
            .await
            .unwrap();

        // Get the searcher
        let mut searcher = search_task.await.unwrap().unwrap();

        // Send some matches related to the file
        transport
            .write_frame_for(&Response::new(
                req.id,
                vec![
                    protocol::Response::SearchResults {
                        id,
                        matches: vec![
                            SearchQueryMatch::Path(SearchQueryPathMatch {
                                path: PathBuf::from("/some/path/1"),
                                submatches: vec![SearchQuerySubmatch {
                                    r#match: SearchQueryMatchData::Text("test match".to_string()),
                                    start: 3,
                                    end: 7,
                                }],
                            }),
                            SearchQueryMatch::Path(SearchQueryPathMatch {
                                path: PathBuf::from("/some/path/2"),
                                submatches: vec![SearchQuerySubmatch {
                                    r#match: SearchQueryMatchData::Text("test match 2".to_string()),
                                    start: 88,
                                    end: 99,
                                }],
                            }),
                        ],
                    },
                    protocol::Response::SearchResults {
                        id,
                        matches: vec![SearchQueryMatch::Path(SearchQueryPathMatch {
                            path: PathBuf::from("/some/path/3"),
                            submatches: vec![SearchQuerySubmatch {
                                r#match: SearchQueryMatchData::Text("test match 3".to_string()),
                                start: 5,
                                end: 9,
                            }],
                        })],
                    },
                ],
            ))
            .await
            .unwrap();

        // Verify that the searcher gets the matches, one at a time
        let m = searcher.next().await.expect("Searcher closed unexpectedly");
        assert_eq!(
            m,
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from("/some/path/1"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text("test match".to_string()),
                    start: 3,
                    end: 7,
                }],
            })
        );

        let m = searcher.next().await.expect("Searcher closed unexpectedly");
        assert_eq!(
            m,
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from("/some/path/2"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text("test match 2".to_string()),
                    start: 88,
                    end: 99,
                }],
            }),
        );

        let m = searcher.next().await.expect("Searcher closed unexpectedly");
        assert_eq!(
            m,
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from("/some/path/3"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text("test match 3".to_string()),
                    start: 5,
                    end: 9,
                }],
            })
        );
    }

    #[test(tokio::test)]
    async fn searcher_should_distinguish_match_events_and_only_receive_matches_for_itself() {
        let (mut transport, session) = make_session();

        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        // Create a task for searcher as we need to handle the request and a response
        // in a separate async block
        let search_task =
            tokio::spawn(
                async move { Searcher::search(session.clone_channel(), test_query).await },
            );

        // Wait until we get the request from the session
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();

        // Send back an acknowledgement that a searcher was created
        let id = rand::random();
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                protocol::Response::SearchStarted { id },
            ))
            .await
            .unwrap();

        // Get the searcher
        let mut searcher = search_task.await.unwrap().unwrap();

        // Send a match from the appropriate origin
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                protocol::Response::SearchResults {
                    id,
                    matches: vec![SearchQueryMatch::Path(SearchQueryPathMatch {
                        path: PathBuf::from("/some/path/1"),
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("test match".to_string()),
                            start: 3,
                            end: 7,
                        }],
                    })],
                },
            ))
            .await
            .unwrap();

        // Send a chanmatchge from a different origin
        transport
            .write_frame_for(&Response::new(
                req.id.clone() + "1",
                protocol::Response::SearchResults {
                    id,
                    matches: vec![SearchQueryMatch::Path(SearchQueryPathMatch {
                        path: PathBuf::from("/some/path/2"),
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("test match 2".to_string()),
                            start: 88,
                            end: 99,
                        }],
                    })],
                },
            ))
            .await
            .unwrap();

        // Send a chanmatchge from the appropriate origin
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::SearchResults {
                    id,
                    matches: vec![SearchQueryMatch::Path(SearchQueryPathMatch {
                        path: PathBuf::from("/some/path/3"),
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("test match 3".to_string()),
                            start: 5,
                            end: 9,
                        }],
                    })],
                },
            ))
            .await
            .unwrap();

        // Verify that the searcher gets the matches, one at a time
        let m = searcher.next().await.expect("Searcher closed unexpectedly");
        assert_eq!(
            m,
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from("/some/path/1"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text("test match".to_string()),
                    start: 3,
                    end: 7,
                }],
            })
        );

        let m = searcher.next().await.expect("Watcher closed unexpectedly");
        assert_eq!(
            m,
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from("/some/path/3"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text("test match 3".to_string()),
                    start: 5,
                    end: 9,
                }],
            })
        );
    }

    #[test(tokio::test)]
    async fn searcher_should_stop_receiving_events_if_cancelled() {
        let (mut transport, session) = make_session();

        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        // Create a task for searcher as we need to handle the request and a response
        // in a separate async block
        let search_task =
            tokio::spawn(
                async move { Searcher::search(session.clone_channel(), test_query).await },
            );

        // Wait until we get the request from the session
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();

        // Send back an acknowledgement that a watcher was created
        let id = rand::random::<SearchId>();
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                protocol::Response::SearchStarted { id },
            ))
            .await
            .unwrap();

        // Send some matches from the appropriate origin
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::SearchResults {
                    id,
                    matches: vec![
                        SearchQueryMatch::Path(SearchQueryPathMatch {
                            path: PathBuf::from("/some/path/1"),
                            submatches: vec![SearchQuerySubmatch {
                                r#match: SearchQueryMatchData::Text("test match".to_string()),
                                start: 3,
                                end: 7,
                            }],
                        }),
                        SearchQueryMatch::Path(SearchQueryPathMatch {
                            path: PathBuf::from("/some/path/2"),
                            submatches: vec![SearchQuerySubmatch {
                                r#match: SearchQueryMatchData::Text("test match 2".to_string()),
                                start: 88,
                                end: 99,
                            }],
                        }),
                    ],
                },
            ))
            .await
            .unwrap();

        // Wait a little bit for all matches to be queued
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Create a task for for cancelling as we need to handle the request and a response
        // in a separate async block
        let searcher = Arc::new(Mutex::new(search_task.await.unwrap().unwrap()));

        // Verify that the searcher gets the first match
        let m = searcher
            .lock()
            .await
            .next()
            .await
            .expect("Searcher closed unexpectedly");
        assert_eq!(
            m,
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from("/some/path/1"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text("test match".to_string()),
                    start: 3,
                    end: 7,
                }],
            }),
        );

        // Cancel the search, verify the request is sent out, and respond with ok
        let searcher_2 = Arc::clone(&searcher);
        let cancel_task = tokio::spawn(async move { searcher_2.lock().await.cancel().await });

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();

        transport
            .write_frame_for(&Response::new(req.id.clone(), protocol::Response::Ok))
            .await
            .unwrap();

        // Wait for the cancel to complete
        cancel_task.await.unwrap().unwrap();

        // Send a match that will get ignored
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::SearchResults {
                    id,
                    matches: vec![SearchQueryMatch::Path(SearchQueryPathMatch {
                        path: PathBuf::from("/some/path/3"),
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("test match 3".to_string()),
                            start: 5,
                            end: 9,
                        }],
                    })],
                },
            ))
            .await
            .unwrap();

        // Verify that we get any remaining matches that were received before cancel,
        // but nothing new after that
        assert_eq!(
            searcher.lock().await.next().await,
            Some(SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from("/some/path/2"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text("test match 2".to_string()),
                    start: 88,
                    end: 99,
                }],
            }))
        );
        assert_eq!(searcher.lock().await.next().await, None);
    }

    #[test(tokio::test)]
    async fn searcher_debug_should_include_id_and_query() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        let search_task = {
            let test_query = test_query.clone();
            tokio::spawn(async move { Searcher::search(session.clone_channel(), test_query).await })
        };

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        let id: SearchId = 42;
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::SearchStarted { id },
            ))
            .await
            .unwrap();

        let searcher = search_task.await.unwrap().unwrap();
        let debug_str = format!("{:?}", searcher);
        assert!(debug_str.contains("Searcher"));
        assert!(debug_str.contains("42"));
    }

    #[test(tokio::test)]
    async fn searcher_is_active_should_return_true_while_task_is_running() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        let search_task = {
            let test_query = test_query.clone();
            tokio::spawn(async move { Searcher::search(session.clone_channel(), test_query).await })
        };

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        let id: SearchId = rand::random();
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                protocol::Response::SearchStarted { id },
            ))
            .await
            .unwrap();

        let searcher = search_task.await.unwrap().unwrap();

        // Task should be active since we have not sent SearchDone
        assert!(searcher.is_active());

        // Send SearchDone to complete the background task
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::SearchDone { id },
            ))
            .await
            .unwrap();

        // Give background task time to finish
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert!(!searcher.is_active());
    }

    #[test(tokio::test)]
    async fn searcher_should_fail_when_no_started_confirmation_received() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        let search_task =
            tokio::spawn(
                async move { Searcher::search(session.clone_channel(), test_query).await },
            );

        // Wait until we get the request
        let _req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();

        // Drop the transport so the mailbox closes without sending SearchStarted
        drop(transport);

        let err = search_task.await.unwrap().unwrap_err();
        assert!(err
            .to_string()
            .contains("Search query missing started confirmation"));
    }

    #[test(tokio::test)]
    async fn searcher_should_fail_when_error_response_received_during_setup() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        let search_task =
            tokio::spawn(
                async move { Searcher::search(session.clone_channel(), test_query).await },
            );

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();

        // Send an error response instead of SearchStarted
        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::Other,
                    description: String::from("search failed"),
                }),
            ))
            .await
            .unwrap();

        let err = search_task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
    }

    #[test(tokio::test)]
    async fn searcher_should_fail_when_unexpected_response_received_during_setup() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        let search_task =
            tokio::spawn(
                async move { Searcher::search(session.clone_channel(), test_query).await },
            );

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();

        // Send an unexpected response
        transport
            .write_frame_for(&Response::new(req.id, protocol::Response::Ok))
            .await
            .unwrap();

        let err = search_task.await.unwrap().unwrap_err();
        assert!(err.to_string().contains("Unexpected response"));
    }

    #[test(tokio::test)]
    async fn searcher_should_queue_results_received_before_started_confirmation() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        let search_task =
            tokio::spawn(
                async move { Searcher::search(session.clone_channel(), test_query).await },
            );

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        let id: SearchId = rand::random();

        // Send results BEFORE SearchStarted (this can happen in batched responses)
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                vec![
                    protocol::Response::SearchResults {
                        id,
                        matches: vec![SearchQueryMatch::Path(SearchQueryPathMatch {
                            path: PathBuf::from("/queued/path"),
                            submatches: vec![SearchQuerySubmatch {
                                r#match: SearchQueryMatchData::Text("queued".to_string()),
                                start: 0,
                                end: 6,
                            }],
                        })],
                    },
                    protocol::Response::SearchStarted { id },
                ],
            ))
            .await
            .unwrap();

        let mut searcher = search_task.await.unwrap().unwrap();

        // The queued match should be delivered
        let m = searcher.next().await.expect("Searcher closed unexpectedly");
        assert_eq!(
            m,
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from("/queued/path"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text("queued".to_string()),
                    start: 0,
                    end: 6,
                }],
            })
        );
    }

    #[test(tokio::test)]
    async fn searcher_should_return_none_after_search_done() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            paths: vec![PathBuf::from("/some/test/path")],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::Regex {
                value: String::from("."),
            },
            options: SearchQueryOptions::default(),
        };

        let search_task =
            tokio::spawn(
                async move { Searcher::search(session.clone_channel(), test_query).await },
            );

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        let id: SearchId = rand::random();

        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                protocol::Response::SearchStarted { id },
            ))
            .await
            .unwrap();

        let mut searcher = search_task.await.unwrap().unwrap();

        // Send one match followed by SearchDone
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                protocol::Response::SearchResults {
                    id,
                    matches: vec![SearchQueryMatch::Path(SearchQueryPathMatch {
                        path: PathBuf::from("/done/path"),
                        submatches: vec![SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("done".to_string()),
                            start: 0,
                            end: 4,
                        }],
                    })],
                },
            ))
            .await
            .unwrap();

        transport
            .write_frame_for(&Response::new(
                req.id,
                protocol::Response::SearchDone { id },
            ))
            .await
            .unwrap();

        // Get the match
        let m = searcher.next().await.expect("Searcher closed unexpectedly");
        assert_eq!(
            m,
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from("/done/path"),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text("done".to_string()),
                    start: 0,
                    end: 4,
                }],
            })
        );

        // After SearchDone and the task finishes, next() should return None
        assert_eq!(searcher.next().await, None);
    }
}
