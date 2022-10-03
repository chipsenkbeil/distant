/// Represents a generic id type
pub type Id = String;

mod request;
mod response;

pub use request::*;
pub use response::*;

#[derive(Clone, Debug, PartialEq, Eq)]
enum MsgPackStrParseError {
    InvalidFormat,
    Utf8Error(std::str::Utf8Error),
}

/// Writes the given str to the end of `buf` as the str's msgpack representation.
///
/// # Panics
///
/// Panics if `s.len() >= 2 ^ 32` as the maximum str length for a msgpack str is `(2 ^ 32) - 1`.
fn write_str_msg_pack(s: &str, buf: &mut Vec<u8>) {
    assert!(
        s.len() < 2usize.pow(32),
        "str cannot be longer than (2^32)-1 bytes"
    );

    if s.len() < 32 {
        buf.push(s.len() as u8 | 0b10100000);
    } else if s.len() < 2usize.pow(8) {
        buf.push(0xd9);
        buf.push(s.len() as u8);
    } else if s.len() < 2usize.pow(16) {
        buf.push(0xda);
        for b in (s.len() as u16).to_be_bytes() {
            buf.push(b);
        }
    } else {
        buf.push(0xdb);
        for b in (s.len() as u32).to_be_bytes() {
            buf.push(b);
        }
    }

    buf.extend_from_slice(s.as_bytes());
}

/// Parse msgpack str, returning remaining bytes and str on success, or error on failure.
fn parse_msg_pack_str(input: &[u8]) -> Result<(&[u8], &str), MsgPackStrParseError> {
    let ilen = input.len();
    if ilen == 0 {
        return Err(MsgPackStrParseError::InvalidFormat);
    }

    // * fixstr using 0xa0 - 0xbf to mark the start of the str where < 32 bytes
    // * str 8 (0xd9) if up to (2^8)-1 bytes, using next byte for len
    // * str 16 (0xda) if up to (2^16)-1 bytes, using next two bytes for len
    // * str 32 (0xdb)  if up to (2^32)-1 bytes, using next four bytes for len
    let (input, len): (&[u8], usize) = if input[0] >= 0xa0 && input[0] <= 0xbf {
        (&input[1..], (input[0] & 0b00011111).into())
    } else if input[0] == 0xd9 && ilen > 2 {
        (&input[2..], input[1].into())
    } else if input[0] == 0xda && ilen > 3 {
        (&input[3..], u16::from_be_bytes([input[1], input[2]]).into())
    } else if input[0] == 0xdb && ilen > 5 {
        (
            &input[5..],
            u32::from_be_bytes([input[1], input[2], input[3], input[4]])
                .try_into()
                .unwrap(),
        )
    } else {
        return Err(MsgPackStrParseError::InvalidFormat);
    };

    let s = match std::str::from_utf8(&input[..len]) {
        Ok(s) => s,
        Err(x) => return Err(MsgPackStrParseError::Utf8Error(x)),
    };

    Ok((&input[len..], s))
}

#[cfg(test)]
mod tests {
    use super::*;

    mod write_str_msg_pack {
        use super::*;

