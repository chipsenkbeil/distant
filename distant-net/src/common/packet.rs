/// Represents a generic id type
pub type Id = String;

/// Represents a packet header for a request or response
pub type Header = std::collections::HashMap<String, crate::common::Value>;

/// Generates a new [`Header`] of key/value pairs based on literals.
///
/// ```
/// use distant_net::header;
///
/// let _header = header!("key" -> "value", "key2" -> 123);
/// ```
#[macro_export]
macro_rules! header {
    ($($key:literal -> $value:expr),* $(,)?) => {{
        let mut _header = ::std::collections::HashMap::new();

        $(
            _header.insert($key.to_string(), $crate::common::Value::from($value));
        )*

        _header
    }};
}

mod request;
mod response;

pub use request::*;
pub use response::*;

use std::io::Cursor;

/// Reads the header bytes from msgpack input, including the marker and len bytes.
///
/// * If succeeds, returns (header, remaining).
/// * If fails, returns existing bytes.
fn read_header_bytes(input: &[u8]) -> Result<(&[u8], &[u8]), &[u8]> {
    let mut cursor = Cursor::new(input);

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

        // Point locally to just past the str key so we can determine next byte len to skip
        let input = &input[cursor.position() as usize..];

        // Read the type of object and advance accordingly
        match find_msgpack_byte_len(input) {
            Some(len) => cursor.set_position(cursor.position() + len),
            None => return Err(input),
        }
    }

    let pos = cursor.position() as usize;
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
