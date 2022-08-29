use crate::data::{
    DistantResponseData, SearchId, SearchQuery, SearchQueryCondition, SearchQueryContentsMatch,
    SearchQueryMatch, SearchQueryMatchData, SearchQueryOptions, SearchQueryPathMatch,
    SearchQuerySubmatch, SearchQueryTarget,
};
use distant_net::Reply;
use grep::{
    matcher::Matcher,
    regex::RegexMatcher,
    searcher::{Searcher, Sink, SinkMatch},
};
use log::*;
use std::{collections::HashMap, io, ops::Deref, path::Path};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use walkdir::WalkDir;

/// Holds information related to active searches on the server
pub struct SearchState {
    channel: SearchChannel,
    task: JoinHandle<()>,
}

impl Drop for SearchState {
    /// Aborts the task that handles search operations and management
    fn drop(&mut self) {
        self.abort();
    }
}

impl SearchState {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(1);
        let task = tokio::spawn(search_task(tx.clone(), rx));

        Self {
            channel: SearchChannel { tx },
            task,
        }
    }

    #[allow(dead_code)]
    pub fn clone_channel(&self) -> SearchChannel {
        self.channel.clone()
    }

    /// Aborts the process task
    pub fn abort(&self) {
        self.task.abort();
    }
}

impl Deref for SearchState {
    type Target = SearchChannel;

    fn deref(&self) -> &Self::Target {
        &self.channel
    }
}

#[derive(Clone)]
pub struct SearchChannel {
    tx: mpsc::Sender<InnerSearchMsg>,
}

impl Default for SearchChannel {
    /// Creates a new channel that is closed by default
    fn default() -> Self {
        let (tx, _) = mpsc::channel(1);
        Self { tx }
    }
}

impl SearchChannel {
    /// Starts a new search using the provided query
    pub async fn start(
        &self,
        query: SearchQuery,
        reply: Box<dyn Reply<Data = DistantResponseData>>,
    ) -> io::Result<SearchId> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerSearchMsg::Start { query, reply, cb })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Internal search task closed"))?;
        rx.await
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Response to start dropped"))?
    }

    /// Cancels an active search
    pub async fn cancel(&self, id: SearchId) -> io::Result<()> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerSearchMsg::Cancel { id, cb })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Internal search task closed"))?;
        rx.await
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Response to cancel dropped"))?
    }
}

/// Internal message to pass to our task below to perform some action
enum InnerSearchMsg {
    Start {
        query: SearchQuery,
        reply: Box<dyn Reply<Data = DistantResponseData>>,
        cb: oneshot::Sender<io::Result<SearchId>>,
    },
    Cancel {
        id: SearchId,
        cb: oneshot::Sender<io::Result<()>>,
    },
    InternalRemove {
        id: SearchId,
    },
}

