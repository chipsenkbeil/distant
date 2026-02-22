use std::borrow::Cow;

use bytes::{Buf, BufMut, BytesMut};

/// Represents a frame whose lifetime is static
pub type OwnedFrame = Frame<'static>;

/// Represents some data wrapped in a frame in order to ship it over the network. The format is
/// simple and follows `{len}{item}` where `len` is the length of the item as a `u64`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    /// Represents the item that will be shipped across the network
    item: Cow<'a, [u8]>,
}

impl<'a> Frame<'a> {
    /// Creates a new frame wrapping the `item` that will be shipped across the network.
    pub fn new(item: &'a [u8]) -> Self {
        Self {
            item: Cow::Borrowed(item),
        }
    }

    /// Consumes the frame and returns its underlying item.
    pub fn into_item(self) -> Cow<'a, [u8]> {
        self.item
    }
}

impl Frame<'_> {
    /// Total bytes to use as the header field denoting a frame's size.
    pub const HEADER_SIZE: usize = 8;

    /// Creates a new frame with no item.
    pub fn empty() -> Self {
        Self::new(&[])
    }

    /// Returns the len (in bytes) of the item wrapped by the frame.
    pub fn len(&self) -> usize {
        self.item.len()
    }

    /// Returns true if the frame is comprised of zero bytes.
    pub fn is_empty(&self) -> bool {
        self.item.is_empty()
    }

    /// Returns true if the frame is comprised of some bytes.
    #[inline]
    pub fn is_nonempty(&self) -> bool {
        !self.is_empty()
    }

    /// Returns a reference to the bytes of the frame's item.
    pub fn as_item(&self) -> &[u8] {
        &self.item
    }

    /// Writes the frame to a new [`Vec`] of bytes, returning them on success.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = BytesMut::new();
        self.write(&mut bytes);
        bytes.to_vec()
    }

    /// Writes the frame to the end of `dst`, including the header representing the length of the
    /// item as part of the written bytes.
    pub fn write(&self, dst: &mut BytesMut) {
        dst.reserve(Self::HEADER_SIZE + self.item.len());

        // Add data in form of {LEN}{ITEM}
        dst.put_u64((self.item.len()) as u64);
        dst.put_slice(&self.item);
    }

    /// Attempts to read a frame from `src`, returning `Some(Frame)` if a frame was found
    /// (including the header) or `None` if the current `src` does not contain a frame.
    pub fn read(src: &mut BytesMut) -> Option<OwnedFrame> {
        // First, check if we have more data than just our frame's message length
        if src.len() <= Self::HEADER_SIZE {
            return None;
        }

        // Second, retrieve total size of our frame's message
        let item_len = u64::from_be_bytes(src[..Self::HEADER_SIZE].try_into().unwrap()) as usize;

        // Third, check if we have all data for our frame; if not, exit early
        if src.len() < item_len + Self::HEADER_SIZE {
            return None;
        }

        // Fourth, get and return our item
        let item = src[Self::HEADER_SIZE..(Self::HEADER_SIZE + item_len)].to_vec();

        // Fifth, advance so frame is no longer kept around
        src.advance(Self::HEADER_SIZE + item_len);

        Some(Frame::from(item))
    }

    /// Checks if a full frame is available from `src`, returning true if a frame was found false
    /// if the current `src` does not contain a frame. Does not consume the frame.
    pub fn available(src: &BytesMut) -> bool {
        Frame::read(&mut src.clone()).is_some()
    }

    /// Returns a new frame which is identical but has a lifetime tied to this frame.
    pub fn as_borrowed(&self) -> Frame<'_> {
        let item = match &self.item {
            Cow::Borrowed(x) => x,
            Cow::Owned(x) => x.as_slice(),
        };

        Frame {
            item: Cow::Borrowed(item),
        }
    }

    /// Converts the [`Frame`] into an owned copy.
    ///
    /// If you construct the frame from an item with a non-static lifetime, you may run into
    /// lifetime problems due to the way the struct is designed. Calling this function will ensure
    /// that the returned value has a static lifetime.
    ///
    /// This is different from just cloning. Cloning the frame will just copy the references, and
    /// thus the lifetime will remain the same.
    pub fn into_owned(self) -> OwnedFrame {
        Frame {
            item: Cow::from(self.item.into_owned()),
        }
    }
}

