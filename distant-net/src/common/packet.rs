mod header;
mod request;
mod response;
mod value;

pub use header::*;
pub use request::*;
pub use response::*;
pub use value::*;

use std::io::Cursor;

/// Represents a generic id type
pub type Id = String;

/// Reads the header bytes from msgpack input, including the marker and len bytes.
///
/// * If succeeds, returns (header, remaining).
/// * If fails, returns existing bytes.
fn read_header_bytes(input: &[u8]) -> Result<(&[u8], &[u8]), &[u8]> {
    let mut cursor = Cursor::new(input);
    let input_len = input.len();

    // Determine size of header map in terms of total objects
    let len = match rmp::decode::read_map_len(&mut cursor) {
        Ok(x) => x,
        Err(_) => return Err(input),
    };

    // For each object, we have a corresponding key in front of it has a string,
    // so we need to iterate, advancing by a string key and then the object
    for _i in 0..len {
        // Read just the length of the key to avoid copying the key itself
        let key_len = match rmp::decode::read_str_len(&mut cursor) {
            Ok(x) => x as u64,
            Err(_) => return Err(input),
        };

        // Advance forward past the key
        cursor.set_position(cursor.position() + key_len);

        // If we would have advanced past our input, fail
        if cursor.position() as usize > input_len {
            return Err(input);
        }

        // Point locally to just past the str key so we can determine next byte len to skip
        let input = &input[cursor.position() as usize..];

        // Read the type of object and advance accordingly
        match find_msgpack_byte_len(input) {
            Some(len) => cursor.set_position(cursor.position() + len),
            None => return Err(input),
        }

        // If we would have advanced past our input, fail
        if cursor.position() as usize > input_len {
            return Err(input);
        }
    }

    let pos = cursor.position() as usize;

    // Check if we've read beyond the input (being equal to len is okay
    // because we could consume all of the remaining input this way)
    if pos > input_len {
        return Err(input);
    }

    Ok((&input[..pos], &input[pos..]))
}