async fn search_task(tx: mpsc::Sender<InnerSearchMsg>, mut rx: mpsc::Receiver<InnerSearchMsg>) {
    let mut searches: HashMap<SearchId, oneshot::Sender<()>> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        match msg {
            InnerSearchMsg::Start { query, reply, cb } => {
                let id = rand::random::<SearchId>();

                // Attach a callback for when the process is finished where
                // we will remove it from our above list
                let tx = tx.clone();

                // Create a cancel channel to support interrupting and stopping the search
                let (cancel_tx, mut cancel_rx) = oneshot::channel();

                // Queue up our search internally and report back the id
                searches.insert(id, cancel_tx);
                let _ = cb.send(Ok(id));

                let SearchQuery {
                    path,
                    target,
                    condition,
                    options,
                } = query;

                // Create a blocking task that will search through all files within the
                // query path and look for matches
                tokio::task::spawn_blocking(move || {
                    let SearchQueryOptions {
                        limit,
                        pagination,
                        allowed_file_types,
                        follow_symbolic_links,
                    } = options;

                    // Define our walking setup
                    let walk_dir = WalkDir::new(path).follow_links(follow_symbolic_links);

                    // Define our cache of matches
                    let mut matches = Vec::new();

                    // Pushes a match, clearing and sending matches if we reach pagination,
                    // and returning true if should continue or false if limit reached
                    let mut push_match = |m: SearchQueryMatch| -> io::Result<bool> {
                        matches.push(m);

                        let should_continue = match limit.as_ref() {
                            Some(cnt) if *cnt == matches.len() as u64 => {
                                trace!("[Query {id}] Reached limit of {cnt} matches, so stopping search");
                                false
                            }
                            _ => true,
                        };

                        if let Some(len) = pagination {
                            if matches.len() as u64 >= len {
                                trace!(
                                    "[Query {id}] Reached pagination capacity of {len} matches, so forwarding search results to client"
                                );
                                let _ = reply.blocking_send(DistantResponseData::SearchResults {
                                    id,
                                    matches: std::mem::take(&mut matches),
                                });
                            }
                        }

                        Ok(should_continue)
                    };

                    // Define our search pattern
                    let pattern = match condition {
                        SearchQueryCondition::EndsWith { value } => format!(r"{value}$"),
                        SearchQueryCondition::Equals { value } => format!(r"^{value}$"),
                        SearchQueryCondition::Regex { value } => value,
                        SearchQueryCondition::StartsWith { value } => format!(r"^{value}"),
                    };

                    // Define our matcher using regex as the condition and execute the search
                    match RegexMatcher::new(&pattern) {
                        Ok(matcher) => {
                            // Search all entries for matches and report them
                            for entry in walk_dir.into_iter().filter_map(|e| e.ok()) {
                                // Check if we are being interrupted, and if so exit our loop early
                                match cancel_rx.try_recv() {
                                    Err(oneshot::error::TryRecvError::Empty) => (),
                                    _ => break,
                                }

                                // Skipy if provided explicit file types to search
                                if !allowed_file_types.is_empty()
                                    && !allowed_file_types.contains(&entry.file_type().into())
                                {
                                    continue;
                                }

                                let res = match target {
                                    // Perform the search against the path itself
                                    SearchQueryTarget::Path => {
                                        let path_str = entry.path().to_string_lossy();
                                        Searcher::new().search_slice(
                                            &matcher,
                                            path_str.as_bytes(),
                                            SearchQueryPathSink {
                                                search_id: id,
                                                path: entry.path(),
                                                matcher: &matcher,
                                                callback: &mut push_match,
                                            },
                                        )
                                    }

                                    // Skip if trying to search contents of non-file
                                    SearchQueryTarget::Contents if !entry.file_type().is_file() => {
                                        continue
                                    }

                                    // Perform the search against the file's contents
                                    SearchQueryTarget::Contents => Searcher::new().search_path(
                                        &matcher,
                                        entry.path(),
                                        SearchQueryContentsSink {
                                            search_id: id,
                                            path: entry.path(),
                                            matcher: &matcher,
                                            callback: &mut push_match,
                                        },
                                    ),
                                };

                                if let Err(x) = res {
                                    error!(
                                        "[Query {id}] Search failed for {:?}: {x}",
                                        entry.path()
                                    );
                                }
                            }
                        }
                        Err(x) => {
                            error!("[Query {id}] Failed to define regex matcher: {x}");
                        }
                    }

                    // Send any remaining matches
                    if !matches.is_empty() {
                        trace!("[Query {id}] Sending final {} matches", matches.len());
                        let _ =
                            reply.blocking_send(DistantResponseData::SearchResults { id, matches });
                    }

                    // Send back our search completion event
                    let _ = reply.blocking_send(DistantResponseData::SearchDone { id });

                    // Once complete, we need to send a request to remove the search from our list
                    let _ = tx.blocking_send(InnerSearchMsg::InternalRemove { id });
                });
            }
            InnerSearchMsg::Cancel { id, cb } => {
                let _ = cb.send(match searches.remove(&id) {
                    Some(tx) => {
                        let _ = tx.send(());
                        Ok(())
                    }
                    None => Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("[Query {id}] Cancellation failed because no search found"),
                    )),
                });
            }
            InnerSearchMsg::InternalRemove { id } => {
                searches.remove(&id);
            }
        }
    }
}

