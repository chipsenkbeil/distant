use std::collections::HashMap;
use std::ops::Deref;
use std::path::Path;
use std::{cmp, io};

use distant_net::server::Reply;
use grep::matcher::Matcher;
use grep::regex::{RegexMatcher, RegexMatcherBuilder};
use grep::searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::types::TypesBuilder;
use ignore::{DirEntry, ParallelVisitor, ParallelVisitorBuilder, WalkBuilder, WalkParallel};
use log::*;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::protocol::{
    Response, SearchId, SearchQuery, SearchQueryContentsMatch, SearchQueryMatch,
    SearchQueryMatchData, SearchQueryOptions, SearchQueryPathMatch, SearchQuerySubmatch,
    SearchQueryTarget,
};

const MAXIMUM_SEARCH_THREADS: usize = 12;

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
        reply: Box<dyn Reply<Data = Response>>,
    ) -> io::Result<SearchId> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerSearchMsg::Start {
                query: Box::new(query),
                reply,
                cb,
            })
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
        query: Box<SearchQuery>,
        reply: Box<dyn Reply<Data = Response>>,
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
    let mut searches: HashMap<SearchId, broadcast::Sender<()>> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        match msg {
            InnerSearchMsg::Start { query, reply, cb } => {
                let options = query.options.clone();

                // Build our executor and send an error if it fails
                let mut executor = match SearchQueryExecutor::new(*query) {
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
    reply: Box<dyn Reply<Data = Response>>,
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
                        .send(Response::SearchResults {
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
            if let Err(x) = reply.send(Response::SearchResults { id, matches }).await {
                error!("[Query {id}] Failed to send final matches: {x}");
            }
        }

        // Report that we are done
        trace!("[Query {id}] Reporting as done");
        if let Err(x) = reply.send(Response::SearchDone { id }).await {
            error!("[Query {id}] Failed to send done status: {x}");
        }
    }
}

struct SearchQueryExecutor {
    id: SearchId,
    query: SearchQuery,
    walker: WalkParallel,
    matcher: RegexMatcher,

    cancel_tx: Option<broadcast::Sender<()>>,
    cancel_rx: broadcast::Receiver<()>,

    match_tx: mpsc::UnboundedSender<SearchQueryMatch>,
    match_rx: Option<mpsc::UnboundedReceiver<SearchQueryMatch>>,
}

impl SearchQueryExecutor {
    /// Creates a new executor
    pub fn new(query: SearchQuery) -> io::Result<Self> {
        let (cancel_tx, cancel_rx) = broadcast::channel(1);
        let (match_tx, match_rx) = mpsc::unbounded_channel();

        let regex = query.condition.to_regex_string();
        let mut matcher_builder = RegexMatcherBuilder::new();
        matcher_builder
            .case_insensitive(false)
            .case_smart(false)
            .multi_line(true)
            .dot_matches_new_line(false)
            .swap_greed(false)
            .ignore_whitespace(false)
            .unicode(true)
            .octal(false)
            .line_terminator(Some(b'\n'));
        let matcher = matcher_builder
            .build(&regex)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?;

        if query.paths.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "missing paths"));
        }

        // Build our list of paths so we can ensure we weed out duplicates
        let mut target_paths = Vec::new();
        for mut path in query.paths.iter().map(Deref::deref) {
            // For each explicit path, we will add it directly UNLESS we
            // are searching upward and have a max depth > 0 to avoid
            // searching this path twice
            if !query.options.upward || query.options.max_depth == Some(0) {
                target_paths.push(path);
            }

            // For going in the upward direction, we will add ancestor paths as long
            // as the max depth allows it
            if query.options.upward {
                let mut remaining = query.options.max_depth;
                if query.options.max_depth.is_none() || query.options.max_depth > Some(0) {
                    while let Some(parent) = path.parent() {
                        // If we have a maximum depth and it has reached zero, we
                        // don't want to include any more paths
                        if remaining == Some(0) {
                            break;
                        }

                        path = parent;
                        target_paths.push(path);

                        if let Some(x) = remaining.as_mut() {
                            *x -= 1;
                        }
                    }
                }
            }
        }

        target_paths.sort_unstable();
        target_paths.dedup();

        // Construct the walker with our paths
        let mut walker_builder = WalkBuilder::new(target_paths[0]);
        for path in &target_paths[1..] {
            walker_builder.add(path);
        }

        // Apply common configuration options to our walker
        walker_builder
            .follow_links(query.options.follow_symbolic_links)
            .threads(cmp::min(MAXIMUM_SEARCH_THREADS, num_cpus::get()))
            .types(
                TypesBuilder::new()
                    .add_defaults()
                    .build()
                    .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?,
            )
            .skip_stdout(true);

        if query.options.upward {
            // If traversing upward, we need to use max depth to determine how many
            // path segments to support, break those up, and add them. The max
            // depth setting itself should be 1 to avoid searching anything but
            // the immediate children of each path component
            walker_builder.max_depth(Some(1));
        } else {
            // Otherwise, we apply max depth like expected
            walker_builder.max_depth(
                query
                    .options
                    .max_depth
                    .as_ref()
                    .copied()
                    .map(|d| d as usize),
            );
        }

        Ok(Self {
            id: rand::random(),
            query,
            matcher,
            walker: walker_builder.build_parallel(),
            cancel_tx: Some(cancel_tx),
            cancel_rx,

            match_tx,
            match_rx: Some(match_rx),
        })
    }

    pub fn id(&self) -> SearchId {
        self.id
    }

    pub fn take_cancel_tx(&mut self) -> Option<broadcast::Sender<()>> {
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
        let walker = self.walker;
        let tx = self.match_tx;
        let cancel = self.cancel_rx;
        let matcher = self.matcher;

        // Create our path filter we will use to filter out entries that do not match filter
        let include_path_filter = match self.query.options.include.as_ref() {
            Some(condition) => match SearchQueryPathFilter::new(&condition.to_regex_string()) {
                Ok(filter) => {
                    trace!("[Query {id}] Using regex include path filter for {condition:?}");
                    filter
                }
                Err(x) => {
                    error!("[Query {id}] Failed to instantiate include path filter: {x}");
                    return;
                }
            },
            None => {
                trace!("[Query {id}] Using fixed include path filter of true");
                SearchQueryPathFilter::fixed(true)
            }
        };

        // Create our path filter we will use to filter out entries that match filter
        let exclude_path_filter = match self.query.options.exclude.as_ref() {
            Some(condition) => match SearchQueryPathFilter::new(&condition.to_regex_string()) {
                Ok(filter) => {
                    trace!("[Query {id}] Using regex exclude path filter for {condition:?}");
                    filter
                }
                Err(x) => {
                    error!("[Query {id}] Failed to instantiate exclude path filter: {x}");
                    return;
                }
            },
            None => {
                trace!("[Query {id}] Using fixed exclude path filter of false");
                SearchQueryPathFilter::fixed(false)
            }
        };

        let options_filter = SearchQueryOptionsFilter {
            target: self.query.target,
            options: self.query.options.clone(),
        };

        let mut builder = SearchQueryExecutorParallelVistorBuilder {
            search_id: self.id,
            target: self.query.target,
            cancel,
            tx,
            matcher: &matcher,
            include_path_filter: &include_path_filter,
            exclude_path_filter: &exclude_path_filter,
            options_filter: &options_filter,
        };

        // Search all entries for matches and report them
        //
        // NOTE: This should block waiting for all threads to complete
        walker.visit(&mut builder);
    }
}