        #[test]
        fn should_support_fixstr() {
            // 0-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("", &mut buf);
            assert_eq!(buf, &[0xa0]);

            // 1-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("a", &mut buf);
            assert_eq!(buf, &[0xa1, b'a']);

            // 2-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("ab", &mut buf);
            assert_eq!(buf, &[0xa2, b'a', b'b']);

            // 3-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abc", &mut buf);
            assert_eq!(buf, &[0xa3, b'a', b'b', b'c']);

            // 4-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcd", &mut buf);
            assert_eq!(buf, &[0xa4, b'a', b'b', b'c', b'd']);

            // 5-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcde", &mut buf);
            assert_eq!(buf, &[0xa5, b'a', b'b', b'c', b'd', b'e']);

            // 6-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdef", &mut buf);
            assert_eq!(buf, &[0xa6, b'a', b'b', b'c', b'd', b'e', b'f']);

            // 7-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefg", &mut buf);
            assert_eq!(buf, &[0xa7, b'a', b'b', b'c', b'd', b'e', b'f', b'g']);

            // 8-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefgh", &mut buf);
            assert_eq!(buf, &[0xa8, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h']);

            // 9-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghi", &mut buf);
            assert_eq!(
                buf,
                &[0xa9, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i']
            );

            // 10-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghij", &mut buf);
            assert_eq!(
                buf,
                &[0xaa, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j']
            );

            // 11-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijk", &mut buf);
            assert_eq!(
                buf,
                &[0xab, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k']
            );

            // 12-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijkl", &mut buf);
            assert_eq!(
                buf,
                &[0xac, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l']
            );

            // 13-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklm", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xad, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm'
                ]
            );

            // 14-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmn", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xae, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n'
                ]
            );

            // 15-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmno", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xaf, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o'
                ]
            );

            // 16-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnop", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xb0, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p'
                ]
            );

