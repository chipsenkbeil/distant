use bytes::{Buf, BufMut, BytesMut};
use std::{borrow::Cow, io};

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
    /// Creates a new frame wrapping the `item` that will be shipped across the network
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
    /// Total bytes to use as the header field denoting a frame's size
    pub const HEADER_SIZE: usize = 8;

    /// Returns the len (in bytes) of the item wrapped by the frame
    pub fn len(&self) -> usize {
        self.item.len()
    }

    /// Returns true if the frame is comprised of zero bytes
    pub fn is_empty(&self) -> bool {
        self.item.is_empty()
    }

    /// Returns a reference to the bytes of the frame's item
    pub fn as_item(&self) -> &[u8] {
        &self.item
    }

    /// Writes the frame to the end of `dst`, including the header representing the length of the
    /// item as part of the written bytes
    pub fn write(&self, dst: &mut BytesMut) -> io::Result<()> {
        if self.item.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Empty item provided",
            ));
        }

        dst.reserve(Self::HEADER_SIZE + self.item.len());

        // Add data in form of {LEN}{ITEM}
        dst.put_u64((self.item.len()) as u64);
        dst.put_slice(&self.item);

        Ok(())
    }

    /// Attempts to read a frame from `src`, returning `Some(Frame)` if a frame was found
    /// (including the header) or `None` if the current `src` does not contain a frame
    pub fn read(src: &mut BytesMut) -> io::Result<Option<OwnedFrame>> {
        // First, check if we have more data than just our frame's message length
        if src.len() <= Self::HEADER_SIZE {
            return Ok(None);
        }

        // Second, retrieve total size of our frame's message
        let item_len = u64::from_be_bytes(src[..Self::HEADER_SIZE].try_into().unwrap()) as usize;

        // In the case that our item len is 0, we skip over the invalid frame
        if item_len == 0 {
            // Ensure we advance to remove the frame
            src.advance(Self::HEADER_SIZE);

            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Frame's msg cannot have length of 0",
            ));
        }

        // Third, check if we have all data for our frame; if not, exit early
        if src.len() < item_len + Self::HEADER_SIZE {
            return Ok(None);
        }

        // Fourth, get and return our item
        let item = src[Self::HEADER_SIZE..(Self::HEADER_SIZE + item_len)].to_vec();

        // Fifth, advance so frame is no longer kept around
        src.advance(Self::HEADER_SIZE + item_len);

        Ok(Some(Frame::from(item)))
    }

    /// Checks if a full frame is available from `src`, returning true if a frame was found false
    /// if the current `src` does not contain a frame. Does not consume the frame.
    pub fn available(src: &BytesMut) -> bool {
        matches!(Frame::read(&mut src.clone()), Ok(Some(_)))
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
    fn from(item: &'a [u8]) -> Self {
        Self {
            item: Cow::Borrowed(item),
        }
    }
}

impl<'a, const N: usize> From<&'a [u8; N]> for Frame<'a> {
    fn from(item: &'a [u8; N]) -> Self {
        Self {
            item: Cow::Borrowed(item),
        }
    }
}

impl<const N: usize> From<[u8; N]> for OwnedFrame {
    fn from(item: [u8; N]) -> Self {
        Self {
            item: Cow::Owned(item.to_vec()),
        }
    }
}

impl From<Vec<u8>> for OwnedFrame {
    fn from(item: Vec<u8>) -> Self {
        Self {
            item: Cow::Owned(item),
        }
    }
}

impl AsRef<[u8]> for Frame<'_> {
    fn as_ref(&self) -> &[u8] {
        AsRef::as_ref(&self.item)
    }
}

impl Extend<u8> for Frame<'_> {
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
    fn eq(&self, item: &[u8]) -> bool {
        self.item.as_ref().eq(item)
    }
}

impl<'a> PartialEq<&'a [u8]> for Frame<'_> {
    fn eq(&self, item: &&'a [u8]) -> bool {
        self.item.as_ref().eq(*item)
    }
}

impl<const N: usize> PartialEq<[u8; N]> for Frame<'_> {
    fn eq(&self, item: &[u8; N]) -> bool {
        self.item.as_ref().eq(item)
    }
}

impl<'a, const N: usize> PartialEq<&'a [u8; N]> for Frame<'_> {
    fn eq(&self, item: &&'a [u8; N]) -> bool {
        self.item.as_ref().eq(*item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    #[test]
    fn write_should_fail_when_item_is_zero_bytes() {
        let frame = Frame::new(&[]);

        let mut buf = BytesMut::new();
        let result = frame.write(&mut buf);

        match result {
            Err(x) if x.kind() == io::ErrorKind::InvalidInput => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test]
    fn write_should_build_a_frame_containing_a_length_and_item() {
        let frame = Frame::new(b"hello, world");

        let mut buf = BytesMut::new();
        frame.write(&mut buf).expect("Failed to write");

        let len = buf.get_u64() as usize;
        assert_eq!(len, 12, "Wrong length writed");
        assert_eq!(buf.as_ref(), b"hello, world");
    }

    #[test]
    fn read_should_return_none_if_data_smaller_than_or_equal_to_item_length_field() {
        let mut buf = BytesMut::new();
        buf.put_bytes(0, Frame::HEADER_SIZE);

        let result = Frame::read(&mut buf);
        assert!(
            matches!(result, Ok(None)),
            "Unexpected result: {:?}",
            result
        );
    }

    #[test]
    fn read_should_return_none_if_not_enough_data_for_frame() {
        let mut buf = BytesMut::new();
        buf.put_u64(0);

        let result = Frame::read(&mut buf);
        assert!(
            matches!(result, Ok(None)),
            "Unexpected result: {:?}",
            result
        );
    }

    #[test]
    fn read_should_fail_if_writed_item_length_is_zero() {
        let mut buf = BytesMut::new();
        buf.put_u64(0);
        buf.put_u8(255);

        let result = Frame::read(&mut buf);
        match result {
            Err(x) if x.kind() == io::ErrorKind::InvalidData => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test]
    fn read_should_advance_src_by_frame_size_even_if_item_length_is_zero() {
        let mut buf = BytesMut::new();
        buf.put_u64(0);
        buf.put_bytes(0, 3);

        assert!(
            Frame::read(&mut buf).is_err(),
            "read unexpectedly succeeded"
        );
        assert_eq!(buf.len(), 3, "Advanced an unexpected amount in src buf");
    }

    #[test]
    fn read_should_advance_src_by_frame_size_when_successful() {
        // Add 3 extra bytes after a full frame
        let mut buf = BytesMut::new();
        Frame::new(b"hello, world")
            .write(&mut buf)
            .expect("Failed to write");
        buf.put_bytes(0, 3);

        assert!(Frame::read(&mut buf).is_ok(), "read unexpectedly failed");
        assert_eq!(buf.len(), 3, "Advanced an unexpected amount in src buf");
    }

    #[test]
    fn read_should_return_some_byte_vec_when_successful() {
        let mut buf = BytesMut::new();
        Frame::new(b"hello, world")
            .write(&mut buf)
            .expect("Failed to write");

        let item = Frame::read(&mut buf)
            .expect("Failed to read")
            .expect("Item not properly captured");
        assert_eq!(item, b"hello, world");
    }
}
