use std::collections::VecDeque;

use super::{Frame, OwnedFrame};

/// Maximum size (in bytes) for saved frames (256MiB)
const MAX_BACKUP_SIZE: usize = 256 * 1024 * 1024;

/// Stores [`Frame`]s for reuse later.
///
/// ### Note
///
/// Empty [`Frame`]s are an exception and are not stored within the backup nor
/// are they tracked in terms of sent/received counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backup {
    /// Maximum size (in bytes) to save frames in case we need to backup them
    ///
    /// NOTE: If 0, no frames will be stored.
    max_backup_size: usize,

    /// Tracker for the total size (in bytes) of stored frames
    current_backup_size: usize,

    /// Storage used to hold outgoing frames in case they need to be reused
    frames: VecDeque<OwnedFrame>,

    /// Counter keeping track of total frames sent
    sent_cnt: u64,

    /// Counter keeping track of total frames received
    received_cnt: u64,

    /// Indicates whether the backup is frozen, which indicates that mutations are ignored
    frozen: bool,
}

impl Default for Backup {
    fn default() -> Self {
        Self::new()
    }
}

impl Backup {
    /// Creates a new, unfrozen backup.
    pub fn new() -> Self {
        Self {
            max_backup_size: MAX_BACKUP_SIZE,
            current_backup_size: 0,
            frames: VecDeque::new(),
            sent_cnt: 0,
            received_cnt: 0,
            frozen: false,
        }
    }

    /// Clears the backup of any stored data and resets the state to being new.
    ///
    /// ### Note
    ///
    /// Like all other modifications, this will do nothing if the backup is frozen.
    pub fn clear(&mut self) {
        if !self.frozen {
            self.current_backup_size = 0;
            self.frames.clear();
            self.sent_cnt = 0;
            self.received_cnt = 0;
        }
    }

    /// Returns true if the backup is frozen, meaning that modifications will be ignored.
    #[inline]
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Sets the frozen status.
    #[inline]
    pub fn set_frozen(&mut self, frozen: bool) {
        self.frozen = frozen;
    }

    /// Marks the backup as frozen.
    #[inline]
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Marks the backup as no longer frozen.
    #[inline]
    pub fn unfreeze(&mut self) {
        self.frozen = false;
    }

    /// Sets the maximum size (in bytes) of collective frames stored in case a backup is needed
    /// during reconnection. Setting the `size` to 0 will result in no frames being stored.
    ///
    /// ### Note
    ///
    /// Like all other modifications, this will do nothing if the backup is frozen.
    pub fn set_max_backup_size(&mut self, size: usize) {
        if !self.frozen {
            self.max_backup_size = size;
        }
    }

    /// Returns the maximum size (in bytes) of collective frames stored in case a backup is needed
    /// during reconnection.
    pub fn max_backup_size(&self) -> usize {
        self.max_backup_size
    }

    /// Increments (by 1) the total sent frames.
    ///
    /// ### Note
    ///
    /// Like all other modifications, this will do nothing if the backup is frozen.
    pub(crate) fn increment_sent_cnt(&mut self) {
        if !self.frozen {
            self.sent_cnt += 1;
        }
    }

    /// Returns how many frames have been sent.
    pub(crate) fn sent_cnt(&self) -> u64 {
        self.sent_cnt
    }

    /// Increments (by 1) the total received frames.
    ///
    /// ### Note
    ///
    /// Like all other modifications, this will do nothing if the backup is frozen.
    pub(super) fn increment_received_cnt(&mut self) {
        if !self.frozen {
            self.received_cnt += 1;
        }
    }

    /// Returns how many frames have been received.
    pub(crate) fn received_cnt(&self) -> u64 {
        self.received_cnt
    }

    /// Sets the total received frames to the specified `cnt`.
    ///
    /// ### Note
    ///
    /// Like all other modifications, this will do nothing if the backup is frozen.
    pub(super) fn set_received_cnt(&mut self, cnt: u64) {
        if !self.frozen {
            self.received_cnt = cnt;
        }
    }

