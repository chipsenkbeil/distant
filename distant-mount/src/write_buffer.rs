//! Per-file write accumulation buffers for buffered write-back.
//!
//! Writes are buffered in memory during a file handle's lifetime and flushed
//! to the remote on `flush`, `fsync`, or `release`. Dirty byte ranges are
//! tracked to minimize the data sent on flush.

use std::collections::HashMap;
use std::ops::Range;

/// Accumulates writes for a single file and tracks which byte ranges are dirty.
///
/// Writes are stored in a contiguous `Vec<u8>` buffer. Gaps between the
/// original file size and a write offset are zero-filled. Overlapping or
/// adjacent dirty ranges are coalesced to keep the range list compact.
pub(crate) struct WriteBuffer {
    data: Vec<u8>,
    dirty_ranges: Vec<Range<u64>>,
    original_size: u64,
}

impl WriteBuffer {
    /// Creates an empty write buffer for a file of the given size.
    ///
    /// The buffer starts empty; bytes are only materialized when
    /// [`write`](Self::write) is called.
    pub(crate) fn new(original_size: u64) -> Self {
        Self {
            data: Vec::new(),
            dirty_ranges: Vec::new(),
            original_size,
        }
    }

    /// Buffers a write at the given byte offset.
    ///
    /// If the write extends beyond the current buffer length, the buffer is
    /// grown and any gap between the old end and the new write is zero-filled.
    /// Overlapping or adjacent dirty ranges are merged.
    pub(crate) fn write(&mut self, offset: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }

        let end = offset + data.len() as u64;

        // Grow the buffer if necessary, zero-filling any gap.
        let required_len = end as usize;
        if required_len > self.data.len() {
            self.data.resize(required_len, 0);
        }

        self.data[offset as usize..end as usize].copy_from_slice(data);
        self.merge_range(offset..end);
    }

    /// Returns `true` if any writes have been buffered.
    pub(crate) fn is_dirty(&self) -> bool {
        !self.dirty_ranges.is_empty()
    }

    /// Returns the dirty byte ranges, coalesced and sorted by start offset.
    pub(crate) fn dirty_ranges(&self) -> &[Range<u64>] {
        &self.dirty_ranges
    }

    /// Returns the full buffer contents.
    ///
    /// Regions that were never written to contain zeros.
    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns the original file size this buffer was created for.
    #[allow(dead_code)]
    pub(crate) fn original_size(&self) -> u64 {
        self.original_size
    }

    /// Clears the buffer and dirty ranges after a successful flush.
    pub(crate) fn clear(&mut self) {
        self.data.clear();
        self.dirty_ranges.clear();
    }

    /// Inserts a range into the dirty list, merging with any overlapping or
    /// adjacent ranges.
    fn merge_range(&mut self, new: Range<u64>) {
        // Find all existing ranges that overlap or are adjacent to `new`.
        // A range `r` overlaps/is adjacent when r.start <= new.end AND r.end >= new.start.
        let mut merged_start = new.start;
        let mut merged_end = new.end;
        let mut i = 0;

        while i < self.dirty_ranges.len() {
            let r = &self.dirty_ranges[i];
            if r.start <= merged_end && r.end >= merged_start {
                // This range overlaps or is adjacent; absorb it.
                merged_start = merged_start.min(r.start);
                merged_end = merged_end.max(r.end);
                self.dirty_ranges.swap_remove(i);
                // Don't increment i — the swapped-in element needs checking too.
            } else {
                i += 1;
            }
        }

        self.dirty_ranges.push(merged_start..merged_end);
        self.dirty_ranges.sort_by_key(|r| r.start);
    }
}

/// Collection of [`WriteBuffer`]s keyed by inode number.
pub(crate) struct WriteBuffers {
    buffers: HashMap<u64, WriteBuffer>,
}

impl WriteBuffers {
    /// Creates an empty collection.
    pub(crate) fn new() -> Self {
        Self {
            buffers: HashMap::new(),
        }
    }

    /// Returns a mutable reference to the buffer for `ino`, creating one if it
    /// does not already exist.
    pub(crate) fn get_or_create(&mut self, ino: u64, original_size: u64) -> &mut WriteBuffer {
        self.buffers
            .entry(ino)
            .or_insert_with(|| WriteBuffer::new(original_size))
    }

    /// Returns a shared reference to the buffer for `ino`, if one exists.
    #[allow(dead_code)]
    pub(crate) fn get(&self, ino: u64) -> Option<&WriteBuffer> {
        self.buffers.get(&ino)
    }

    /// Returns a mutable reference to the buffer for `ino`, if one exists.
    #[allow(dead_code)]
    pub(crate) fn get_mut(&mut self, ino: u64) -> Option<&mut WriteBuffer> {
        self.buffers.get_mut(&ino)
    }