struct SearchQueryExecutorParallelVistorBuilder<'a> {
    search_id: SearchId,
    target: SearchQueryTarget,
    cancel: broadcast::Receiver<()>,
    tx: mpsc::UnboundedSender<SearchQueryMatch>,
    matcher: &'a RegexMatcher,
    include_path_filter: &'a SearchQueryPathFilter,
    exclude_path_filter: &'a SearchQueryPathFilter,
    options_filter: &'a SearchQueryOptionsFilter,
}

impl<'a> ParallelVisitorBuilder<'a> for SearchQueryExecutorParallelVistorBuilder<'a> {
    fn build(&mut self) -> Box<dyn ParallelVisitor + 'a> {
        // For files that are searched as part of a recursive search
        //
        // Details:
        //     * Will quit early if detecting binary file due to null byte
        //
        // NOTE: Searchers are not Send/Sync so we must create them here
        let implicit_searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(0))
            .build();

        // For files that are searched because they are provided as one of our initial paths
        // (so explicitly by the user)
        //
        // Details:
        //     * Will convert binary data with null bytes into newlines
        //
        // NOTE: Searchers are not Send/Sync so we must create them here
        let explicit_searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::convert(0))
            .build();

        Box::new(SearchQueryExecutorParallelVistor {
            search_id: self.search_id,
            target: self.target,
            cancel: self.cancel.resubscribe(),
            tx: self.tx.clone(),
            matcher: self.matcher,
            implicit_searcher,
            explicit_searcher,
            include_path_filter: self.include_path_filter,
            exclude_path_filter: self.exclude_path_filter,
            options_filter: self.options_filter,
        })
    }
}

struct SearchQueryExecutorParallelVistor<'a> {
    search_id: SearchId,
    target: SearchQueryTarget,
    cancel: broadcast::Receiver<()>,
    tx: mpsc::UnboundedSender<SearchQueryMatch>,
    matcher: &'a RegexMatcher,
    implicit_searcher: Searcher,
    explicit_searcher: Searcher,
    include_path_filter: &'a SearchQueryPathFilter,
    exclude_path_filter: &'a SearchQueryPathFilter,
    options_filter: &'a SearchQueryOptionsFilter,
}

