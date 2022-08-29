use crate::{
    client::{DistantChannel, DistantChannelExt},
    constants::CLIENT_SEARCHER_CAPACITY,
    data::{DistantRequestData, DistantResponseData, SearchId, SearchQuery, SearchQueryMatch},
    DistantMsg,
};
use distant_net::Request;
use log::*;
use std::{fmt, io};
use tokio::{sync::mpsc, task::JoinHandle};

/// Represents a searcher for files, directories, and symlinks on the filesystem
pub struct Searcher {
    channel: DistantChannel,
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
    pub async fn search(mut channel: DistantChannel, query: SearchQuery) -> io::Result<Self> {
        trace!("Searching using {query:?}",);

        // Submit our run request and get back a mailbox for responses
        let mut mailbox = channel
            .mail(Request::new(DistantMsg::Single(
                DistantRequestData::Search {
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
                    DistantResponseData::SearchResults { id, matches } => {
                        search_id = Some(id);
                        queue.extend(matches);
                    }
                    DistantResponseData::SearchStarted { id } => {
                        search_id = Some(id);
                    }
                    DistantResponseData::Error(x) => return Err(io::Error::from(x)),
                    x => {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!("Unexpected response: {:?}", x),
                        ))
                    }
                }
            }

            // Exit if we got the confirmation
            // NOTE: Doing this later because we want to make sure the entire payload is processed
            //       first before exiting the loop
            if search_id.is_some() {
                break;
            }
        }

        // Send out any of our queued changes that we got prior to the acknowledgement
        trace!(
            "[Query {}] Forwarding {} queued matches",
            queue.len(),
            search_id.unwrap_or(0),
        );
        for r#match in queue {
            if tx.send(r#match).await.is_err() {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Queue search match dropped",
                ));
            }
        }

        // If we never received an acknowledgement of search before the mailbox closed,
        // fail with a missing confirmation error
        if search_id.is_none() {
            return Err(io::Error::new(io::ErrorKind::Other, "Missing confirmation"));
        }

        let search_id = search_id.unwrap();

        // Spawn a task that continues to look for search result events and the conclusion of the
        // search, discarding anything else that it gets
        let task = tokio::spawn({
            async move {
                while let Some(res) = mailbox.next().await {
                    for data in res.payload.into_vec() {
                        match data {
                            DistantResponseData::SearchResults { matches, .. } => {
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
                            DistantResponseData::SearchDone { .. } => {
                                break;
                            }

                            _ => continue,
                        }
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
    use super::*;
    use crate::data::{
        SearchQueryCondition, SearchQueryMatchData, SearchQueryOptions, SearchQueryPathMatch,
        SearchQuerySubmatch, SearchQueryTarget,
    };
    use crate::DistantClient;
    use distant_net::{
        Client, FramedTransport, InmemoryTransport, IntoSplit, PlainCodec, Response,
        TypedAsyncRead, TypedAsyncWrite,
    };
    use std::{path::PathBuf, sync::Arc};
    use tokio::sync::Mutex;

    fn make_session() -> (
        FramedTransport<InmemoryTransport, PlainCodec>,
        DistantClient,
    ) {
        let (t1, t2) = FramedTransport::pair(100);
        let (writer, reader) = t2.into_split();
        (t1, Client::new(writer, reader).unwrap())
    }

    #[tokio::test]
    async fn searcher_should_have_query_reflect_ongoing_query() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            path: PathBuf::from("/some/test/path"),
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
        let req: Request<DistantRequestData> = transport.read().await.unwrap().unwrap();

        // Send back an acknowledgement that a search was started
        transport
            .write(Response::new(
                req.id,
                DistantResponseData::SearchStarted { id: rand::random() },
            ))
            .await
            .unwrap();

        // Get the searcher and verify the query
        let searcher = search_task.await.unwrap().unwrap();
        assert_eq!(searcher.query(), &test_query);
    }

    #[tokio::test]
    async fn searcher_should_support_getting_next_match() {
        let (mut transport, session) = make_session();
        let test_query = SearchQuery {
            path: PathBuf::from("/some/test/path"),
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
        let req: Request<DistantRequestData> = transport.read().await.unwrap().unwrap();

        // Send back an acknowledgement that a searcher was created
        let id = rand::random::<SearchId>();
        transport
            .write(Response::new(
                req.id.clone(),
                DistantResponseData::SearchStarted { id },
            ))
            .await
            .unwrap();

        // Get the searcher
        let mut searcher = search_task.await.unwrap().unwrap();

        // Send some matches related to the file
        transport
            .write(Response::new(
                req.id,
                vec![
                    DistantResponseData::SearchResults {
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
                    DistantResponseData::SearchResults {
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

    #[tokio::test]
    async fn searcher_should_distinguish_match_events_and_only_receive_matches_for_itself() {
        let (mut transport, session) = make_session();

        let test_query = SearchQuery {
            path: PathBuf::from("/some/test/path"),
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
        let req: Request<DistantRequestData> = transport.read().await.unwrap().unwrap();

        // Send back an acknowledgement that a searcher was created
        let id = rand::random();
        transport
            .write(Response::new(
                req.id.clone(),
                DistantResponseData::SearchStarted { id },
            ))
            .await
            .unwrap();

        // Get the searcher
        let mut searcher = search_task.await.unwrap().unwrap();

        // Send a match from the appropriate origin
        transport
            .write(Response::new(
                req.id.clone(),
                DistantResponseData::SearchResults {
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
            .write(Response::new(
                req.id.clone() + "1",
                DistantResponseData::SearchResults {
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
            .write(Response::new(
                req.id,
                DistantResponseData::SearchResults {
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

    #[tokio::test]
    async fn searcher_should_stop_receiving_events_if_cancelled() {
        let (mut transport, session) = make_session();

        let test_query = SearchQuery {
            path: PathBuf::from("/some/test/path"),
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
        let req: Request<DistantRequestData> = transport.read().await.unwrap().unwrap();

        // Send back an acknowledgement that a watcher was created
        let id = rand::random::<SearchId>();
        transport
            .write(Response::new(
                req.id.clone(),
                DistantResponseData::SearchStarted { id },
            ))
            .await
            .unwrap();

        // Send some matches from the appropriate origin
        transport
            .write(Response::new(
                req.id,
                DistantResponseData::SearchResults {
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

        let req: Request<DistantRequestData> = transport.read().await.unwrap().unwrap();

        transport
            .write(Response::new(req.id.clone(), DistantResponseData::Ok))
            .await
            .unwrap();

        // Wait for the cancel to complete
        cancel_task.await.unwrap().unwrap();

        // Send a match that will get ignored
        transport
            .write(Response::new(
                req.id,
                DistantResponseData::SearchResults {
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
}
