use crate::data::{
    DistantResponseData, SearchId, SearchQuery, SearchQueryCondition, SearchQueryContentsMatch,
    SearchQueryMatch, SearchQueryMatchData, SearchQueryOption, SearchQueryPathMatch,
    SearchQuerySubmatch, SearchQueryTarget,
};
use distant_net::Reply;
use log::*;
use std::{
    collections::{HashMap, HashSet},
    io,
    ops::Deref,
};
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
                use grep::{
                    matcher::Matcher,
                    regex::RegexMatcher,
                    searcher::{
                        sinks::{Lossy, UTF8},
                        Searcher,
                    },
                };

                let id = rand::random::<SearchId>();

                // Attach a callback for when the process is finished where
                // we will remove it from our above list
                let tx = tx.clone();

                // Create a cancel channel to support interrupting and stopping the search
                let (cancel_tx, mut cancel_rx) = oneshot::channel();

                let SearchQuery {
                    path,
                    target,
                    condition,
                    options,
                } = query;

                // Create a blocking task that will search through all files within the
                // query path and look for matches
                tokio::task::spawn_blocking(move || {
                    let mut limit = None;
                    let mut pagination = None;
                    let mut allowed_file_types = HashSet::new();
                    let mut follow_symbolic_links = false;

                    // Read in our options
                    for opt in options {
                        match opt {
                            SearchQueryOption::FileType { kind } => {
                                allowed_file_types.insert(kind);
                            }
                            SearchQueryOption::FollowSymbolicLinks => follow_symbolic_links = true,
                            SearchQueryOption::Limit { limit: value } => limit = Some(value),
                            SearchQueryOption::Pagination { count } => pagination = Some(count),
                        }
                    }

                    // Define our walking setup
                    let walk_dir = WalkDir::new(path).follow_links(follow_symbolic_links);

                    // Define our cache of matches
                    let mut matches = Vec::new();

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

                                let res = match target {
                                    // Perform the search against the path itself
                                    SearchQueryTarget::Path => {
                                        let path_str = entry.path().to_string_lossy();
                                        Searcher::new().search_slice(
                                            &matcher,
                                            path_str.as_bytes(),
                                            UTF8(|lnum, line| {

                                            // TODO: Write a custom Sink like https://docs.rs/grep-searcher/0.1.10/src/grep_searcher/sink.rs.html#536-538
                                            //       Will put together a SearchQueryPathMatch
                                            //       and invoke a function with it, which we can
                                            //       use to populate our matches

                                                let mymatch =
                                                    matcher.find(line.as_bytes())?.unwrap();
                                                matches.push((lnum, line[mymatch].to_string()));
                                                Ok(true)
                                            }),
                                        )
                                    }

                                    // Perform the search against the file's contents
                                    SearchQueryTarget::Contents => Searcher::new().search_path(
                                        &matcher,
                                        entry.path(),
                                        Lossy(|lnum, line| {
                                            let mut submatches = Vec::new();

                                            // TODO: Write a custom Sink like https://docs.rs/grep-searcher/0.1.10/src/grep_searcher/sink.rs.html#536-538
                                            //       Will put together a SearchQueryContentsMatch
                                            //       and invoke a function with it, which we can
                                            //       use to populate our matches

                                            // Find all matches within the line
                                            matcher.find_iter(line.as_bytes(), |m| {
                                                submatches.push(SearchQuerySubmatch {
                                                    r#match: SearchQueryMatchData::Text(
                                                        line[m].to_string(),
                                                    ),
                                                    start: m.start() as u64,
                                                    end: m.end() as u64,
                                                });

                                                true
                                            });

                                            // If we have at least one submatch, then we have a
                                            // match
                                            if !submatches.is_empty() {
                                                matches.push(SearchQueryMatch::Contents(
                                                    SearchQueryContentsMatch {
                                                        path: entry.path().to_path_buf(),
                                                        lines: SearchQueryMatchData::Text(
                                                            line.to_string(),
                                                        ),
                                                        line_number: lnum,
                                                        absolute_offset: todo!(),
                                                        submatches,
                                                    },
                                                ));
                                            }

                                            Ok(true)
                                        }),
                                    ),
                                };

                                if let Err(x) = res {
                                    error!("Search failed: {x}");
                                }
                            }
                        }
                        Err(x) => {
                            error!("Failed to define regex matcher: {x}");
                        }
                    }

                    // Send any remaining matches
                    if !matches.is_empty() {
                        let _ =
                            reply.blocking_send(DistantResponseData::SearchResults { id, matches });
                    }

                    // Send back our search completion event
                    let _ = reply.blocking_send(DistantResponseData::SearchDone { id });

                    // Once complete, we need to send a request to remove the search from our list
                    let _ = tx.blocking_send(InnerSearchMsg::InternalRemove { id });
                });

                searches.insert(id, cancel_tx);
                let _ = cb.send(Ok(id));
            }
            InnerSearchMsg::Cancel { id, cb } => {
                let _ = cb.send(match searches.remove(&id) {
                    Some(tx) => {
                        let _ = tx.send(());
                        Ok(())
                    }
                    None => Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("No search found with id {id}"),
                    )),
                });
            }
            InnerSearchMsg::InternalRemove { id } => {
                searches.remove(&id);
            }
        }
    }
}