impl<'a> ParallelVisitor for SearchQueryExecutorParallelVistor<'a> {
    fn visit(&mut self, entry: Result<DirEntry, ignore::Error>) -> ignore::WalkState {
        use ignore::WalkState;
        let id = self.search_id;

        // Get the entry, skipping errors with directories, and continuing on
        // errors with non-directories
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => return WalkState::Skip,
        };

        // Validate the path of the entry should be processed
        //
        // NOTE: We do not SKIP here as we cannot cancel a directory traversal early as this can
        //       cause us to miss relevant submatches deeper in the traversal
        if !self.include_path_filter.filter(entry.path())
            || self.exclude_path_filter.filter(entry.path())
            || !self.options_filter.filter(&entry)
        {
            return WalkState::Continue;
        }

        // Check if we are being interrupted, and if so exit our loop early
        match self.cancel.try_recv() {
            Err(broadcast::error::TryRecvError::Empty) => (),
            _ => {
                debug!("[Query {id}] Cancelled");
                return WalkState::Quit;
            }
        }

        // Pick searcher based on whether this was an explicit or recursive path
        let searcher = if entry.depth() == 0 {
            &mut self.explicit_searcher
        } else {
            &mut self.implicit_searcher
        };

        let res = match self.target {
            // Perform the search against the path itself
            SearchQueryTarget::Path => {
                let path_str = entry.path().to_string_lossy();
                searcher.search_slice(
                    self.matcher,
                    path_str.as_bytes(),
                    SearchQueryPathSink {
                        search_id: id,
                        path: entry.path(),
                        matcher: self.matcher,
                        callback: |m| Ok(self.tx.send(m).is_ok()),
                    },
                )
            }

            // Perform the search against the file's contents
            SearchQueryTarget::Contents => searcher.search_path(
                self.matcher,
                entry.path(),
                SearchQueryContentsSink {
                    search_id: id,
                    path: entry.path(),
                    matcher: self.matcher,
                    callback: |m| Ok(self.tx.send(m).is_ok()),
                },
            ),
        };

        if let Err(x) = res {
            error!("[Query {id}] Search failed for {:?}: {x}", entry.path());
        }

        WalkState::Continue
    }
}

struct SearchQueryPathFilter {
    matcher: Option<RegexMatcher>,
    default_value: bool,
}

impl SearchQueryPathFilter {
    pub fn new(regex: &str) -> io::Result<Self> {
        Ok(Self {
            matcher: Some(
                RegexMatcher::new(regex)
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?,
            ),
            default_value: false,
        })
    }

    /// Returns a filter that always returns `value`
    pub fn fixed(value: bool) -> Self {
        Self {
            matcher: None,
            default_value: value,
        }
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
            None => Ok(self.default_value),
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
            || entry
                .file_type()
                .map(|ft| self.options.allowed_file_types.contains(&ft.into()))
                .unwrap_or_default();

        // Check if target is appropriate
        let targeted = match self.target {
            SearchQueryTarget::Contents => {
                entry.file_type().map(|ft| ft.is_file()).unwrap_or_default()
            }
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

                // NOTE: Since we are defining the searcher, we control always including the line
                //       number, so we can safely unwrap here
                line_number: mat.line_number().unwrap(),

                // NOTE: absolute_byte_offset from grep tells us where the bytes start for the
                //       match, but not inclusive of where within the match
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
    use std::path::PathBuf;

    use assert_fs::prelude::*;
    use test_log::test;

    use super::*;
    use crate::protocol::{FileType, SearchQueryCondition, SearchQueryMatchData};

    fn make_path(path: &str) -> PathBuf {
        use std::path::MAIN_SEPARATOR;

        // Ensure that our path is compliant with the current platform
        let path = path.replace('/', &MAIN_SEPARATOR.to_string());

        PathBuf::from(path)
    }

    fn setup_dir(files: Vec<(&str, &str)>) -> assert_fs::TempDir {
        let root = assert_fs::TempDir::new().unwrap();

        for (path, contents) in files {
            root.child(make_path(path)).write_str(contents).unwrap();
        }

        root
    }

    fn get_matches(data: Response) -> Vec<SearchQueryMatch> {
        match data {
            Response::SearchResults { matches, .. } => matches,
            x => panic!("Did not get search results: {x:?}"),
        }
    }

    #[test(tokio::test)]
    async fn should_send_event_when_query_finished() {
        let root = setup_dir(Vec::new());

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::equals(""),
            options: Default::default(),
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_send_all_matches_at_once_by_default() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", ""),
            ("path/to/file2.txt", ""),
            ("other/file.txt", ""),
            ("dir/other/bin", ""),
        ]);

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::regex("other"),
            options: Default::default(),
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let mut matches = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_path_match())
            .collect::<Vec<_>>();

        matches.sort_unstable_by_key(|m| m.path.to_path_buf());

        // Root path len (including trailing separator) + 1 to be at start of child path
        let child_start = (root.path().to_string_lossy().len() + 1) as u64;

