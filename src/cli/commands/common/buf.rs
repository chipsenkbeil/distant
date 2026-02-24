use std::ops::{Deref, DerefMut};

/// Wraps a string to provide some friendly read and write methods
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StringBuf(String);

impl StringBuf {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consumes data within the buffer that represent full lines (end with a newline) and returns
    /// the string containing those lines.
    ///
    /// The remaining buffer contains are returned as the second part of the tuple
    pub fn into_full_lines(mut self) -> (Option<String>, StringBuf) {
        match self.rfind('\n') {
            Some(idx) => {
                let remaining = self.0.split_off(idx + 1);
                (Some(self.0), Self(remaining))
            }
            None => (None, self),
        }
    }
}

impl From<String> for StringBuf {
    fn from(x: String) -> Self {
        Self(x)
    }
}

impl From<StringBuf> for String {
    fn from(x: StringBuf) -> Self {
        x.0
    }
}

impl Deref for StringBuf {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for StringBuf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `StringBuf`: construction, `into_full_lines()` splitting,
    //! `Deref`/`DerefMut`, `From`, `PartialEq`, `Hash`, and `Clone`.

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // StringBuf::new
    // -------------------------------------------------------
    #[test]
    fn new_creates_empty_buffer() {
        let buf = StringBuf::new();
        assert!(buf.is_empty());
        assert_eq!(&*buf, "");
    }

    // -------------------------------------------------------
    // Default
    // -------------------------------------------------------
    #[test]
    fn default_creates_empty_buffer() {
        let buf = StringBuf::default();
        assert!(buf.is_empty());
    }

    // -------------------------------------------------------
    // From<String> / From<StringBuf> for String
    // -------------------------------------------------------
    #[test]
    fn from_string() {
        let buf = StringBuf::from(String::from("hello"));
        assert_eq!(&*buf, "hello");
    }

    #[test]
    fn into_string() {
        let buf = StringBuf::from(String::from("hello"));
        let s: String = buf.into();
        assert_eq!(s, "hello");
    }

    // -------------------------------------------------------
    // Deref / DerefMut
    // -------------------------------------------------------
    #[test]
    fn deref_gives_string_ref() {
        let buf = StringBuf::from(String::from("test"));
        // Can call String methods through Deref
        assert_eq!(buf.len(), 4);
        assert!(buf.contains("es"));
    }

    #[test]
    fn deref_mut_allows_modification() {
        let mut buf = StringBuf::new();
        buf.push_str("hello ");
        buf.push_str("world");
        assert_eq!(&*buf, "hello world");
    }

    // -------------------------------------------------------
    // into_full_lines – no newline
    // -------------------------------------------------------
    #[test]
    fn into_full_lines_no_newline_returns_none_and_preserves_buffer() {
        let buf = StringBuf::from(String::from("partial data"));
        let (lines, remaining) = buf.into_full_lines();
        assert!(lines.is_none());
        assert_eq!(&*remaining, "partial data");
    }

    // -------------------------------------------------------
    // into_full_lines – single complete line
    // -------------------------------------------------------
    #[test]
    fn into_full_lines_single_complete_line() {
        let buf = StringBuf::from(String::from("hello\n"));
        let (lines, remaining) = buf.into_full_lines();
        assert_eq!(lines.unwrap(), "hello\n");
        assert_eq!(&*remaining, "");
    }

    // -------------------------------------------------------
    // into_full_lines – multiple lines with trailing partial
    // -------------------------------------------------------
    #[test]
    fn into_full_lines_multiple_lines_with_partial() {
        let buf = StringBuf::from(String::from("line1\nline2\npartial"));
        let (lines, remaining) = buf.into_full_lines();
        assert_eq!(lines.unwrap(), "line1\nline2\n");
        assert_eq!(&*remaining, "partial");
    }

    // -------------------------------------------------------
    // into_full_lines – multiple complete lines, no trailing partial
    // -------------------------------------------------------
    #[test]
    fn into_full_lines_multiple_complete_lines() {
        let buf = StringBuf::from(String::from("line1\nline2\n"));
        let (lines, remaining) = buf.into_full_lines();
        assert_eq!(lines.unwrap(), "line1\nline2\n");
        assert_eq!(&*remaining, "");
    }

    // -------------------------------------------------------
    // into_full_lines – empty buffer
    // -------------------------------------------------------
    #[test]
    fn into_full_lines_empty_buffer() {
        let buf = StringBuf::new();
        let (lines, remaining) = buf.into_full_lines();
        assert!(lines.is_none());
        assert!(remaining.is_empty());
    }

    // -------------------------------------------------------
    // into_full_lines – only newline
    // -------------------------------------------------------
    #[test]
    fn into_full_lines_only_newline() {
        let buf = StringBuf::from(String::from("\n"));
        let (lines, remaining) = buf.into_full_lines();
        assert_eq!(lines.unwrap(), "\n");
        assert_eq!(&*remaining, "");
    }

    // -------------------------------------------------------
    // into_full_lines – newline at beginning with trailing text
    // -------------------------------------------------------
    #[test]
    fn into_full_lines_newline_at_start_with_trailing() {
        let buf = StringBuf::from(String::from("\ntrailing"));
        let (lines, remaining) = buf.into_full_lines();
        assert_eq!(lines.unwrap(), "\n");
        assert_eq!(&*remaining, "trailing");
    }

    // -------------------------------------------------------
    // PartialEq / Hash
    // -------------------------------------------------------
    #[test]
    fn equality() {
        let a = StringBuf::from(String::from("abc"));
        let b = StringBuf::from(String::from("abc"));
        let c = StringBuf::from(String::from("xyz"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn hash_consistency() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(StringBuf::from(String::from("abc")));
        set.insert(StringBuf::from(String::from("abc")));
        assert_eq!(set.len(), 1);
    }

    // -------------------------------------------------------
    // Clone
    // -------------------------------------------------------
    #[test]
    fn clone_is_independent() {
        let mut a = StringBuf::from(String::from("hello"));
        let b = a.clone();
        a.push_str(" world");
        assert_eq!(&*a, "hello world");
        assert_eq!(&*b, "hello");
    }
}