    /// Pushes a new frame to the end of the internal queue.
    ///
    /// ### Note
    ///
    /// Like all other modifications, this will do nothing if the backup is frozen.
    pub(crate) fn push_frame(&mut self, frame: Frame) {
        if self.max_backup_size > 0 && !self.frozen {
            self.current_backup_size += frame.len();
            self.frames.push_back(frame.into_owned());
            while self.current_backup_size > self.max_backup_size {
                match self.frames.pop_front() {
                    Some(frame) => {
                        self.current_backup_size -= frame.len();
                    }

                    // If we have exhausted all frames, then we have reached
                    // an internal size of 0 and should exit the loop
                    None => {
                        self.current_backup_size = 0;
                        break;
                    }
                }
            }
        }
    }

    /// Returns the total frames being kept for potential reuse.
    pub(super) fn frame_cnt(&self) -> usize {
        self.frames.len()
    }

    /// Returns an iterator over the frames contained in the backup.
    pub(super) fn frames(&self) -> impl Iterator<Item = &Frame<'_>> {
        self.frames.iter()
    }

    /// Truncates the stored frames to be no larger than `size` total frames by popping from the
    /// front rather than the back of the list.
    ///
    /// ### Note
    ///
    /// Like all other modifications, this will do nothing if the backup is frozen.
    pub(super) fn truncate_front(&mut self, size: usize) {
        if !self.frozen {
            while self.frames.len() > size {
                if let Some(frame) = self.frames.pop_front() {
                    self.current_backup_size -=
                        std::cmp::min(frame.len(), self.current_backup_size);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for Backup: frame storage with size limits, eviction, freezing (all mutations
    //! ignored), sent/received counters, truncation, and iterator ordering.

    use test_log::test;

    use super::{Backup, Frame};

    // ---- new / Default ----

    #[test]
    fn new_creates_unfrozen_backup() {
        let backup = Backup::new();
        assert!(!backup.is_frozen());
    }

    #[test]
    fn new_creates_backup_with_zero_sent_cnt() {
        let backup = Backup::new();
        assert_eq!(backup.sent_cnt(), 0);
    }

    #[test]
    fn new_creates_backup_with_zero_received_cnt() {
        let backup = Backup::new();
        assert_eq!(backup.received_cnt(), 0);
    }

    #[test]
    fn new_creates_backup_with_zero_frame_cnt() {
        let backup = Backup::new();
        assert_eq!(backup.frame_cnt(), 0);
    }

    #[test]
    fn default_is_same_as_new() {
        let from_new = Backup::new();
        let from_default = Backup::default();
        assert_eq!(from_new, from_default);
    }

    // ---- push_frame ----

    #[test]
    fn push_frame_stores_frame() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"hello"));
        assert_eq!(backup.frame_cnt(), 1);
    }

    #[test]
    fn push_frame_does_not_automatically_increment_sent_cnt() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"hello"));
        assert_eq!(backup.sent_cnt(), 0);
    }

    #[test]
    fn push_frame_stores_multiple_frames() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"first"));
        backup.push_frame(Frame::new(b"second"));
        backup.push_frame(Frame::new(b"third"));
        assert_eq!(backup.frame_cnt(), 3);
    }

    #[test]
    fn push_frame_respects_max_backup_size_evicts_oldest() {
        let mut backup = Backup::new();
        // Set max to 10 bytes
        backup.set_max_backup_size(10);

        // Push a 5-byte frame
        backup.push_frame(Frame::new(b"12345"));
        assert_eq!(backup.frame_cnt(), 1);

        // Push another 5-byte frame (total 10, exactly at limit)
        backup.push_frame(Frame::new(b"67890"));
        assert_eq!(backup.frame_cnt(), 2);

        // Push a 3-byte frame (total would be 13, exceeds 10)
        // Should evict oldest until within limit
        backup.push_frame(Frame::new(b"abc"));
        // After evicting "12345" (5 bytes), total = 5 + 3 = 8, within limit
        assert_eq!(backup.frame_cnt(), 2);

        // Verify the remaining frames are "67890" and "abc"
        let frames: Vec<&[u8]> = backup.frames().map(|f| f.as_item()).collect();
        assert_eq!(frames, vec![b"67890".as_slice(), b"abc".as_slice()]);
    }

    #[test]
    fn push_frame_with_max_backup_size_zero_stores_nothing() {
        let mut backup = Backup::new();
        backup.set_max_backup_size(0);
        backup.push_frame(Frame::new(b"hello"));
        assert_eq!(backup.frame_cnt(), 0);
    }

    #[test]
    fn push_frame_while_frozen_does_nothing() {
        let mut backup = Backup::new();
        backup.freeze();
        backup.push_frame(Frame::new(b"hello"));
        assert_eq!(backup.frame_cnt(), 0);
    }

    // ---- clear ----

    #[test]
    fn clear_resets_frames() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"hello"));
        backup.increment_sent_cnt();
        backup.increment_received_cnt();
        assert_eq!(backup.frame_cnt(), 1);
        assert_eq!(backup.sent_cnt(), 1);
        assert_eq!(backup.received_cnt(), 1);

        backup.clear();

        assert_eq!(backup.frame_cnt(), 0);
        assert_eq!(backup.sent_cnt(), 0);
        assert_eq!(backup.received_cnt(), 0);
    }

    #[test]
    fn clear_while_frozen_does_nothing() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"hello"));
        backup.increment_sent_cnt();
        backup.increment_received_cnt();
        backup.freeze();

        backup.clear();

        assert_eq!(backup.frame_cnt(), 1);
        assert_eq!(backup.sent_cnt(), 1);
        assert_eq!(backup.received_cnt(), 1);
    }

    // ---- freeze / unfreeze / set_frozen / is_frozen ----

    #[test]
    fn freeze_sets_frozen_to_true() {
        let mut backup = Backup::new();
        assert!(!backup.is_frozen());
        backup.freeze();
        assert!(backup.is_frozen());
    }

    #[test]
    fn unfreeze_sets_frozen_to_false() {
        let mut backup = Backup::new();
        backup.freeze();
        assert!(backup.is_frozen());
        backup.unfreeze();
        assert!(!backup.is_frozen());
    }

    #[test]
    fn set_frozen_true() {
        let mut backup = Backup::new();
        backup.set_frozen(true);
        assert!(backup.is_frozen());
    }

    #[test]
    fn set_frozen_false() {
        let mut backup = Backup::new();
        backup.freeze();
        backup.set_frozen(false);
        assert!(!backup.is_frozen());
    }

    // ---- increment_sent_cnt / sent_cnt ----

    #[test]
    fn increment_sent_cnt_increases_by_one() {
        let mut backup = Backup::new();
        backup.increment_sent_cnt();
        assert_eq!(backup.sent_cnt(), 1);
        backup.increment_sent_cnt();
        assert_eq!(backup.sent_cnt(), 2);
    }

    #[test]
    fn increment_sent_cnt_while_frozen_does_nothing() {
        let mut backup = Backup::new();
        backup.increment_sent_cnt();
        assert_eq!(backup.sent_cnt(), 1);
        backup.freeze();
        backup.increment_sent_cnt();
        assert_eq!(backup.sent_cnt(), 1);
    }

    // ---- increment_received_cnt / received_cnt ----

    #[test]
    fn increment_received_cnt_increases_by_one() {
        let mut backup = Backup::new();
        backup.increment_received_cnt();
        assert_eq!(backup.received_cnt(), 1);
        backup.increment_received_cnt();
        assert_eq!(backup.received_cnt(), 2);
    }

    #[test]
    fn increment_received_cnt_while_frozen_does_nothing() {
        let mut backup = Backup::new();
        backup.increment_received_cnt();
        assert_eq!(backup.received_cnt(), 1);
        backup.freeze();
        backup.increment_received_cnt();
        assert_eq!(backup.received_cnt(), 1);
    }

    // ---- set_received_cnt ----

    #[test]
    fn set_received_cnt_updates_value() {
        let mut backup = Backup::new();
        backup.set_received_cnt(42);
        assert_eq!(backup.received_cnt(), 42);
    }

    #[test]
    fn set_received_cnt_while_frozen_does_nothing() {
        let mut backup = Backup::new();
        backup.set_received_cnt(10);
        backup.freeze();
        backup.set_received_cnt(99);
        assert_eq!(backup.received_cnt(), 10);
    }

    // ---- set_max_backup_size / max_backup_size ----

    #[test]
    fn set_max_backup_size_updates_value() {
        let mut backup = Backup::new();
        backup.set_max_backup_size(1024);
        assert_eq!(backup.max_backup_size(), 1024);
    }

    #[test]
    fn set_max_backup_size_while_frozen_does_nothing() {
        let mut backup = Backup::new();
        let original = backup.max_backup_size();
        backup.freeze();
        backup.set_max_backup_size(1024);
        assert_eq!(backup.max_backup_size(), original);
    }

    #[test]
    fn max_backup_size_default_is_256_mib() {
        let backup = Backup::new();
        assert_eq!(backup.max_backup_size(), 256 * 1024 * 1024);
    }

    // ---- truncate_front ----

    #[test]
    fn truncate_front_removes_oldest_frames() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"first"));
        backup.push_frame(Frame::new(b"second"));
        backup.push_frame(Frame::new(b"third"));
        assert_eq!(backup.frame_cnt(), 3);

        backup.truncate_front(1);
        assert_eq!(backup.frame_cnt(), 1);

        let frames: Vec<&[u8]> = backup.frames().map(|f| f.as_item()).collect();
        assert_eq!(frames, vec![b"third".as_slice()]);
    }

    #[test]
    fn truncate_front_does_nothing_if_already_within_size() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"only"));
        assert_eq!(backup.frame_cnt(), 1);

        backup.truncate_front(5);
        assert_eq!(backup.frame_cnt(), 1);
    }

    #[test]
    fn truncate_front_to_zero_removes_all_frames() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"first"));
        backup.push_frame(Frame::new(b"second"));

        backup.truncate_front(0);
        assert_eq!(backup.frame_cnt(), 0);
    }

    #[test]
    fn truncate_front_while_frozen_does_nothing() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"first"));
        backup.push_frame(Frame::new(b"second"));
        backup.freeze();

        backup.truncate_front(0);
        assert_eq!(backup.frame_cnt(), 2);
    }

    // ---- frame_cnt and frames iterator ----

    #[test]
    fn frame_cnt_matches_number_of_pushed_frames() {
        let mut backup = Backup::new();
        assert_eq!(backup.frame_cnt(), 0);
        backup.push_frame(Frame::new(b"a"));
        assert_eq!(backup.frame_cnt(), 1);
        backup.push_frame(Frame::new(b"b"));
        assert_eq!(backup.frame_cnt(), 2);
    }

    #[test]
    fn frames_iterator_returns_frames_in_order() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"first"));
        backup.push_frame(Frame::new(b"second"));
        backup.push_frame(Frame::new(b"third"));

        let items: Vec<&[u8]> = backup.frames().map(|f| f.as_item()).collect();
        assert_eq!(
            items,
            vec![
                b"first".as_slice(),
                b"second".as_slice(),
                b"third".as_slice()
            ]
        );
    }

    #[test]
    fn frames_iterator_is_empty_for_new_backup() {
        let backup = Backup::new();
        assert_eq!(backup.frames().count(), 0);
    }

    // ---- all mutations ignored when frozen ----

    #[test]
    fn all_mutations_ignored_when_frozen() {
        let mut backup = Backup::new();
        // Set up some initial state
        backup.push_frame(Frame::new(b"existing"));
        backup.increment_sent_cnt();
        backup.increment_received_cnt();

        // Freeze
        backup.freeze();

        // Try all mutations
        backup.push_frame(Frame::new(b"should not appear"));
        backup.increment_sent_cnt();
        backup.increment_received_cnt();
        backup.set_received_cnt(100);
        backup.set_max_backup_size(0);
        backup.clear();
        backup.truncate_front(0);

        // Verify nothing changed
        assert_eq!(backup.frame_cnt(), 1);
        assert_eq!(backup.sent_cnt(), 1);
        assert_eq!(backup.received_cnt(), 1);
        assert_eq!(backup.max_backup_size(), 256 * 1024 * 1024);
    }

    // ---- mutations work after unfreeze ----

    #[test]
    fn mutations_work_after_unfreeze() {
        let mut backup = Backup::new();
        backup.push_frame(Frame::new(b"before freeze"));
        backup.freeze();

        // This should be ignored
        backup.push_frame(Frame::new(b"during freeze"));
        assert_eq!(backup.frame_cnt(), 1);

        backup.unfreeze();

        // This should work
        backup.push_frame(Frame::new(b"after unfreeze"));
        assert_eq!(backup.frame_cnt(), 2);
    }
}
