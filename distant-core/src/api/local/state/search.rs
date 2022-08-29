use crate::data::{
    DistantResponseData, SearchId, SearchQuery, SearchQueryContentsMatch, SearchQueryMatch,
    SearchQueryMatchData, SearchQueryOptions, SearchQueryPathMatch, SearchQuerySubmatch,
    SearchQueryTarget,
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
use walkdir::{DirEntry, WalkDir};

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
                let options = query.options.clone();

                // Build our executor and send an error if it fails
                let mut executor = match SearchQueryExecutor::new(query) {
                    Ok(executor) => executor,
                    Err(x) => {
                        let _ = cb.send(Err(x));
                        return;
                    }
                };

                // Get the unique search id
                let id = executor.id();

                // Queue up our search internally with a cancel sender
                searches.insert(id, executor.take_cancel_tx().unwrap());

                // Report back the search id
                let _ = cb.send(Ok(id));

                // Spawn our reporter of matches coming from the executor
                SearchQueryReporter {
                    id,
                    options,
                    rx: executor.take_match_rx().unwrap(),
                    reply,
                }
                .spawn();

                // Spawn our executor to run
                executor.spawn(tx.clone());
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
                trace!("[Query {id}] Removing internal tracking");
                searches.remove(&id);
            }
        }
    }
}

struct SearchQueryReporter {
    id: SearchId,
    options: SearchQueryOptions,
    rx: mpsc::UnboundedReceiver<SearchQueryMatch>,
    reply: Box<dyn Reply<Data = DistantResponseData>>,
}

impl SearchQueryReporter {
    /// Runs the reporter to completion in an async task
    pub fn spawn(self) {
        tokio::spawn(self.run());
    }

    async fn run(self) {
        let Self {
            id,
            options,
            mut rx,
            reply,
        } = self;

        // Queue of matches that we hold until reaching pagination
        let mut matches = Vec::new();
        let mut total_matches_cnt = 0;

        trace!("[Query {id}] Starting reporter with {options:?}");
        while let Some(m) = rx.recv().await {
            matches.push(m);
            total_matches_cnt += 1;

            // Check if we've reached the limit, and quit if we have
            if let Some(len) = options.limit {
                if total_matches_cnt >= len {
                    trace!("[Query {id}] Reached limit of {len} matches");
                    break;
                }
            }

            // Check if we've reached pagination size, and send queued if so
            if let Some(len) = options.pagination {
                if matches.len() as u64 >= len {
                    trace!("[Query {id}] Reached {len} paginated matches");
                    if let Err(x) = reply
                        .send(DistantResponseData::SearchResults {
                            id,
                            matches: std::mem::take(&mut matches),
                        })
                        .await
                    {
                        error!("[Query {id}] Failed to send paginated matches: {x}");
                    }
                }
            }
        }

        // Send any remaining matches
        if !matches.is_empty() {
            trace!("[Query {id}] Sending {} remaining matches", matches.len());
            if let Err(x) = reply
                .send(DistantResponseData::SearchResults { id, matches })
                .await
            {
                error!("[Query {id}] Failed to send final matches: {x}");
            }
        }

        // Report that we are done
        trace!("[Query {id}] Reporting as done");
        if let Err(x) = reply.send(DistantResponseData::SearchDone { id }).await {
            error!("[Query {id}] Failed to send done status: {x}");
        }
    }
}

struct SearchQueryExecutor {
    id: SearchId,
    query: SearchQuery,
    walk_dir: WalkDir,
    matcher: RegexMatcher,

    cancel_tx: Option<oneshot::Sender<()>>,
    cancel_rx: oneshot::Receiver<()>,

    match_tx: mpsc::UnboundedSender<SearchQueryMatch>,
    match_rx: Option<mpsc::UnboundedReceiver<SearchQueryMatch>>,
}

impl SearchQueryExecutor {
    /// Creates a new executor
    pub fn new(query: SearchQuery) -> io::Result<Self> {
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let (match_tx, match_rx) = mpsc::unbounded_channel();

        let path = query.path.as_path();
        let follow_links = query.options.follow_symbolic_links;
        let regex = query.condition.clone().into_regex_string();

        let matcher = RegexMatcher::new(&regex)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?;
        let walk_dir = WalkDir::new(path).follow_links(follow_links);

        Ok(Self {
            id: rand::random(),
            query,
            matcher,
            walk_dir,

            cancel_tx: Some(cancel_tx),
            cancel_rx,

            match_tx,
            match_rx: Some(match_rx),
        })
    }

    pub fn id(&self) -> SearchId {
        self.id
    }

    pub fn take_cancel_tx(&mut self) -> Option<oneshot::Sender<()>> {
        self.cancel_tx.take()
    }

