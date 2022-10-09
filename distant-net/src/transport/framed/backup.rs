use super::{Frame, OwnedFrame};
use bytes::BytesMut;
use std::collections::VecDeque;
use std::io;

/// Maximum size (in bytes) for saved frames (256MiB)
const MAX_BACKUP_SIZE: usize = 256 * 1024 * 1024;

/// Stores [`Frame`]s for reuse later.
#[derive(Clone, Debug)]
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
    sent_cnt: usize,

    /// Counter keeping track of total frames received
    received_cnt: usize,
}

impl Default for Backup {
    fn default() -> Self {
        Self::new()
    }
}

impl Backup {
    pub fn new() -> Self {
        Self {
            max_backup_size: MAX_BACKUP_SIZE,
            current_backup_size: 0,
            frames: VecDeque::new(),
            sent_cnt: 0,
            received_cnt: 0,
        }
    }

    /// Clears the backup of any stored data and resets the state to being new.
    pub fn clear(&mut self) {
        self.current_backup_size = 0;
        self.frames.clear();
        self.sent_cnt = 0;
        self.received_cnt = 0;
    }

    /// Sets the maximum size (in bytes) of collective frames stored in case a backup is needed
    /// during reconnection. Setting the `size` to 0 will result in no frames being stored.
    pub fn set_max_backup_size(&mut self, size: usize) {
        self.max_backup_size = size;
    }

    /// Returns the maximum size (in bytes) of collective frames stored in case a backup is needed
    /// during reconnection.
    pub fn max_backup_size(&self) -> usize {
        self.max_backup_size
    }

    /// Increments (by 1) the total sent frames.
    pub(super) fn increment_sent_cnt(&mut self) {
        self.sent_cnt += 1;
    }

    /// Returns how many frames have been sent.
    pub(super) fn sent_cnt(&self) -> usize {
        self.sent_cnt
    }

    /// Increments (by 1) the total received frames.
    pub(super) fn increment_received_cnt(&mut self) {
        self.received_cnt += 1;
    }

    /// Returns how many frames have been received.
    pub(super) fn received_cnt(&self) -> usize {
        self.received_cnt
    }

    /// Pushes a new frame to the end of the internal queue.
    pub(super) fn push_frame(&mut self, frame: Frame) {
        if self.max_backup_size > 0 {
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

    /// Writes all stored frames to the `dst` by invoking [`Frame::write`] in sequence.
    ///
    /// [`Frame::write`]: super::Frame::write
    pub(super) fn write(&self, dst: &mut BytesMut) -> io::Result<()> {
        for frame in self.frames.iter() {
            frame.write(dst)?;
        }

        Ok(())
    }

    /// Truncates the stored frames to be no larger than `size` total frames by popping from the
    /// front rather than the back of the list.
    pub(super) fn truncate_front(&mut self, size: usize) {
        while self.frames.len() > size {
            self.frames.pop_front();
        }
    }
}
