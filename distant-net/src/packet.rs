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

/// Parse msgpack str, returning remaining bytes and str on success, or error on failure
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
