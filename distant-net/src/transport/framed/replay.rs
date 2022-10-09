use super::{Frame, OwnedFrame};
use bytes::BytesMut;
use std::collections::VecDeque;
use std::io;

/// Maximum size (in bytes) to save for replaying when reconnecting (256MiB)
const MAX_OUTGOING_REPLAY_SIZE: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Replayer {
    /// Maximum size (in bytes) to save frames in case we need to replay them
    ///
    /// NOTE: If 0, no frames will be stored.
    max_replay_size: usize,

    /// Tracker for the total size (in bytes) of frames stored for replay
    current_replay_size: usize,

    /// Storage used to hold outgoing frames in case they need to be replayed
    replay_frames: VecDeque<OwnedFrame>,

    /// Counter keeping track of total frames written by this transport
    sent_frame_cnt: usize,

    /// Counter keeping track of total frames received by this transport
    received_frame_cnt: usize,
}

impl Default for Replayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Replayer {
    pub fn new() -> Self {
        Self {
            max_replay_size: MAX_OUTGOING_REPLAY_SIZE,
            current_replay_size: 0,
            replay_frames: VecDeque::new(),
            sent_frame_cnt: 0,
            received_frame_cnt: 0,
        }
    }

    /// Sets the maximum size (in bytes) of collective frames stored in case a replay is needed
    /// during reconnection. Setting the `size` to 0 will result in no frames being stored.
    pub fn set_max_replay_size(&mut self, size: usize) {
        self.max_replay_size = size;
    }

    /// Returns the maximum size (in bytes) of collective frames stored in case a replay is needed
    /// during reconnection.
    pub fn max_replay_size(&self) -> usize {
        self.max_replay_size
    }

    /// Increments (by 1) the total sent frames.
    pub fn increment_sent_cnt(&mut self) {
        self.sent_frame_cnt += 1;
    }

    /// Returns how many frames have been sent.
    pub fn sent_cnt(&self) -> usize {
        self.sent_frame_cnt
    }

    /// Increments (by 1) the total received frames.
    pub fn increment_received_cnt(&mut self) {
        self.received_frame_cnt += 1;
    }

    /// Returns how many frames have been received.
    pub fn received_cnt(&self) -> usize {
        self.received_frame_cnt
    }

    /// Pushes a new frame to the end of the internal queue.
    pub fn push_frame(&mut self, frame: Frame) {
        if self.max_replay_size > 0 {
            self.current_replay_size += frame.len();
            self.replay_frames.push_back(frame.into_owned());
            while self.current_replay_size > self.max_replay_size {
                match self.replay_frames.pop_front() {
                    Some(frame) => {
                        self.current_replay_size -= frame.len();
                    }

                    // If we have exhausted all frames, then we have reached
                    // an internal size of 0 and should exit the loop
                    None => {
                        self.current_replay_size = 0;
                        break;
                    }
                }
            }
        }
    }

    /// Returns the total frames being kept for potential reuse.
    pub fn frame_cnt(&self) -> usize {
        self.replay_frames.len()
    }

    /// Writes all stored frames to the `dst` by invoking [`Frame::write`] in sequence.
    ///
    /// [`Frame::write`]: super::Frame::write
    pub fn write(&self, dst: &mut BytesMut) -> io::Result<()> {
        for frame in self.replay_frames.iter() {
            frame.write(dst)?;
        }

        Ok(())
    }

    /// Truncates the stored frames to be no larger than `size` total frames by popping from the
    /// front rather than the back of the list.
    pub fn truncate_front(&mut self, size: usize) {
        while self.replay_frames.len() > size {
            self.replay_frames.pop_front();
        }
    }

    /// Clears the replayer, resetting the sent and received counts as well as removing any stored
    /// frames that would be replayed.
    pub fn clear(&mut self) {
        self.sent_frame_cnt = 0;
        self.received_frame_cnt = 0;
        self.current_replay_size = 0;
        self.replay_frames.clear();
    }
}