impl<'a> From<&'a [u8]> for Frame<'a> {
    /// Consumes the byte slice and returns a [`Frame`] whose item references those bytes.
    fn from(item: &'a [u8]) -> Self {
        Self {
            item: Cow::Borrowed(item),
        }
    }
}

impl<'a, const N: usize> From<&'a [u8; N]> for Frame<'a> {
    /// Consumes the byte array slice and returns a [`Frame`] whose item references those bytes.
    fn from(item: &'a [u8; N]) -> Self {
        Self {
            item: Cow::Borrowed(item),
        }
    }
}

impl<const N: usize> From<[u8; N]> for OwnedFrame {
    /// Consumes an array of bytes and returns a [`Frame`] with an owned item of those bytes
    /// allocated as a [`Vec`].
    fn from(item: [u8; N]) -> Self {
        Self {
            item: Cow::Owned(item.to_vec()),
        }
    }
}

impl From<Vec<u8>> for OwnedFrame {
    /// Consumes a [`Vec`] of bytes and returns a [`Frame`] with an owned item of those bytes.
    fn from(item: Vec<u8>) -> Self {
        Self {
            item: Cow::Owned(item),
        }
    }
}

impl AsRef<[u8]> for Frame<'_> {
    /// Returns a reference to this [`Frame`]'s item as bytes.
    fn as_ref(&self) -> &[u8] {
        AsRef::as_ref(&self.item)
    }
}

impl Extend<u8> for Frame<'_> {
    /// Extends the [`Frame`]'s item with the provided bytes, allocating an owned [`Vec`]
    /// underneath if this frame had borrowed bytes as an item.
    fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
        match &mut self.item {
            // If we only have a borrowed item, we need to allocate it into a new vec so we can
            // extend it with additional bytes
            Cow::Borrowed(item) => {
                let mut item = item.to_vec();
                item.extend(iter);
                self.item = Cow::Owned(item);
            }

            // Othewise, if we already have an owned allocation of bytes, we just extend it
            Cow::Owned(item) => {
                item.extend(iter);
            }
        }
    }
}

impl PartialEq<[u8]> for Frame<'_> {
    /// Test if [`Frame`]'s item matches the provided bytes.
    fn eq(&self, item: &[u8]) -> bool {
        self.item.as_ref().eq(item)
    }
}

impl<'a> PartialEq<&'a [u8]> for Frame<'_> {
    /// Test if [`Frame`]'s item matches the provided bytes.
    fn eq(&self, item: &&'a [u8]) -> bool {
        self.item.as_ref().eq(*item)
    }
}

impl<const N: usize> PartialEq<[u8; N]> for Frame<'_> {
    /// Test if [`Frame`]'s item matches the provided bytes.
    fn eq(&self, item: &[u8; N]) -> bool {
        self.item.as_ref().eq(item)
    }
}