    pub fn take_match_rx(&mut self) -> Option<mpsc::UnboundedReceiver<SearchQueryMatch>> {
        self.match_rx.take()
    }

    /// Runs the executor to completion in another thread
    pub fn spawn(self, tx: mpsc::Sender<InnerSearchMsg>) {
        tokio::task::spawn_blocking(move || {
            let id = self.id;
            self.run();

            // Once complete, we need to send a request to remove the search from our list
            let _ = tx.blocking_send(InnerSearchMsg::InternalRemove { id });
        });
    }

    fn run(self) {
        let id = self.id;
        let walk_dir = self.walk_dir;
        let tx = self.match_tx;
        let mut cancel = self.cancel_rx;

        // Create our path filter we will use to filter entries
        let path_filter = match self.query.options.path_regex.as_deref() {
            Some(regex) => match SearchQueryPathFilter::new(regex) {
                Ok(filter) => {
                    trace!("[Query {id}] Using regex path filter for {regex:?}");
                    filter
                }
                Err(x) => {
                    error!("[Query {id}] Failed to instantiate path filter: {x}");
                    return;
                }
            },
            None => {
                trace!("[Query {id}] Using noop path filter");
                SearchQueryPathFilter::noop()
            }
        };

        let options_filter = SearchQueryOptionsFilter {
            target: self.query.target,
            options: self.query.options.clone(),
        };

        // Search all entries for matches and report them
        for entry in walk_dir
            .into_iter()
            .filter_entry(|e| path_filter.filter(e.path()))
            .filter_map(|e| e.ok())
            .filter(|e| options_filter.filter(e))
        {
            // Check if we are being interrupted, and if so exit our loop early
            match cancel.try_recv() {
                Err(oneshot::error::TryRecvError::Empty) => (),
                _ => {
                    debug!("[Query {id}] Cancelled");
                    break;
                }
            }

            let res = match self.query.target {
                // Perform the search against the path itself
                SearchQueryTarget::Path => {
                    let path_str = entry.path().to_string_lossy();
                    Searcher::new().search_slice(
                        &self.matcher,
                        path_str.as_bytes(),
                        SearchQueryPathSink {
                            search_id: id,
                            path: entry.path(),
                            matcher: &self.matcher,
                            callback: |m| Ok(tx.send(m).is_ok()),
                        },
                    )
                }

                // Perform the search against the file's contents
                SearchQueryTarget::Contents => Searcher::new().search_path(
                    &self.matcher,
                    entry.path(),
                    SearchQueryContentsSink {
                        search_id: id,
                        path: entry.path(),
                        matcher: &self.matcher,
                        callback: |m| Ok(tx.send(m).is_ok()),
                    },
                ),
            };

            if let Err(x) = res {
                error!("[Query {id}] Search failed for {:?}: {x}", entry.path());
            }
        }
    }
}

struct SearchQueryPathFilter {
    matcher: Option<RegexMatcher>,
}

impl SearchQueryPathFilter {
    pub fn new(regex: &str) -> io::Result<Self> {
        Ok(Self {
            matcher: Some(
                RegexMatcher::new(regex)
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?,
            ),
        })
    }

    /// Returns a filter that always passes the path
    pub fn noop() -> Self {
        Self { matcher: None }
    }

    /// Returns true if path passes the filter
    pub fn filter(&self, path: impl AsRef<Path>) -> bool {
        self.try_filter(path).unwrap_or(false)
    }

    fn try_filter(&self, path: impl AsRef<Path>) -> io::Result<bool> {
        match &self.matcher {
            Some(matcher) => matcher
                .is_match(path.as_ref().to_string_lossy().as_bytes())
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x)),
            None => Ok(true),
        }
    }
}

struct SearchQueryOptionsFilter {
    target: SearchQueryTarget,
    options: SearchQueryOptions,
}

impl SearchQueryOptionsFilter {
    pub fn filter(&self, entry: &DirEntry) -> bool {
        // Check if filetype is allowed
        let file_type_allowed = self.options.allowed_file_types.is_empty()
            || self
                .options
                .allowed_file_types
                .contains(&entry.file_type().into());

        // Check if target is appropriate
        let targeted = match self.target {
            SearchQueryTarget::Contents => entry.file_type().is_file(),
            _ => true,
        };

        file_type_allowed && targeted
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_send_event_when_starting_query() {
        todo!();
    }

    #[test]
    fn should_send_event_when_query_finished() {
        todo!();
    }

    #[test]
    fn should_send_all_matches_at_once_by_default() {
        todo!();
    }

    #[test]
    fn should_send_paginated_results_if_specified() {
        todo!();
    }

    #[test]
    fn should_send_maximum_of_limit_results_if_specified() {
        todo!();
    }

    #[test]
    fn should_limit_searched_paths_using_regex_filter_if_specified() {
        todo!();
    }
}