#[derive(Clone, Debug)]
struct SearchQueryPathSink<'a, M, F>
where
    M: Matcher,
    F: FnMut(SearchQueryMatch) -> Result<bool, io::Error>,
{
    search_id: SearchId,
    path: &'a Path,
    matcher: &'a M,
    callback: F,
}

impl<'a, M, F> Sink for SearchQueryPathSink<'a, M, F>
where
    M: Matcher,
    F: FnMut(SearchQueryMatch) -> Result<bool, io::Error>,
{
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let mut submatches = Vec::new();

        // Find all matches within the line
        let res = self.matcher.find_iter(mat.bytes(), |m| {
            let bytes = &mat.bytes()[m];
            submatches.push(SearchQuerySubmatch {
                r#match: match std::str::from_utf8(bytes) {
                    Ok(s) => SearchQueryMatchData::Text(s.to_string()),
                    Err(_) => SearchQueryMatchData::Bytes(bytes.to_vec()),
                },
                start: m.start() as u64,
                end: m.end() as u64,
            });

            true
        });

        if let Err(x) = res {
            error!(
                "[Query {}] SearchQueryPathSink encountered matcher error: {x}",
                self.search_id
            );
        }

        // If we have at least one submatch, then we have a match
        let should_continue = if !submatches.is_empty() {
            let r#match = SearchQueryMatch::Path(SearchQueryPathMatch {
                path: self.path.to_path_buf(),
                submatches,
            });

            (self.callback)(r#match)?
        } else {
            true
        };

        Ok(should_continue)
    }
}

#[derive(Clone, Debug)]
struct SearchQueryContentsSink<'a, M, F>
where
    M: Matcher,
    F: FnMut(SearchQueryMatch) -> Result<bool, io::Error>,
{
    search_id: SearchId,
    path: &'a Path,
    matcher: &'a M,
    callback: F,
}

impl<'a, M, F> Sink for SearchQueryContentsSink<'a, M, F>
where
    M: Matcher,
    F: FnMut(SearchQueryMatch) -> Result<bool, io::Error>,
{
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let mut submatches = Vec::new();

        // Find all matches within the line
        let res = self.matcher.find_iter(mat.bytes(), |m| {
            let bytes = &mat.bytes()[m];
            submatches.push(SearchQuerySubmatch {
                r#match: match std::str::from_utf8(bytes) {
                    Ok(s) => SearchQueryMatchData::Text(s.to_string()),
                    Err(_) => SearchQueryMatchData::Bytes(bytes.to_vec()),
                },
                start: m.start() as u64,
                end: m.end() as u64,
            });

            true
        });

        if let Err(x) = res {
            error!(
                "[Query {}] SearchQueryContentsSink encountered matcher error: {x}",
                self.search_id
            );
        }

        // If we have at least one submatch, then we have a match
        let should_continue = if !submatches.is_empty() {
            let r#match = SearchQueryMatch::Contents(SearchQueryContentsMatch {
                path: self.path.to_path_buf(),
                lines: match std::str::from_utf8(mat.bytes()) {
                    Ok(s) => SearchQueryMatchData::Text(s.to_string()),
                    Err(_) => SearchQueryMatchData::Bytes(mat.bytes().to_vec()),
                },
                line_number: mat.line_number().unwrap_or(0),
                absolute_offset: mat.absolute_byte_offset(),
                submatches,
            });

            (self.callback)(r#match)?
        } else {
            true
        };

        Ok(should_continue)
    }
}