    /// Removes and returns the buffer for `ino`, if one exists.
    pub(crate) fn remove(&mut self, ino: u64) -> Option<WriteBuffer> {
        self.buffers.remove(&ino)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_buffer_should_store_and_return_written_data() {
        let mut buf = WriteBuffer::new(0);
        buf.write(0, b"hello");

        assert_eq!(buf.data(), b"hello");
    }

    #[test]
    fn write_buffer_should_zero_fill_gap_before_offset() {
        let mut buf = WriteBuffer::new(100);
        buf.write(5, b"world");

        assert_eq!(buf.data().len(), 10);
        assert_eq!(&buf.data()[..5], &[0, 0, 0, 0, 0]);
        assert_eq!(&buf.data()[5..], b"world");
    }

    #[test]
    fn write_buffer_should_overwrite_existing_region() {
        let mut buf = WriteBuffer::new(0);
        buf.write(0, b"hello");
        buf.write(0, b"HELLO");

        assert_eq!(buf.data(), b"HELLO");
    }

    #[test]
    fn write_buffer_should_ignore_empty_write() {
        let mut buf = WriteBuffer::new(0);
        buf.write(10, b"");

        assert!(!buf.is_dirty());
        assert!(buf.data().is_empty());
    }

    #[test]
    fn write_buffer_should_track_single_dirty_range() {
        let mut buf = WriteBuffer::new(0);
        buf.write(0, b"hello");

        assert!(buf.is_dirty());
        assert_eq!(buf.dirty_ranges(), &[0..5]);
    }

    #[test]
    fn write_buffer_should_track_disjoint_dirty_ranges() {
        let mut buf = WriteBuffer::new(100);
        buf.write(0, b"hello");
        buf.write(10, b"world");

        assert_eq!(buf.dirty_ranges().len(), 2);
        assert_eq!(buf.dirty_ranges()[0], 0..5);
        assert_eq!(buf.dirty_ranges()[1], 10..15);
    }

    #[test]
    fn write_buffer_should_merge_overlapping_ranges() {
        let mut buf = WriteBuffer::new(0);
        buf.write(0, &[1; 10]);
        buf.write(5, &[2; 10]);

        assert_eq!(buf.dirty_ranges(), &[0..15]);
    }

    #[test]
    fn write_buffer_should_merge_adjacent_ranges() {
        let mut buf = WriteBuffer::new(0);
        buf.write(0, &[1; 5]);
        buf.write(5, &[2; 5]);

        assert_eq!(buf.dirty_ranges(), &[0..10]);
    }

    #[test]
    fn write_buffer_should_absorb_subset_range_into_superset() {
        let mut buf = WriteBuffer::new(0);
        buf.write(0, &[1; 20]);
        buf.write(5, &[2; 5]);

        assert_eq!(buf.dirty_ranges(), &[0..20]);
    }

    #[test]
    fn write_buffer_should_coalesce_three_ranges_via_bridging_write() {
        let mut buf = WriteBuffer::new(100);
        buf.write(0, &[1; 5]);
        buf.write(10, &[2; 5]);

        // This bridges the gap between the two existing ranges.
        buf.write(3, &[3; 10]);

        assert_eq!(buf.dirty_ranges(), &[0..15]);
    }

    #[test]
    fn write_buffer_should_reset_on_clear() {
        let mut buf = WriteBuffer::new(42);
        buf.write(0, b"data");
        assert!(buf.is_dirty());

        buf.clear();

        assert!(!buf.is_dirty());
        assert!(buf.data().is_empty());
        assert!(buf.dirty_ranges().is_empty());
    }

    #[test]
    fn write_buffers_should_return_same_buffer_for_same_inode() {
        let mut bufs = WriteBuffers::new();
        bufs.get_or_create(1, 100).write(0, b"hello");

        let buf = bufs.get_or_create(1, 100);
        assert_eq!(buf.data(), b"hello");
    }

    #[test]
    fn write_buffers_should_keep_inodes_independent() {
        let mut bufs = WriteBuffers::new();
        bufs.get_or_create(1, 0).write(0, b"one");
        bufs.get_or_create(2, 0).write(0, b"two");

        assert_eq!(bufs.get(1).map(WriteBuffer::data), Some(b"one".as_slice()));
        assert_eq!(bufs.get(2).map(WriteBuffer::data), Some(b"two".as_slice()));
    }

    #[test]
    fn write_buffers_should_return_buffer_on_remove() {
        let mut bufs = WriteBuffers::new();
        bufs.get_or_create(1, 0).write(0, b"data");

        let removed = bufs.remove(1);
        assert!(removed.is_some());
        assert_eq!(
            removed.as_ref().map(WriteBuffer::data),
            Some(b"data".as_slice()),
        );
        assert!(bufs.get(1).is_none());
    }

    #[test]
    fn write_buffers_should_return_none_for_nonexistent_inode() {
        let mut bufs = WriteBuffers::new();
        assert!(bufs.remove(999).is_none());
    }

    #[test]
    fn write_buffers_should_allow_mutation_via_get_mut() {
        let mut bufs = WriteBuffers::new();
        bufs.get_or_create(1, 0).write(0, b"old");

        if let Some(buf) = bufs.get_mut(1) {
            buf.write(0, b"new");
        }

        assert_eq!(bufs.get(1).map(WriteBuffer::data), Some(b"new".as_slice()));
    }
}