        assert_eq!(
            matches,
            vec![
                SearchQueryPathMatch {
                    path: root.child(make_path("dir/other")).to_path_buf(),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("other".to_string()),
                        start: child_start + 4,
                        end: child_start + 9,
                    }]
                },
                SearchQueryPathMatch {
                    path: root.child(make_path("dir/other/bin")).to_path_buf(),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("other".to_string()),
                        start: child_start + 4,
                        end: child_start + 9,
                    }]
                },
                SearchQueryPathMatch {
                    path: root.child(make_path("other")).to_path_buf(),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("other".to_string()),
                        start: child_start,
                        end: child_start + 5,
                    }]
                },
                SearchQueryPathMatch {
                    path: root.child(make_path("other/file.txt")).to_path_buf(),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("other".to_string()),
                        start: child_start,
                        end: child_start + 5,
                    }]
                },
            ]
        );

        assert_eq!(
            rx.recv().await,
            Some(Response::SearchDone { id: search_id })
        );

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_support_targeting_paths() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", ""),
            ("path/to/file2.txt", ""),
            ("other/file.txt", ""),
            ("other/dir/bin", ""),
        ]);

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::regex("path"),
            options: Default::default(),
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let mut matches = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_path_match())
            .collect::<Vec<_>>();

        matches.sort_unstable_by_key(|m| m.path.to_path_buf());

        // Root path len (including trailing separator) + 1 to be at start of child path
        let child_start = (root.path().to_string_lossy().len() + 1) as u64;

        assert_eq!(
            matches,
            vec![
                SearchQueryPathMatch {
                    path: root.child(make_path("path")).to_path_buf(),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("path".to_string()),
                        start: child_start,
                        end: child_start + 4,
                    }]
                },
                SearchQueryPathMatch {
                    path: root.child(make_path("path/to")).to_path_buf(),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("path".to_string()),
                        start: child_start,
                        end: child_start + 4,
                    }]
                },
                SearchQueryPathMatch {
                    path: root.child(make_path("path/to/file1.txt")).to_path_buf(),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("path".to_string()),
                        start: child_start,
                        end: child_start + 4,
                    }]
                },
                SearchQueryPathMatch {
                    path: root.child(make_path("path/to/file2.txt")).to_path_buf(),
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("path".to_string()),
                        start: child_start,
                        end: child_start + 4,
                    }]
                }
            ]
        );

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_support_targeting_contents() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", "some\nlines of text in\na\nfile"),
            ("path/to/file2.txt", "more text"),
            ("other/file.txt", "some other file with text"),
            ("other/dir/bin", "asdfasdfasdfasdfasdfasdfasdfasdfasdf"),
        ]);

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Contents,
            condition: SearchQueryCondition::regex("text"),
            options: Default::default(),
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let mut matches = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_contents_match())
            .collect::<Vec<_>>();

        matches.sort_unstable_by_key(|m| m.path.to_path_buf());

        assert_eq!(
            matches,
            vec![
                SearchQueryContentsMatch {
                    path: root.child(make_path("other/file.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("some other file with text"),
                    line_number: 1,
                    absolute_offset: 0,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("text".to_string()),
                        start: 21,
                        end: 25,
                    }]
                },
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file1.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("lines of text in\n"),
                    line_number: 2,
                    absolute_offset: 5,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("text".to_string()),
                        start: 9,
                        end: 13,
                    }]
                },
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file2.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("more text"),
                    line_number: 1,
                    absolute_offset: 0,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("text".to_string()),
                        start: 5,
                        end: 9,
                    }]
                }
            ]
        );

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_support_multiple_submatches() {
        let root = setup_dir(vec![("path/to/file.txt", "aa ab ac\nba bb bc\nca cb cc")]);

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Contents,
            condition: SearchQueryCondition::regex(r"[abc][ab]"),
            options: Default::default(),
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let mut matches = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_contents_match())
            .collect::<Vec<_>>();

        matches.sort_unstable_by_key(|m| m.line_number);

        assert_eq!(
            matches,
            vec![
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("aa ab ac\n"),
                    line_number: 1,
                    absolute_offset: 0,
                    submatches: vec![
                        SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("aa".to_string()),
                            start: 0,
                            end: 2,
                        },
                        SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("ab".to_string()),
                            start: 3,
                            end: 5,
                        }
                    ]
                },
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("ba bb bc\n"),
                    line_number: 2,
                    absolute_offset: 9,
                    submatches: vec![
                        SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("ba".to_string()),
                            start: 0,
                            end: 2,
                        },
                        SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("bb".to_string()),
                            start: 3,
                            end: 5,
                        }
                    ]
                },
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("ca cb cc"),
                    line_number: 3,
                    absolute_offset: 18,
                    submatches: vec![
                        SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("ca".to_string()),
                            start: 0,
                            end: 2,
                        },
                        SearchQuerySubmatch {
                            r#match: SearchQueryMatchData::Text("cb".to_string()),
                            start: 3,
                            end: 5,
                        }
                    ]
                },
            ]
        );

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_send_paginated_results_if_specified() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", "some\nlines of text in\na\nfile"),
            ("path/to/file2.txt", "more text"),
            ("other/file.txt", "some other file with text"),
            ("other/dir/bin", "asdfasdfasdfasdfasdfasdfasdfasdfasdf"),
        ]);

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Contents,
            condition: SearchQueryCondition::regex("text"),
            options: SearchQueryOptions {
                pagination: Some(2),
                ..Default::default()
            },
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        // Collect all matches here
        let mut matches = Vec::new();

        // Get first two matches
        let paginated_matches = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_contents_match())
            .collect::<Vec<_>>();

        assert_eq!(paginated_matches.len(), 2);
        matches.extend(paginated_matches);

        // Get last match
        let paginated_matches = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_contents_match())
            .collect::<Vec<_>>();

        assert_eq!(paginated_matches.len(), 1);
        matches.extend(paginated_matches);

        // Sort our matches so we can check them all
        matches.sort_unstable_by_key(|m| m.path.to_path_buf());

        assert_eq!(
            matches,
            vec![
                SearchQueryContentsMatch {
                    path: root.child(make_path("other/file.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("some other file with text"),
                    line_number: 1,
                    absolute_offset: 0,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("text".to_string()),
                        start: 21,
                        end: 25,
                    }]
                },
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file1.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("lines of text in\n"),
                    line_number: 2,
                    absolute_offset: 5,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("text".to_string()),
                        start: 9,
                        end: 13,
                    }]
                },
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file2.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("more text"),
                    line_number: 1,
                    absolute_offset: 0,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("text".to_string()),
                        start: 5,
                        end: 9,
                    }]
                }
            ]
        );

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_send_maximum_of_limit_results_if_specified() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", "some\nlines of text in\na\nfile"),
            ("path/to/file2.txt", "more text"),
            ("other/file.txt", "some other file with text"),
            ("other/dir/bin", "asdfasdfasdfasdfasdfasdfasdfasdfasdf"),
        ]);

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Contents,
            condition: SearchQueryCondition::regex("text"),
            options: SearchQueryOptions {
                limit: Some(2),
                ..Default::default()
            },
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        // Get all matches and verify the len
        let matches = get_matches(rx.recv().await.unwrap());
        assert_eq!(matches.len(), 2);

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_send_maximum_of_limit_results_with_pagination_if_specified() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", "some\nlines of text in\na\nfile"),
            ("path/to/file2.txt", "more text"),
            ("other/file.txt", "some other file with text"),
            ("other/dir/bin", "asdfasdfasdfasdfasdfasdfasdfasdfasdf"),
        ]);

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Contents,
            condition: SearchQueryCondition::regex("text"),
            options: SearchQueryOptions {
                pagination: Some(1),
                limit: Some(2),
                ..Default::default()
            },
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        // Verify that we get one match at a time up to the limit
        let matches = get_matches(rx.recv().await.unwrap());
        assert_eq!(matches.len(), 1);

        let matches = get_matches(rx.recv().await.unwrap());
        assert_eq!(matches.len(), 1);

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_traverse_no_deeper_than_max_depth_if_specified() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", ""),
            ("path/to/file2.txt", ""),
            ("other/file.txt", ""),
            ("other/dir/bin", ""),
        ]);

        async fn test_max_depth(
            root: &assert_fs::TempDir,
            depth: u64,
            expected_paths: Vec<PathBuf>,
        ) {
            let state = SearchState::new();
            let (reply, mut rx) = mpsc::channel(100);
            let query = SearchQuery {
                paths: vec![root.path().to_path_buf()],
                target: SearchQueryTarget::Path,
                condition: SearchQueryCondition::regex(".*"),
                options: SearchQueryOptions {
                    max_depth: Some(depth),
                    ..Default::default()
                },
            };

            let search_id = state.start(query, Box::new(reply)).await.unwrap();

            let mut paths = get_matches(rx.recv().await.unwrap())
                .into_iter()
                .filter_map(|m| m.into_path_match())
                .map(|m| m.path)
                .collect::<Vec<_>>();

            paths.sort_unstable();

            assert_eq!(paths, expected_paths);

            let data = rx.recv().await;
            assert_eq!(data, Some(Response::SearchDone { id: search_id }));

            assert_eq!(rx.recv().await, None);
        }

        // Maximum depth of 0 should only include root
        test_max_depth(&root, 0, vec![root.to_path_buf()]).await;

        // Maximum depth of 1 should only include root and children
        test_max_depth(
            &root,
            1,
            vec![
                root.to_path_buf(),
                root.child(make_path("other")).to_path_buf(),
                root.child(make_path("path")).to_path_buf(),
            ],
        )
        .await;

        // Maximum depth of 2 should only include root and children and grandchildren
        test_max_depth(
            &root,
            2,
            vec![
                root.to_path_buf(),
                root.child(make_path("other")).to_path_buf(),
                root.child(make_path("other/dir")).to_path_buf(),
                root.child(make_path("other/file.txt")).to_path_buf(),
                root.child(make_path("path")).to_path_buf(),
                root.child(make_path("path/to")).to_path_buf(),
            ],
        )
        .await;

        // Maximum depth of 3 should include everything we have in our test
        test_max_depth(
            &root,
            3,
            vec![
                root.to_path_buf(),
                root.child(make_path("other")).to_path_buf(),
                root.child(make_path("other/dir")).to_path_buf(),
                root.child(make_path("other/dir/bin")).to_path_buf(),
                root.child(make_path("other/file.txt")).to_path_buf(),
                root.child(make_path("path")).to_path_buf(),
                root.child(make_path("path/to")).to_path_buf(),
                root.child(make_path("path/to/file1.txt")).to_path_buf(),
                root.child(make_path("path/to/file2.txt")).to_path_buf(),
            ],
        )
        .await;
    }

    #[test(tokio::test)]
    async fn should_filter_searched_paths_to_only_those_that_match_include_regex() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", "some\nlines of text in\na\nfile"),
            ("path/to/file2.txt", "more text"),
            ("other/file.txt", "some other file with text"),
            ("other/dir/bin", "asdfasdfasdfasdfasdfasdfasdfasdfasdf"),
        ]);

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Contents,
            condition: SearchQueryCondition::regex("text"),
            options: SearchQueryOptions {
                include: Some(SearchQueryCondition::regex("other")),
                ..Default::default()
            },
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let mut matches = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_contents_match())
            .collect::<Vec<_>>();

        matches.sort_unstable_by_key(|m| m.path.to_path_buf());

        assert_eq!(
            matches,
            vec![SearchQueryContentsMatch {
                path: root.child(make_path("other/file.txt")).to_path_buf(),
                lines: SearchQueryMatchData::text("some other file with text"),
                line_number: 1,
                absolute_offset: 0,
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text("text".to_string()),
                    start: 21,
                    end: 25,
                }]
            }]
        );

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_filter_searched_paths_to_only_those_that_do_not_match_exclude_regex() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", "some\nlines of text in\na\nfile"),
            ("path/to/file2.txt", "more text"),
            ("other/file.txt", "some other file with text"),
            ("other/dir/bin", "asdfasdfasdfasdfasdfasdfasdfasdfasdf"),
        ]);

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Contents,
            condition: SearchQueryCondition::regex("text"),
            options: SearchQueryOptions {
                exclude: Some(SearchQueryCondition::regex("other")),
                ..Default::default()
            },
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let mut matches = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_contents_match())
            .collect::<Vec<_>>();

        matches.sort_unstable_by_key(|m| m.path.to_path_buf());

        assert_eq!(
            matches,
            vec![
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file1.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("lines of text in\n"),
                    line_number: 2,
                    absolute_offset: 5,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("text".to_string()),
                        start: 9,
                        end: 13,
                    }]
                },
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file2.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("more text"),
                    line_number: 1,
                    absolute_offset: 0,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("text".to_string()),
                        start: 5,
                        end: 9,
                    }]
                }
            ]
        );

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_return_binary_match_data_if_match_is_not_utf8_but_path_is_explicit() {
        let root = assert_fs::TempDir::new().unwrap();
        let bin_file = root.child(make_path("file.bin"));

        // Write some invalid bytes, a newline, and then "hello"
        bin_file
            .write_binary(&[0, 159, 146, 150, 10, 72, 69, 76, 76, 79])
            .unwrap();

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        // NOTE: We provide regex that matches an invalid UTF-8 character by disabling the u flag
        //       and checking for 0x9F (159)
        let query = SearchQuery {
            paths: vec![bin_file.path().to_path_buf()],
            target: SearchQueryTarget::Contents,
            condition: SearchQueryCondition::regex(r"(?-u:\x9F)"),
            options: Default::default(),
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let matches = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_contents_match())
            .collect::<Vec<_>>();

        // NOTE: Null bytes are treated as newlines, so that shifts us to being on "line 2"
        //       and associated other shifts
        assert_eq!(
            matches,
            vec![SearchQueryContentsMatch {
                path: root.child(make_path("file.bin")).to_path_buf(),
                lines: SearchQueryMatchData::bytes([159, 146, 150, 10]),
                line_number: 2,
                absolute_offset: 1,
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::bytes([159]),
                    start: 0,
                    end: 1,
                }]
            },]
        );

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_not_return_binary_match_data_if_match_is_not_utf8_and_not_explicit_path() {
        let root = assert_fs::TempDir::new().unwrap();
        let bin_file = root.child(make_path("file.bin"));

        // Write some invalid bytes, a newline, and then "hello"
        bin_file
            .write_binary(&[0, 159, 146, 150, 10, 72, 69, 76, 76, 79])
            .unwrap();

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        // NOTE: We provide regex that matches an invalid UTF-8 character by disabling the u flag
        //       and checking for 0x9F (159)
        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Contents,
            condition: SearchQueryCondition::regex(r"(?-u:\x9F)"),
            options: Default::default(),
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        // Get done indicator next as there were no matches
        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_filter_searched_paths_to_only_those_are_an_allowed_file_type() {
        let root = assert_fs::TempDir::new().unwrap();
        let file = root.child(make_path("file"));
        file.touch().unwrap();
        root.child(make_path("dir")).create_dir_all().unwrap();
        root.child(make_path("symlink"))
            .symlink_to_file(file.path())
            .unwrap();

        async fn test_allowed_file_types(
            root: &assert_fs::TempDir,
            allowed_file_types: Vec<FileType>,
            expected_paths: Vec<PathBuf>,
        ) {
            let state = SearchState::new();
            let (reply, mut rx) = mpsc::channel(100);

            let query = SearchQuery {
                paths: vec![root.path().to_path_buf()],
                target: SearchQueryTarget::Path,
                condition: SearchQueryCondition::regex(".*"),
                options: SearchQueryOptions {
                    allowed_file_types: allowed_file_types.iter().copied().collect(),
                    ..Default::default()
                },
            };

            let search_id = state.start(query, Box::new(reply)).await.unwrap();

            let mut paths = get_matches(rx.recv().await.unwrap())
                .into_iter()
                .filter_map(|m| m.into_path_match())
                .map(|m| m.path)
                .collect::<Vec<_>>();

            paths.sort_unstable();

            assert_eq!(
                paths, expected_paths,
                "Path types did not match allowed: {allowed_file_types:?}"
            );

            let data = rx.recv().await;
            assert_eq!(data, Some(Response::SearchDone { id: search_id }));

            assert_eq!(rx.recv().await, None);
        }

        // Empty set of allowed types falls back to allowing everything
        test_allowed_file_types(
            &root,
            vec![],
            vec![
                root.to_path_buf(),
                root.child("dir").to_path_buf(),
                root.child("file").to_path_buf(),
                root.child("symlink").to_path_buf(),
            ],
        )
        .await;

        test_allowed_file_types(
            &root,
            vec![FileType::File],
            vec![root.child("file").to_path_buf()],
        )
        .await;

        test_allowed_file_types(
            &root,
            vec![FileType::Dir],
            vec![root.to_path_buf(), root.child("dir").to_path_buf()],
        )
        .await;

        test_allowed_file_types(
            &root,
            vec![FileType::Symlink],
            vec![root.child("symlink").to_path_buf()],
        )
        .await;
    }

    #[test(tokio::test)]
    async fn should_follow_not_symbolic_links_if_specified_in_options() {
        let root = assert_fs::TempDir::new().unwrap();

        let file = root.child(make_path("file"));
        file.touch().unwrap();
        let dir = root.child(make_path("dir"));
        dir.create_dir_all().unwrap();
        root.child(make_path("file_symlink"))
            .symlink_to_file(file.path())
            .unwrap();
        root.child(make_path("dir_symlink"))
            .symlink_to_dir(dir.path())
            .unwrap();

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::regex(".*"),
            options: SearchQueryOptions {
                follow_symbolic_links: true,
                ..Default::default()
            },
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let mut paths = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_path_match())
            .map(|m| m.path)
            .collect::<Vec<_>>();

        paths.sort_unstable();

        assert_eq!(
            paths,
            vec![
                root.to_path_buf(),
                root.child("dir").to_path_buf(),
                root.child("dir_symlink").to_path_buf(),
                root.child("file").to_path_buf(),
                root.child("file_symlink").to_path_buf(),
            ]
        );

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_follow_symbolic_links_if_specified_in_options() {
        let root = assert_fs::TempDir::new().unwrap();

        let file = root.child(make_path("file"));
        file.touch().unwrap();
        let dir = root.child(make_path("dir"));
        dir.create_dir_all().unwrap();
        root.child(make_path("file_symlink"))
            .symlink_to_file(file.path())
            .unwrap();
        root.child(make_path("dir_symlink"))
            .symlink_to_dir(dir.path())
            .unwrap();

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        // NOTE: Following symlobic links on its own does nothing, but when combined with a file
        //       type filter, it will evaluate the underlying type of symbolic links and filter
        //       based on that instead of the the symbolic link
        let query = SearchQuery {
            paths: vec![root.path().to_path_buf()],
            target: SearchQueryTarget::Path,
            condition: SearchQueryCondition::regex(".*"),
            options: SearchQueryOptions {
                allowed_file_types: vec![FileType::File].into_iter().collect(),
                follow_symbolic_links: true,
                ..Default::default()
            },
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let mut paths = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_path_match())
            .map(|m| m.path)
            .collect::<Vec<_>>();

        paths.sort_unstable();

        assert_eq!(
            paths,
            vec![
                root.child("file").to_path_buf(),
                root.child("file_symlink").to_path_buf(),
            ]
        );

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_support_being_supplied_more_than_one_path() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", "some\nlines of text in\na\nfile"),
            ("path/to/file2.txt", "more text"),
        ]);

        let state = SearchState::new();
        let (reply, mut rx) = mpsc::channel(100);

        let query = SearchQuery {
            paths: vec![
                root.child(make_path("path/to/file1.txt"))
                    .path()
                    .to_path_buf(),
                root.child(make_path("path/to/file2.txt"))
                    .path()
                    .to_path_buf(),
            ],
            target: SearchQueryTarget::Contents,
            condition: SearchQueryCondition::regex("text"),
            options: Default::default(),
        };

        let search_id = state.start(query, Box::new(reply)).await.unwrap();

        let mut matches = get_matches(rx.recv().await.unwrap())
            .into_iter()
            .filter_map(|m| m.into_contents_match())
            .collect::<Vec<_>>();

        matches.sort_unstable_by_key(|m| m.path.to_path_buf());

        assert_eq!(
            matches,
            vec![
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file1.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("lines of text in\n"),
                    line_number: 2,
                    absolute_offset: 5,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("text".to_string()),
                        start: 9,
                        end: 13,
                    }]
                },
                SearchQueryContentsMatch {
                    path: root.child(make_path("path/to/file2.txt")).to_path_buf(),
                    lines: SearchQueryMatchData::text("more text"),
                    line_number: 1,
                    absolute_offset: 0,
                    submatches: vec![SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text("text".to_string()),
                        start: 5,
                        end: 9,
                    }]
                }
            ]
        );

        let data = rx.recv().await;
        assert_eq!(data, Some(Response::SearchDone { id: search_id }));

        assert_eq!(rx.recv().await, None);
    }

    #[test(tokio::test)]
    async fn should_support_searching_upward_with_max_depth_applying_in_reverse() {
        let root = setup_dir(vec![
            ("path/to/file1.txt", ""),
            ("path/to/file2.txt", ""),
            ("path/to/child/file1.txt", ""),
            ("path/to/child/file2.txt", ""),
            ("path/file1.txt", ""),
            ("path/file2.txt", ""),
            ("other/file1.txt", ""),
            ("other/file2.txt", ""),
            ("file1.txt", ""),
            ("file2.txt", ""),
        ]);

        // Make a path within root path
        let p = |path: &str| root.child(make_path(path)).to_path_buf();

        async fn test_max_depth(
            path: PathBuf,
            regex: &str,
            depth: impl Into<Option<u64>>,
            expected_paths: Vec<PathBuf>,
        ) {
            let state = SearchState::new();
            let (reply, mut rx) = mpsc::channel(100);
            let query = SearchQuery {
                paths: vec![path],
                target: SearchQueryTarget::Path,
                condition: SearchQueryCondition::regex(regex),
                options: SearchQueryOptions {
                    max_depth: depth.into(),
                    upward: true,
                    ..Default::default()
                },
            };

            let search_id = state.start(query, Box::new(reply)).await.unwrap();

            // If we expect to get no paths, then there won't be results, otherwise check
            if !expected_paths.is_empty() {
                let mut paths = get_matches(rx.recv().await.unwrap())
                    .into_iter()
                    .filter_map(|m| m.into_path_match())
                    .map(|m| m.path)
                    .collect::<Vec<_>>();

                paths.sort_unstable();

                assert_eq!(paths, expected_paths);
            }

            let data = rx.recv().await;
            assert_eq!(data, Some(Response::SearchDone { id: search_id }));

            assert_eq!(rx.recv().await, None);
        }

        // Maximum depth of 0 should only include current file if it matches
        test_max_depth(
            p("path/to/file1.txt"),
            "to",
            0,
            vec![p("path/to/file1.txt")],
        )
        .await;
        test_max_depth(p("path/to"), "other", 0, vec![]).await;

        // Maximum depth of 0 will still look through an explicit path's entries
        test_max_depth(
            p("path/to"),
            "to",
            0,
            vec![
                p("path/to"),
                p("path/to/child"),
                p("path/to/file1.txt"),
                p("path/to/file2.txt"),
            ],
        )
        .await;

        // Maximum depth of 1 should only include path and its parent directory & entries
        test_max_depth(
            p("path/to/file1.txt"),
            "to",
            1,
            vec![
                p("path/to"),
                p("path/to/child"),
                p("path/to/file1.txt"),
                p("path/to/file2.txt"),
            ],
        )
        .await;

        // Maximum depth of 2 should search path, parent, and grandparent
        test_max_depth(
            p("path/to/file1.txt"),
            "file1",
            2,
            vec![p("path/file1.txt"), p("path/to/file1.txt")],
        )
        .await;

        // Maximum depth greater than total path elements should just search all of them
        test_max_depth(
            p("path/to/file1.txt"),
            "file1",
            99,
            vec![p("file1.txt"), p("path/file1.txt"), p("path/to/file1.txt")],
        )
        .await;

        // No max depth will also search all ancestor paths
        test_max_depth(
            p("path/to/file1.txt"),
            "file1",
            None,
            vec![p("file1.txt"), p("path/file1.txt"), p("path/to/file1.txt")],
        )
        .await;
    }
}