/// Determines the length of the next object based on its marker. From the marker, some objects
/// need to be traversed (e.g. map) in order to fully understand the total byte length.
///
/// This will include the marker bytes in the total byte len such that collecting all of the
/// bytes up to len will yield a valid msgpack object in byte form.
///
/// If the first byte does not signify a valid marker, this method returns None.
fn find_msgpack_byte_len(input: &[u8]) -> Option<u64> {
    if input.is_empty() {
        return None;
    }

    macro_rules! read_len {
        (u8: $input:expr $(, start = $start:expr)?) => {{
            let input = $input;

            $(
                if input.len() < $start {
                    return None;
                }
                let input = &input[$start..];
            )?

            if input.is_empty() {
                return None;
            } else {
                input[0] as u64
            }
        }};
        (u16: $input:expr $(, start = $start:expr)?) => {{
            let input = $input;

            $(
                if input.len() < $start {
                    return None;
                }
                let input = &input[$start..];
            )?

            if input.len() < 2 {
                return None;
            } else {
                u16::from_be_bytes([input[0], input[1]]) as u64
            }
        }};
        (u32: $input:expr $(, start = $start:expr)?) => {{
            let input = $input;

            $(
                if input.len() < $start {
                    return None;
                }
                let input = &input[$start..];
            )?

            if input.len() < 4 {
                return None;
            } else {
                u32::from_be_bytes([input[0], input[1], input[2], input[3]]) as u64
            }
        }};
        ($cnt:expr => $input:expr $(, start = $start:expr)?) => {{
            let input = $input;

            $(
                if input.len() < $start {
                    return None;
                }
                let input = &input[$start..];
            )?

            let cnt = $cnt;
            let mut len = 0;
            for _i in 0..cnt {
                if input.len() < len {
                    return None;
                }

                let input = &input[len..];
                match find_msgpack_byte_len(input) {
                    Some(x) => len += x as usize,
                    None => return None,
                }
            }
            len as u64
        }};
    }

    Some(match rmp::Marker::from_u8(input[0]) {
        // Booleans and nil (aka null) are a combination of marker and value (single byte)
        rmp::Marker::Null => 1,
        rmp::Marker::True => 1,
        rmp::Marker::False => 1,

        // Integers are stored in 1, 2, 3, 5, or 9 bytes
        rmp::Marker::FixPos(_) => 1,
        rmp::Marker::FixNeg(_) => 1,
        rmp::Marker::U8 => 2,
        rmp::Marker::U16 => 3,
        rmp::Marker::U32 => 5,
        rmp::Marker::U64 => 9,
        rmp::Marker::I8 => 2,
        rmp::Marker::I16 => 3,
        rmp::Marker::I32 => 5,
        rmp::Marker::I64 => 9,

        // Floats are stored in 5 or 9 bytes
        rmp::Marker::F32 => 5,
        rmp::Marker::F64 => 9,

        // Str are stored in 1, 2, 3, or 5 bytes + the data buffer
        rmp::Marker::FixStr(len) => 1 + len as u64,
        rmp::Marker::Str8 => 2 + read_len!(u8: input, start = 1),
        rmp::Marker::Str16 => 3 + read_len!(u16: input, start = 1),
        rmp::Marker::Str32 => 5 + read_len!(u32: input, start = 1),

        // Bin are stored in 2, 3, or 5 bytes + the data buffer
        rmp::Marker::Bin8 => 2 + read_len!(u8: input, start = 1),
        rmp::Marker::Bin16 => 3 + read_len!(u16: input, start = 1),
        rmp::Marker::Bin32 => 5 + read_len!(u32: input, start = 1),

        // Arrays are stored in 1, 3, or 5 bytes + N objects (where each object has its own len)
        rmp::Marker::FixArray(cnt) => 1 + read_len!(cnt => input, start = 1),
        rmp::Marker::Array16 => {
            let cnt = read_len!(u16: input, start = 1);
            3 + read_len!(cnt => input, start = 3)
        }
        rmp::Marker::Array32 => {
            let cnt = read_len!(u32: input, start = 1);
            5 + read_len!(cnt => input, start = 5)
        }

        // Maps are stored in 1, 3, or 5 bytes + 2*N objects (where each object has its own len)
        rmp::Marker::FixMap(cnt) => 1 + read_len!(2 * cnt => input, start = 1),
        rmp::Marker::Map16 => {
            let cnt = read_len!(u16: input, start = 1);
            3 + read_len!(2 * cnt => input, start = 3)
        }
        rmp::Marker::Map32 => {
            let cnt = read_len!(u32: input, start = 1);
            5 + read_len!(2 * cnt => input, start = 5)
        }

        // Ext are stored in an integer (8-bit, 16-bit, 32-bit), type (8-bit), and byte array
        rmp::Marker::FixExt1 => 3,
        rmp::Marker::FixExt2 => 4,
        rmp::Marker::FixExt4 => 6,
        rmp::Marker::FixExt8 => 10,
        rmp::Marker::FixExt16 => 18,
        rmp::Marker::Ext8 => 3 + read_len!(u8: input, start = 1),
        rmp::Marker::Ext16 => 4 + read_len!(u16: input, start = 1),
        rmp::Marker::Ext32 => 6 + read_len!(u32: input, start = 1),

        // NOTE: This is marked in the msgpack spec as never being used, so we return none
        //       as this is signfies something has gone wrong!
        rmp::Marker::Reserved => return None,
    })
}

/// Reads the str bytes from msgpack input, including the marker and len bytes.
///
/// * If succeeds, returns (str, remaining).
/// * If fails, returns existing bytes.
fn read_str_bytes(input: &[u8]) -> Result<(&str, &[u8]), &[u8]> {
    match rmp::decode::read_str_from_slice(input) {
        Ok(x) => Ok(x),
        Err(_) => Err(input),
    }
}