impl<'a, const N: usize> PartialEq<&'a [u8; N]> for Frame<'_> {
    /// Test if [`Frame`]'s item matches the provided bytes.
    fn eq(&self, item: &&'a [u8; N]) -> bool {
        self.item.as_ref().eq(*item)
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn write_should_succeed_when_item_is_zero_bytes() {
        let frame = Frame::new(&[]);

        let mut buf = BytesMut::new();
        frame.write(&mut buf);

        // Writing a frame of zero bytes means that the header is all zeros and there is
        // no item that follows the header
        assert_eq!(buf.as_ref(), &[0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn write_should_build_a_frame_containing_a_length_and_item() {
        let frame = Frame::new(b"hello, world");

        let mut buf = BytesMut::new();
        frame.write(&mut buf);

        let len = buf.get_u64() as usize;
        assert_eq!(len, 12, "Wrong length writed");
        assert_eq!(buf.as_ref(), b"hello, world");
    }

    #[test]
    fn read_should_return_none_if_data_smaller_than_or_equal_to_item_length_field() {
        let mut buf = BytesMut::new();
        buf.put_bytes(0, Frame::HEADER_SIZE);

        let result = Frame::read(&mut buf);
        assert!(result.is_none(), "Unexpected result: {:?}", result);
    }

    #[test]
    fn read_should_return_none_if_not_enough_data_for_frame() {
        let mut buf = BytesMut::new();
        buf.put_u64(0);

        let result = Frame::read(&mut buf);
        assert!(result.is_none(), "Unexpected result: {:?}", result);
    }

    #[test]
    fn read_should_succeed_if_written_item_length_is_zero() {
        let mut buf = BytesMut::new();
        buf.put_u64(0);
        buf.put_u8(255);

        // Reading will result in a frame of zero bytes
        let frame = Frame::read(&mut buf).expect("missing frame");
        assert_eq!(frame, Frame::empty());

        // Nothing following the frame header should have been extracted
        assert_eq!(buf.as_ref(), &[255]);
    }

    #[test]
    fn read_should_advance_src_by_frame_size_even_if_item_length_is_zero() {
        let mut buf = BytesMut::new();
        buf.put_u64(0);
        buf.put_bytes(0, 3);

        assert_eq!(Frame::read(&mut buf).unwrap(), Frame::empty());
        assert_eq!(buf.len(), 3, "Advanced an unexpected amount in src buf");
    }

    #[test]
    fn read_should_advance_src_by_frame_size_when_successful() {
        // Add 3 extra bytes after a full frame
        let mut buf = BytesMut::new();
        Frame::new(b"hello, world").write(&mut buf);
        buf.put_bytes(0, 3);

        assert!(
            Frame::read(&mut buf).is_some(),
            "read unexpectedly missing frame"
        );
        assert_eq!(buf.len(), 3, "Advanced an unexpected amount in src buf");
    }

    #[test]
    fn read_should_return_some_byte_vec_when_successful() {
        let mut buf = BytesMut::new();
        Frame::new(b"hello, world").write(&mut buf);

        let item = Frame::read(&mut buf).expect("missing frame");
        assert_eq!(item, b"hello, world");
    }

    // --- Tests for constructors and basic accessors ---

    #[test]
    fn empty_should_create_frame_with_zero_length() {
        let frame = Frame::empty();
        assert!(frame.is_empty());
        assert_eq!(frame.len(), 0);
        assert!(!frame.is_nonempty());
    }

    #[test]
    fn new_should_borrow_the_provided_bytes() {
        let data = b"test data";
        let frame = Frame::new(data);
        assert_eq!(frame.len(), 9);
        assert!(!frame.is_empty());
        assert!(frame.is_nonempty());
    }

    #[test]
    fn as_item_should_return_reference_to_item_bytes() {
        let frame = Frame::new(b"hello");
        assert_eq!(frame.as_item(), b"hello");
    }

    #[test]
    fn as_item_on_empty_frame_should_return_empty_slice() {
        let frame = Frame::empty();
        assert_eq!(frame.as_item(), b"");
    }

    #[test]
    fn into_item_should_return_cow_with_borrowed_bytes() {
        let data = b"borrowed";
        let frame = Frame::new(data);
        let item = frame.into_item();
        assert_eq!(item.as_ref(), b"borrowed");
    }

    #[test]
    fn into_item_on_owned_frame_should_return_cow_with_owned_bytes() {
        let frame = OwnedFrame::from(vec![1, 2, 3]);
        let item = frame.into_item();
        assert_eq!(item.as_ref(), &[1, 2, 3]);
    }

    // --- Tests for to_bytes ---

    #[test]
    fn to_bytes_should_produce_header_and_item() {
        let frame = Frame::new(b"hi");
        let bytes = frame.to_bytes();
        // 8 bytes header (u64 len = 2) + 2 bytes item
        assert_eq!(bytes.len(), 10);
        assert_eq!(&bytes[..8], &[0, 0, 0, 0, 0, 0, 0, 2]);
        assert_eq!(&bytes[8..], b"hi");
    }

    #[test]
    fn to_bytes_on_empty_frame_should_produce_only_header() {
        let frame = Frame::empty();
        let bytes = frame.to_bytes();
        assert_eq!(bytes.len(), 8);
        assert_eq!(bytes, vec![0, 0, 0, 0, 0, 0, 0, 0]);
    }

    // --- Tests for into_owned and as_borrowed ---

    #[test]
    fn into_owned_should_produce_static_frame_from_borrowed() {
        let data = vec![10, 20, 30];
        let frame = Frame::new(&data);
        let owned: OwnedFrame = frame.into_owned();
        assert_eq!(owned.as_item(), &[10, 20, 30]);
    }

    #[test]
    fn as_borrowed_should_produce_frame_with_same_content() {
        let frame = OwnedFrame::from(vec![5, 6, 7]);
        let borrowed = frame.as_borrowed();
        assert_eq!(borrowed.as_item(), frame.as_item());
        assert_eq!(borrowed, frame);
    }

    #[test]
    fn as_borrowed_on_borrowed_frame() {
        let data = b"hello";
        let frame = Frame::new(data);
        let borrowed = frame.as_borrowed();
        assert_eq!(borrowed.as_item(), b"hello");
    }

    // --- Tests for available ---

    #[test]
    fn available_should_return_true_when_full_frame_present() {
        let mut buf = BytesMut::new();
        Frame::new(b"test").write(&mut buf);
        assert!(Frame::available(&buf));
    }

    #[test]
    fn available_should_return_false_when_no_frame_present() {
        let buf = BytesMut::new();
        assert!(!Frame::available(&buf));
    }

    #[test]
    fn available_should_return_false_when_partial_frame_present() {
        let mut buf = BytesMut::new();
        // Write header indicating 100 bytes but only provide 5 bytes of data
        buf.put_u64(100);
        buf.put_bytes(0, 5);
        assert!(!Frame::available(&buf));
    }

    #[test]
    fn available_should_not_consume_the_frame() {
        let mut buf = BytesMut::new();
        Frame::new(b"test").write(&mut buf);

        let len_before = buf.len();
        assert!(Frame::available(&buf));
        assert_eq!(buf.len(), len_before, "available() should not consume src");
    }

    // --- Tests for From impls ---

    #[test]
    fn from_byte_slice_should_create_borrowed_frame() {
        let data: &[u8] = b"slice";
        let frame = Frame::from(data);
        assert_eq!(frame.as_item(), b"slice");
    }

    #[test]
    fn from_byte_array_ref_should_create_borrowed_frame() {
        let data: &[u8; 3] = b"abc";
        let frame = Frame::from(data);
        assert_eq!(frame.as_item(), b"abc");
    }

    #[test]
    fn from_byte_array_should_create_owned_frame() {
        let frame = OwnedFrame::from([1u8, 2, 3]);
        assert_eq!(frame.as_item(), &[1, 2, 3]);
    }

    #[test]
    fn from_vec_should_create_owned_frame() {
        let frame = OwnedFrame::from(vec![4, 5, 6]);
        assert_eq!(frame.as_item(), &[4, 5, 6]);
    }

    #[test]
    fn from_empty_vec_should_create_empty_owned_frame() {
        let frame = OwnedFrame::from(vec![]);
        assert!(frame.is_empty());
    }

    // --- Tests for AsRef ---

    #[test]
    fn as_ref_should_return_item_bytes() {
        let frame = Frame::new(b"ref test");
        let bytes: &[u8] = frame.as_ref();
        assert_eq!(bytes, b"ref test");
    }

    // --- Tests for Extend ---

    #[test]
    fn extend_should_append_bytes_to_borrowed_frame() {
        let data = b"hello";
        let mut frame = Frame::new(data);
        frame.extend(b", world".iter().copied());
        assert_eq!(frame.as_item(), b"hello, world");
    }

    #[test]
    fn extend_should_append_bytes_to_owned_frame() {
        let mut frame = OwnedFrame::from(vec![1, 2, 3]);
        frame.extend(vec![4, 5]);
        assert_eq!(frame.as_item(), &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn extend_empty_iterator_should_not_change_frame() {
        let mut frame = OwnedFrame::from(vec![1, 2, 3]);
        frame.extend(std::iter::empty::<u8>());
        assert_eq!(frame.as_item(), &[1, 2, 3]);
    }

    // --- Tests for PartialEq ---

    #[test]
    fn partial_eq_with_byte_slice_should_compare_item() {
        let frame = Frame::new(b"abc");
        assert_eq!(frame, *b"abc");
    }

    #[test]
    fn partial_eq_with_byte_slice_ref() {
        let frame = Frame::new(b"abc");
        let slice: &[u8] = b"abc";
        assert_eq!(frame, slice);
    }

    #[test]
    fn partial_eq_with_byte_array() {
        let frame = Frame::new(b"abc");
        assert_eq!(frame, [b'a', b'b', b'c']);
    }

    #[test]
    fn partial_eq_with_byte_array_ref() {
        let frame = Frame::new(b"abc");
        assert_eq!(frame, b"abc");
    }

    #[test]
    fn partial_eq_should_return_false_for_different_content() {
        let frame = Frame::new(b"abc");
        assert_ne!(frame, *b"xyz");
    }

    // --- Tests for HEADER_SIZE ---

    #[test]
    fn header_size_should_be_8_bytes() {
        assert_eq!(Frame::HEADER_SIZE, 8);
    }

    // --- Tests for write/read round-trip ---

    #[test]
    fn write_then_read_should_round_trip_arbitrary_data() {
        let data = (0u8..=255).collect::<Vec<u8>>();
        let frame = Frame::new(&data);

        let mut buf = BytesMut::new();
        frame.write(&mut buf);

        let recovered = Frame::read(&mut buf).expect("missing frame");
        assert_eq!(recovered.as_item(), data.as_slice());
    }

    #[test]
    fn write_then_read_should_round_trip_empty_frame() {
        let frame = Frame::empty();
        let mut buf = BytesMut::new();
        frame.write(&mut buf);
        // We need at least one extra byte beyond the header for read to proceed
        buf.put_u8(0xFF);

        let recovered = Frame::read(&mut buf).expect("missing frame");
        assert!(recovered.is_empty());
        // The extra byte should remain
        assert_eq!(buf.as_ref(), &[0xFF]);
    }

    #[test]
    fn multiple_frames_should_be_readable_sequentially() {
        let mut buf = BytesMut::new();
        Frame::new(b"first").write(&mut buf);
        Frame::new(b"second").write(&mut buf);

        let f1 = Frame::read(&mut buf).expect("missing first frame");
        assert_eq!(f1, b"first");

        let f2 = Frame::read(&mut buf).expect("missing second frame");
        assert_eq!(f2, b"second");

        assert!(Frame::read(&mut buf).is_none());
    }

    // --- Tests for clone and equality ---

    #[test]
    fn clone_should_produce_equal_frame() {
        let frame = OwnedFrame::from(vec![1, 2, 3]);
        let cloned = frame.clone();
        assert_eq!(frame, cloned);
    }

    #[test]
    fn frames_with_same_content_but_different_ownership_should_be_equal() {
        let data = b"same";
        let borrowed = Frame::new(data);
        let owned = OwnedFrame::from(data.to_vec());
        assert_eq!(borrowed, owned);
    }
}