            // 17-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopq", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xb1, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q'
                ]
            );

            // 18-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqr", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xb2, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r'
                ]
            );

            // 19-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrs", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xb3, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's'
                ]
            );

            // 20-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrst", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xb4, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't'
                ]
            );

            // 21-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstu", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xb5, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u'
                ]
            );

            // 22-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstuv", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xb6, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v'
                ]
            );

            // 23-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstuvw", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xb7, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w'
                ]
            );

            // 24-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstuvwx", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xb8, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x'
                ]
            );

            // 25-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstuvwxy", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xb9, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y'
                ]
            );

            // 26-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstuvwxyz", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xba, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y',
                    b'z'
                ]
            );

            // 27-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstuvwxyz0", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xbb, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y',
                    b'z', b'0'
                ]
            );

            // 28-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstuvwxyz01", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xbc, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y',
                    b'z', b'0', b'1'
                ]
            );

            // 29-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstuvwxyz012", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xbd, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y',
                    b'z', b'0', b'1', b'2'
                ]
            );

            // 30-byte str
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstuvwxyz0123", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xbe, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y',
                    b'z', b'0', b'1', b'2', b'3'
                ]
            );

            // 31-byte str is maximum len of fixstr
            let mut buf = Vec::new();
            write_str_msg_pack("abcdefghijklmnopqrstuvwxyz01234", &mut buf);
            assert_eq!(
                buf,
                &[
                    0xbf, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
                    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y',
                    b'z', b'0', b'1', b'2', b'3', b'4'
                ]
            );
        }

        #[test]
        fn should_support_str_8() {
            let input = "a".repeat(32);
            let mut buf = Vec::new();
            write_str_msg_pack(&input, &mut buf);
            assert_eq!(buf[0], 0xd9);
            assert_eq!(buf[1], input.len() as u8);
            assert_eq!(&buf[2..], input.as_bytes());

            let input = "a".repeat(2usize.pow(8) - 1);
            let mut buf = Vec::new();
            write_str_msg_pack(&input, &mut buf);
            assert_eq!(buf[0], 0xd9);
            assert_eq!(buf[1], input.len() as u8);
            assert_eq!(&buf[2..], input.as_bytes());
        }

        #[test]
        fn should_support_str_16() {
            let input = "a".repeat(2usize.pow(8));
            let mut buf = Vec::new();
            write_str_msg_pack(&input, &mut buf);
            assert_eq!(buf[0], 0xda);
            assert_eq!(&buf[1..3], &(input.len() as u16).to_be_bytes());
            assert_eq!(&buf[3..], input.as_bytes());

            let input = "a".repeat(2usize.pow(16) - 1);
            let mut buf = Vec::new();
            write_str_msg_pack(&input, &mut buf);
            assert_eq!(buf[0], 0xda);
            assert_eq!(&buf[1..3], &(input.len() as u16).to_be_bytes());
            assert_eq!(&buf[3..], input.as_bytes());
        }

        #[test]
        fn should_support_str_32() {
            let input = "a".repeat(2usize.pow(16));
            let mut buf = Vec::new();
            write_str_msg_pack(&input, &mut buf);
            assert_eq!(buf[0], 0xdb);
            assert_eq!(&buf[1..5], &(input.len() as u32).to_be_bytes());
            assert_eq!(&buf[5..], input.as_bytes());
        }
    }

    mod parse_msg_pack_str {
        use super::*;

        #[test]
        fn should_be_able_to_parse_fixstr() {
            // Empty str
            let (input, s) = parse_msg_pack_str(&[0xa0]).unwrap();
            assert!(input.is_empty());
            assert_eq!(s, "");

            // Single character
            let (input, s) = parse_msg_pack_str(&[0xa1, b'a']).unwrap();
            assert!(input.is_empty());
            assert_eq!(s, "a");

            // 31 byte str
            let (input, s) = parse_msg_pack_str(&[
                0xbf, b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a',
                b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a',
                b'a', b'a', b'a', b'a',
            ])
            .unwrap();
            assert!(input.is_empty());
            assert_eq!(s, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

            // Verify that we only consume up to fixstr length
            assert_eq!(parse_msg_pack_str(&[0xa0, b'a']).unwrap().0, b"a");
            assert_eq!(
                parse_msg_pack_str(&[
                    0xbf, b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a',
                    b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a',
                    b'a', b'a', b'a', b'a', b'a', b'a', b'b'
                ])
                .unwrap()
                .0,
                b"b"
            );
        }

        #[test]
        fn should_be_able_to_parse_str_8() {
            // 32 byte str
            let (input, s) = parse_msg_pack_str(&[
                0xd9, 32, b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a',
                b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'a',
                b'a', b'a', b'a', b'a', b'a', b'a',
            ])
            .unwrap();
            assert!(input.is_empty());
            assert_eq!(s, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

            // 2^8 - 1 (255) byte str
            let test_str = "a".repeat(2usize.pow(8) - 1);
            let mut input = vec![0xd9, 255];
            input.extend_from_slice(test_str.as_bytes());
            let (input, s) = parse_msg_pack_str(&input).unwrap();
            assert!(input.is_empty());
            assert_eq!(s, test_str);

            // Verify that we only consume up to 2^8 - 1 length
            let mut input = vec![0xd9, 255];
            input.extend_from_slice(test_str.as_bytes());
            input.extend_from_slice(b"hello");
            let (input, s) = parse_msg_pack_str(&input).unwrap();
            assert_eq!(input, b"hello");
            assert_eq!(s, test_str);
        }

        #[test]
        fn should_be_able_to_parse_str_16() {
            // 2^8 byte str (256)
            let test_str = "a".repeat(2usize.pow(8));
            let mut input = vec![0xda, 1, 0];
            input.extend_from_slice(test_str.as_bytes());
            let (input, s) = parse_msg_pack_str(&input).unwrap();
            assert!(input.is_empty());
            assert_eq!(s, test_str);

            // 2^16 - 1 (65535) byte str
            let test_str = "a".repeat(2usize.pow(16) - 1);
            let mut input = vec![0xda, 255, 255];
            input.extend_from_slice(test_str.as_bytes());
            let (input, s) = parse_msg_pack_str(&input).unwrap();
            assert!(input.is_empty());
            assert_eq!(s, test_str);

            // Verify that we only consume up to 2^16 - 1 length
            let mut input = vec![0xda, 255, 255];
            input.extend_from_slice(test_str.as_bytes());
            input.extend_from_slice(b"hello");
            let (input, s) = parse_msg_pack_str(&input).unwrap();
            assert_eq!(input, b"hello");
            assert_eq!(s, test_str);
        }

        #[test]
        fn should_be_able_to_parse_str_32() {
            // 2^16 byte str
            let test_str = "a".repeat(2usize.pow(16));
            let mut input = vec![0xdb, 0, 1, 0, 0];
            input.extend_from_slice(test_str.as_bytes());
            let (input, s) = parse_msg_pack_str(&input).unwrap();
            assert!(input.is_empty());
            assert_eq!(s, test_str);

            // NOTE: We are not going to run the below tests, not because they aren't valid but
            // because this generates a 4GB str which takes 20+ seconds to run

            // 2^32 - 1 byte str (4294967295 bytes)
            /* let test_str = "a".repeat(2usize.pow(32) - 1);
            let mut input = vec![0xdb, 255, 255, 255, 255];
            input.extend_from_slice(test_str.as_bytes());
            let (input, s) = parse_msg_pack_str(&input).unwrap();
            assert!(input.is_empty());
            assert_eq!(s, test_str); */

            // Verify that we only consume up to 2^32 - 1 length
            /* let mut input = vec![0xdb, 255, 255, 255, 255];
            input.extend_from_slice(test_str.as_bytes());
            input.extend_from_slice(b"hello");
            let (input, s) = parse_msg_pack_str(&input).unwrap();
            assert_eq!(input, b"hello");
            assert_eq!(s, test_str); */
        }

        #[test]
        fn should_fail_parsing_str_with_invalid_length() {
            // Make sure that parse doesn't fail looking for bytes after str 8 len
            assert_eq!(
                parse_msg_pack_str(&[0xd9]),
                Err(MsgPackStrParseError::InvalidFormat)
            );
            assert_eq!(
                parse_msg_pack_str(&[0xd9, 0]),
                Err(MsgPackStrParseError::InvalidFormat)
            );

            // Make sure that parse doesn't fail looking for bytes after str 16 len
            assert_eq!(
                parse_msg_pack_str(&[0xda]),
                Err(MsgPackStrParseError::InvalidFormat)
            );
            assert_eq!(
                parse_msg_pack_str(&[0xda, 0]),
                Err(MsgPackStrParseError::InvalidFormat)
            );
            assert_eq!(
                parse_msg_pack_str(&[0xda, 0, 0]),
                Err(MsgPackStrParseError::InvalidFormat)
            );

            // Make sure that parse doesn't fail looking for bytes after str 32 len
            assert_eq!(
                parse_msg_pack_str(&[0xdb]),
                Err(MsgPackStrParseError::InvalidFormat)
            );
            assert_eq!(
                parse_msg_pack_str(&[0xdb, 0]),
                Err(MsgPackStrParseError::InvalidFormat)
            );
            assert_eq!(
                parse_msg_pack_str(&[0xdb, 0, 0]),
                Err(MsgPackStrParseError::InvalidFormat)
            );
            assert_eq!(
                parse_msg_pack_str(&[0xdb, 0, 0, 0]),
                Err(MsgPackStrParseError::InvalidFormat)
            );
            assert_eq!(
                parse_msg_pack_str(&[0xdb, 0, 0, 0, 0]),
                Err(MsgPackStrParseError::InvalidFormat)
            );
        }

        #[test]
        fn should_fail_parsing_other_types() {
            assert_eq!(
                parse_msg_pack_str(&[0xc3]), // Boolean (true)
                Err(MsgPackStrParseError::InvalidFormat)
            );
        }

        #[test]
        fn should_fail_if_empty_input() {
            assert_eq!(
                parse_msg_pack_str(&[]),
                Err(MsgPackStrParseError::InvalidFormat)
            );
        }

        #[test]
        fn should_fail_if_str_is_not_utf8() {
            assert!(matches!(
                parse_msg_pack_str(&[0xa4, 0, 159, 146, 150]),
                Err(MsgPackStrParseError::Utf8Error(_))
            ));
        }
    }
}