/// Reads a str key from msgpack input and checks if it matches `key`. If so, the input is
/// advanced, otherwise the original input is returned.
///
/// * If key read successfully and matches, returns (unit, remaining).
/// * Otherwise, returns existing bytes.
fn read_key_eq<'a>(input: &'a [u8], key: &str) -> Result<((), &'a [u8]), &'a [u8]> {
    match read_str_bytes(input) {
        Ok((s, input)) if s == key => Ok(((), input)),
        _ => Err(input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod read_str_bytes {
        use super::*;
        use test_log::test;

        #[test]
        fn should_fail_if_input_is_empty() {
            let input = read_str_bytes(&[]).unwrap_err();
            assert!(input.is_empty());
        }

        #[test]
        fn should_fail_if_input_does_not_start_with_str() {
            let input = read_str_bytes(&[0xff, 0xa5, b'h', b'e', b'l', b'l', b'o']).unwrap_err();
            assert_eq!(input, [0xff, 0xa5, b'h', b'e', b'l', b'l', b'o']);
        }

        #[test]
        fn should_succeed_if_input_starts_with_str() {
            let (s, remaining) =
                read_str_bytes(&[0xa5, b'h', b'e', b'l', b'l', b'o', 0xff]).unwrap();
            assert_eq!(s, "hello");
            assert_eq!(remaining, [0xff]);
        }
    }

    mod read_key_eq {
        use super::*;
        use test_log::test;

        #[test]
        fn should_fail_if_input_is_empty() {
            let input = read_key_eq(&[], "key").unwrap_err();
            assert!(input.is_empty());
        }

        #[test]
        fn should_fail_if_input_does_not_start_with_str() {
            let input = &[
                0xff,
                rmp::Marker::FixStr(5).to_u8(),
                b'h',
                b'e',
                b'l',
                b'l',
                b'o',
            ];
            let remaining = read_key_eq(input, "key").unwrap_err();
            assert_eq!(remaining, input);
        }

        #[test]
        fn should_fail_if_read_key_does_not_match_specified_key() {
            let input = &[
                rmp::Marker::FixStr(5).to_u8(),
                b'h',
                b'e',
                b'l',
                b'l',
                b'o',
                0xff,
            ];
            let remaining = read_key_eq(input, "key").unwrap_err();
            assert_eq!(remaining, input);
        }

        #[test]
        fn should_succeed_if_read_key_matches_specified_key() {
            let input = &[
                rmp::Marker::FixStr(5).to_u8(),
                b'h',
                b'e',
                b'l',
                b'l',
                b'o',
                0xff,
            ];
            let (_, remaining) = read_key_eq(input, "hello").unwrap();
            assert_eq!(remaining, [0xff]);
        }
    }

    mod read_header_bytes {
        use super::*;
        use test_log::test;

        #[test]
        fn should_fail_if_input_is_empty() {
            let input = vec![];
            assert!(read_header_bytes(&input).is_err());
        }

        #[test]
        fn should_fail_if_not_a_map() {
            // Provide an array instead of a map
            let input = vec![0x93, 0xa3, b'a', b'b', b'c', 0xcc, 0xff, 0xc2];
            assert!(read_header_bytes(&input).is_err());
        }

        #[test]
        fn should_fail_if_cannot_read_str_key_length() {
            let input = vec![
                0x81, // valid map with 1 pair, but key is not a str
                0x03, 0xa3, b'a', b'b', b'c', // 3 -> "abc"
            ];
            assert!(read_header_bytes(&input).is_err());
        }
        #[test]
        fn should_fail_if_key_length_exceeds_remaining_bytes() {
            let input = vec![
                0x81, // valid map with 1 pair, but key length is too long
                0xa8, b'a', b'b', b'c', // key: "abc" (but len is much greater)
                0xa3, b'a', b'b', b'c', // value: "abc"
            ];
            assert!(read_header_bytes(&input).is_err());
        }

        #[test]
        fn should_fail_if_missing_value_for_key() {
            let input = vec![
                0x81, // valid map with 1 pair, but value is missing
                0xa3, b'a', b'b', b'c', // key: "abc"
            ];
            assert!(read_header_bytes(&input).is_err());
        }

        #[test]
        fn should_fail_if_unable_to_read_value_length() {
            let input = vec![
                0x81, // valid map with 1 pair, but value is missing
                0xa3, b'a', b'b', b'c', // key: "abc"
                0xd9, // value: str 8 with missing length
            ];
            assert!(read_header_bytes(&input).is_err());
        }

        #[test]
        fn should_fail_if_value_length_exceeds_remaining_bytes() {
            let input = vec![
                0x81, // valid map with 1 pair, but value is too long
                0xa3, b'a', b'b', b'c', // key: "abc"
                0xa2, b'd', // value: fixstr w/ len 1 too long
            ];
            assert!(read_header_bytes(&input).is_err());
        }

        #[test]
        fn should_succeed_with_empty_map() {
            // fixmap with 0 pairs
            let input = vec![0x80];
            let (header, _) = read_header_bytes(&input).unwrap();
            assert_eq!(header, input);

            // map 16 with 0 pairs
            let input = vec![0xde, 0x00, 0x00];
            let (header, _) = read_header_bytes(&input).unwrap();
            assert_eq!(header, input);

            // map 32 with 0 pairs
            let input = vec![0xdf, 0x00, 0x00, 0x00, 0x00];
            let (header, _) = read_header_bytes(&input).unwrap();
            assert_eq!(header, input);
        }

        #[test]
        fn should_succeed_with_single_key_value_map() {
            // fixmap with single pair
            let input = vec![
                0x81, // valid map with 1 pair
                0xa3, b'k', b'e', b'y', // key: "key"
                0xa5, b'v', b'a', b'l', b'u', b'e', // value: "value"
            ];
            let (header, _) = read_header_bytes(&input).unwrap();
            assert_eq!(header, input);

            // map 16 with single pair
            let input = vec![
                0xde, 0x00, 0x01, // valid map with 1 pair
                0xa3, b'k', b'e', b'y', // key: "key"
                0xa5, b'v', b'a', b'l', b'u', b'e', // value: "value"
            ];
            let (header, _) = read_header_bytes(&input).unwrap();
            assert_eq!(header, input);

            // map 32 with single pair
            let input = vec![
                0xdf, 0x00, 0x00, 0x00, 0x01, // valid map with 1 pair
                0xa3, b'k', b'e', b'y', // key: "key"
                0xa5, b'v', b'a', b'l', b'u', b'e', // value: "value"
            ];
            let (header, _) = read_header_bytes(&input).unwrap();
            assert_eq!(header, input);
        }

        #[test]
        fn should_succeed_with_multiple_key_value_map() {
            // fixmap with single pair
            let input = vec![
                0x82, // valid map with 2 pairs
                0xa3, b'k', b'e', b'y', // key: "key"
                0xa5, b'v', b'a', b'l', b'u', b'e', // value: "value"
                0xa3, b'y', b'e', b'k', // key: "yek"
                0x7b, // value: 123 (fixint)
            ];
            let (header, _) = read_header_bytes(&input).unwrap();
            assert_eq!(header, input);

            // map 16 with single pair
            let input = vec![
                0xde, 0x00, 0x02, // valid map with 2 pairs
                0xa3, b'k', b'e', b'y', // key: "key"
                0xa5, b'v', b'a', b'l', b'u', b'e', // value: "value"
                0xa3, b'y', b'e', b'k', // key: "yek"
                0x7b, // value: 123 (fixint)
            ];
            let (header, _) = read_header_bytes(&input).unwrap();
            assert_eq!(header, input);

            // map 32 with single pair
            let input = vec![
                0xdf, 0x00, 0x00, 0x00, 0x02, // valid map with 2 pairs
                0xa3, b'k', b'e', b'y', // key: "key"
                0xa5, b'v', b'a', b'l', b'u', b'e', // value: "value"
                0xa3, b'y', b'e', b'k', // key: "yek"
                0x7b, // value: 123 (fixint)
            ];
            let (header, _) = read_header_bytes(&input).unwrap();
            assert_eq!(header, input);
        }

        #[test]
        fn should_succeed_with_nested_map() {
            // fixmap with single pair
            let input = vec![
                0x81, // valid map with 1 pair
                0xa3, b'm', b'a', b'p', // key: "map"
                0x81, // value: valid map with 1 pair
                0xa3, b'k', b'e', b'y', // key: "key"
                0xa5, b'v', b'a', b'l', b'u', b'e', // value: "value"
            ];
            let (header, _) = read_header_bytes(&input).unwrap();
            assert_eq!(header, input);
        }

        #[test]
        fn should_only_consume_map_from_input() {
            // fixmap with single pair
            let input = vec![
                0x81, // valid map with 1 pair
                0xa3, b'k', b'e', b'y', // key: "key"
                0xa5, b'v', b'a', b'l', b'u', b'e', // value: "value"
                0xa4, b'm', b'o', b'r', b'e', // "more" (fixstr)
            ];
            let (header, remaining) = read_header_bytes(&input).unwrap();
            assert_eq!(
                header,
                vec![
                    0x81, // valid map with 1 pair
                    0xa3, b'k', b'e', b'y', // key: "key"
                    0xa5, b'v', b'a', b'l', b'u', b'e', // value: "value"
                ]
            );
            assert_eq!(
                remaining,
                vec![
                0xa4, b'm', b'o', b'r', b'e', // "more" (fixstr)
            ]
            );
        }
    }

    mod find_msgpack_byte_len {
        use super::*;
        use test_log::test;

        #[test]
        fn should_return_none_if_input_is_empty() {
            let input = vec![];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, None, "Wrong len for {input:X?}");
        }

        #[test]
        fn should_return_none_if_input_has_reserved_marker() {
            let input = vec![rmp::Marker::Reserved.to_u8()];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, None, "Wrong len for {input:X?}");
        }

        #[test]
        fn should_return_1_if_input_is_nil() {
            let input = vec![0xc0];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1), "Wrong len for {input:X?}");
        }

        #[test]
        fn should_return_1_if_input_is_a_boolean() {
            let input = vec![0xc2]; // false
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1), "Wrong len for {input:X?}");

            let input = vec![0xc3]; // true
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1), "Wrong len for {input:X?}");
        }

        #[test]
        fn should_return_appropriate_len_if_input_is_some_integer() {
            let input = vec![0x00]; // positive fixint (0)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1), "Wrong len for {input:X?}");

            let input = vec![0xff]; // negative fixint (-1)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1), "Wrong len for {input:X?}");

            let input = vec![0xcc, 0xff]; // unsigned 8-bit (255)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(2), "Wrong len for {input:X?}");

            let input = vec![0xcd, 0xff, 0xff]; // unsigned 16-bit (65535)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(3), "Wrong len for {input:X?}");

            let input = vec![0xce, 0xff, 0xff, 0xff, 0xff]; // unsigned 32-bit (4294967295)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(5), "Wrong len for {input:X?}");

            let input = vec![0xcf, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]; // unsigned 64-bit (4294967296)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(9), "Wrong len for {input:X?}");

            let input = vec![0xd0, 0x81]; // signed 8-bit (-127)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(2), "Wrong len for {input:X?}");

            let input = vec![0xd1, 0x80, 0x01]; // signed 16-bit (-32767)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(3), "Wrong len for {input:X?}");

            let input = vec![0xd2, 0x80, 0x00, 0x00, 0x01]; // signed 32-bit (-2147483647)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(5), "Wrong len for {input:X?}");

            let input = vec![0xd3, 0xff, 0xff, 0xff, 0xff, 0x80, 0x00, 0x00, 0x00]; // signed 64-bit (-2147483648)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(9), "Wrong len for {input:X?}");
        }

        #[test]
        fn should_return_appropriate_len_if_input_is_some_float() {
            let input = vec![0xca, 0x3d, 0xcc, 0xcc, 0xcd]; // f32 (0.1)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(5), "Wrong len for {input:X?}");

            let input = vec![0xcb, 0x3f, 0xb9, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a]; // f64 (0.1)
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(9), "Wrong len for {input:X?}");
        }

        #[test]
        fn should_return_appropriate_len_if_input_is_some_str() {
            // fixstr (31 bytes max)
            let input = vec![0xa5, b'h', b'e', b'l', b'l', b'o'];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(5 + 1), "Wrong len for {input:X?}");

            // str 8 will read second byte (u8) for size
            let input = vec![0xd9, 0xff, b'd', b'a', b't', b'a'];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(u8::MAX as u64 + 2), "Wrong len for {input:X?}");

            // str 16 will read second & third bytes (u16) for size
            let input = vec![0xda, 0xff, 0xff, b'd', b'a', b't', b'a'];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(u16::MAX as u64 + 3), "Wrong len for {input:X?}");

            // str 32 will read second, third, fourth, & fifth bytes (u32) for size
            let input = vec![0xdb, 0xff, 0xff, 0xff, 0xff, b'd', b'a', b't', b'a'];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(u32::MAX as u64 + 5), "Wrong len for {input:X?}");
        }

        #[test]
        fn should_return_appropriate_len_if_input_is_some_bin() {
            // bin 8 will read second byte (u8) for size
            let input = vec![0xc4, 0xff, b'd', b'a', b't', b'a'];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(u8::MAX as u64 + 2), "Wrong len for {input:X?}");

            // bin 16 will read second & third bytes (u16) for size
            let input = vec![0xc5, 0xff, 0xff, b'd', b'a', b't', b'a'];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(u16::MAX as u64 + 3), "Wrong len for {input:X?}");

            // bin 32 will read second, third, fourth, & fifth bytes (u32) for size
            let input = vec![0xc6, 0xff, 0xff, 0xff, 0xff, b'd', b'a', b't', b'a'];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(u32::MAX as u64 + 5), "Wrong len for {input:X?}");
        }

        #[test]
        fn should_return_appropriate_len_if_input_is_some_array() {
            // fixarray has a length up to 15 objects
            //
            // In this example, we have an array of 3 objects that are a str, integer, and bool
            let input = vec![0x93, 0xa3, b'a', b'b', b'c', 0xcc, 0xff, 0xc2];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1 + 4 + 2 + 1), "Wrong len for {input:X?}");

            // Invalid fixarray count should return none
            let input = vec![0x93, 0xa3, b'a', b'b', b'c', 0xcc, 0xff];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, None, "Wrong len for {input:X?}");

            // array 16 will read second & third bytes (u16) for object length
            //
            // In this example, we have an array of 3 objects that are a str, integer, and bool
            let input = vec![0xdc, 0x00, 0x03, 0xa3, b'a', b'b', b'c', 0xcc, 0xff, 0xc2];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(3 + 4 + 2 + 1), "Wrong len for {input:X?}");

            // Invalid array 16 count should return none
            let input = vec![0xdc, 0x00, 0x03, 0xa3, b'a', b'b', b'c', 0xcc, 0xff];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, None, "Wrong len for {input:X?}");

            // array 32 will read second, third, fourth, & fifth bytes (u32) for object length
            let input = vec![
                0xdd, 0x00, 0x00, 0x00, 0x03, 0xa3, b'a', b'b', b'c', 0xcc, 0xff, 0xc2,
            ];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(5 + 4 + 2 + 1), "Wrong len for {input:X?}");

            // Invalid array 32 count should return none
            let input = vec![
                0xdd, 0x00, 0x00, 0x00, 0x03, 0xa3, b'a', b'b', b'c', 0xcc, 0xff,
            ];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, None, "Wrong len for {input:X?}");
        }

        #[test]
        fn should_return_appropriate_len_if_input_is_some_map() {
            // fixmap has a length up to 2*15 objects
            let input = vec![
                0x83, // 3 objects /w keys
                0x03, 0xa3, b'a', b'b', b'c', // 3 -> "abc"
                0xa3, b'a', b'b', b'c', 0xcc, 0xff, // "abc" -> 255
                0xc3, 0xc2, // true -> false
            ];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1 + 5 + 6 + 2), "Wrong len for {input:X?}");

            // Invalid fixmap count should return none
            let input = vec![
                0x83, // 3 objects /w keys
                0x03, 0xa3, b'a', b'b', b'c', // 3 -> "abc"
                0xa3, b'a', b'b', b'c', 0xcc, 0xff, // "abc" -> 255
                0xc3, // true -> ???
            ];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, None, "Wrong len for {input:X?}");

            // map 16 will read second & third bytes (u16) for object length
            let input = vec![
                0xde, 0x00, 0x03, // 3 objects w/ keys
                0x03, 0xa3, b'a', b'b', b'c', // 3 -> "abc"
                0xa3, b'a', b'b', b'c', 0xcc, 0xff, // "abc" -> 255
                0xc3, 0xc2, // true -> false
            ];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(3 + 5 + 6 + 2), "Wrong len for {input:X?}");

            // Invalid map 16 count should return none
            let input = vec![
                0xde, 0x00, 0x03, // 3 objects w/ keys
                0x03, 0xa3, b'a', b'b', b'c', // 3 -> "abc"
                0xa3, b'a', b'b', b'c', 0xcc, 0xff, // "abc" -> 255
                0xc3, // true -> ???
            ];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, None, "Wrong len for {input:X?}");

            // map 32 will read second, third, fourth, & fifth bytes (u32) for object length
            let input = vec![
                0xdf, 0x00, 0x00, 0x00, 0x03, // 3 objects w/ keys
                0x03, 0xa3, b'a', b'b', b'c', // 3 -> "abc"
                0xa3, b'a', b'b', b'c', 0xcc, 0xff, // "abc" -> 255
                0xc3, 0xc2, // true -> false
            ];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(5 + 5 + 6 + 2), "Wrong len for {input:X?}");

            // Invalid map 32 count should return none
            let input = vec![
                0xdf, 0x00, 0x00, 0x00, 0x03, // 3 objects w/ keys
                0x03, 0xa3, b'a', b'b', b'c', // 3 -> "abc"
                0xa3, b'a', b'b', b'c', 0xcc, 0xff, // "abc" -> 255
                0xc3, // true -> ???
            ];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, None, "Wrong len for {input:X?}");
        }

        #[test]
        fn should_return_appropriate_len_if_input_is_some_ext() {
            // fixext 1 claims single data byte (excluding type)
            let input = vec![0xd4, 0x00, 0x12];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1 + 1 + 1), "Wrong len for {input:X?}");

            // fixext 2 claims two data bytes (excluding type)
            let input = vec![0xd5, 0x00, 0x12, 0x34];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1 + 1 + 2), "Wrong len for {input:X?}");

            // fixext 4 claims four data bytes (excluding type)
            let input = vec![0xd6, 0x00, 0x12, 0x34, 0x56, 0x78];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1 + 1 + 4), "Wrong len for {input:X?}");

            // fixext 8 claims eight data bytes (excluding type)
            let input = vec![0xd7, 0x00, 0x12, 0x34, 0x56, 0x78];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1 + 1 + 8), "Wrong len for {input:X?}");

            // fixext 16 claims sixteen data bytes (excluding type)
            let input = vec![0xd8, 0x00, 0x12, 0x34, 0x56, 0x78];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(1 + 1 + 16), "Wrong len for {input:X?}");

            // ext 8 will read second byte (u8) for size (excluding type)
            let input = vec![0xc7, 0xff, 0x00, b'd', b'a', b't', b'a'];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(u8::MAX as u64 + 3), "Wrong len for {input:X?}");

            // ext 16 will read second & third bytes (u16) for size (excluding type)
            let input = vec![0xc8, 0xff, 0xff, 0x00, b'd', b'a', b't', b'a'];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(u16::MAX as u64 + 4), "Wrong len for {input:X?}");

            // ext 32 will read second, third, fourth, & fifth bytes (u32) for size (excluding type)
            let input = vec![0xc9, 0xff, 0xff, 0xff, 0xff, 0x00, b'd', b'a', b't', b'a'];
            let len = find_msgpack_byte_len(&input);
            assert_eq!(len, Some(u32::MAX as u64 + 6), "Wrong len for {input:X?}");
        }
    }
}
