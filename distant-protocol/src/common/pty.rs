use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;

use derive_more::{Display, Error};
use serde::{Deserialize, Serialize};

/// Represents the size associated with a remote PTY
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtySize {
    /// Number of lines of text
    pub rows: u16,

    /// Number of columns of text
    pub cols: u16,

    /// Width of a cell in pixels. Note that some systems never fill this value and ignore it.
    #[serde(default)]
    pub pixel_width: u16,

    /// Height of a cell in pixels. Note that some systems never fill this value and ignore it.
    #[serde(default)]
    pub pixel_height: u16,
}

impl PtySize {
    /// Creates new size using just rows and columns
    pub fn from_rows_and_cols(rows: u16, cols: u16) -> Self {
        Self {
            rows,
            cols,
            ..Default::default()
        }
    }
}

impl fmt::Display for PtySize {
    /// Prints out `rows,cols[,pixel_width,pixel_height]` where the
    /// pixel width and pixel height are only included if either
    /// one of them is not zero
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", self.rows, self.cols)?;
        if self.pixel_width > 0 || self.pixel_height > 0 {
            write!(f, ",{},{}", self.pixel_width, self.pixel_height)?;
        }

        Ok(())
    }
}

impl Default for PtySize {
    fn default() -> Self {
        PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Display, Error)]
pub enum PtySizeParseError {
    MissingRows,
    MissingColumns,
    InvalidRows(ParseIntError),
    InvalidColumns(ParseIntError),
    InvalidPixelWidth(ParseIntError),
    InvalidPixelHeight(ParseIntError),
}

impl FromStr for PtySize {
    type Err = PtySizeParseError;

    /// Attempts to parse a str into PtySize using one of the following formats:
    ///
    /// * rows,cols (defaults to 0 for pixel_width & pixel_height)
    /// * rows,cols,pixel_width,pixel_height
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tokens = s.split(',');

        Ok(Self {
            rows: tokens
                .next()
                .ok_or(PtySizeParseError::MissingRows)?
                .trim()
                .parse()
                .map_err(PtySizeParseError::InvalidRows)?,
            cols: tokens
                .next()
                .ok_or(PtySizeParseError::MissingColumns)?
                .trim()
                .parse()
                .map_err(PtySizeParseError::InvalidColumns)?,
            pixel_width: tokens
                .next()
                .map(|s| s.trim().parse())
                .transpose()
                .map_err(PtySizeParseError::InvalidPixelWidth)?
                .unwrap_or(0),
            pixel_height: tokens
                .next()
                .map(|s| s.trim().parse())
                .transpose()
                .map_err(PtySizeParseError::InvalidPixelHeight)?
                .unwrap_or(0),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_be_able_to_serialize_to_json() {
        let size = PtySize {
            rows: 10,
            cols: 20,
            pixel_width: 30,
            pixel_height: 40,
        };

        let value = serde_json::to_value(size).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "rows": 10,
                "cols": 20,
                "pixel_width": 30,
                "pixel_height": 40,
            })
        );
    }

    #[test]
    fn should_be_able_to_deserialize_minimal_size_from_json() {
        let value = serde_json::json!({
            "rows": 10,
            "cols": 20,
        });

        let size: PtySize = serde_json::from_value(value).unwrap();
        assert_eq!(
            size,
            PtySize {
                rows: 10,
                cols: 20,
                pixel_width: 0,
                pixel_height: 0,
            }
        );
    }

    #[test]
    fn should_be_able_to_deserialize_full_size_from_json() {
        let value = serde_json::json!({
            "rows": 10,
            "cols": 20,
            "pixel_width": 30,
            "pixel_height": 40,
        });

        let size: PtySize = serde_json::from_value(value).unwrap();
        assert_eq!(
            size,
            PtySize {
                rows: 10,
                cols: 20,
                pixel_width: 30,
                pixel_height: 40,
            }
        );
    }

    #[test]
    fn should_be_able_to_serialize_to_msgpack() {
        let size = PtySize {
            rows: 10,
            cols: 20,
            pixel_width: 30,
            pixel_height: 40,
        };

        // NOTE: We don't actually check the output here because it's an implementation detail
        // and could change as we change how serialization is done. This is merely to verify
        // that we can serialize since there are times when serde fails to serialize at
        // runtime.
        let _ = rmp_serde::encode::to_vec_named(&size).unwrap();
    }

    #[test]
    fn should_be_able_to_deserialize_minimal_size_from_msgpack() {
        // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
        // verify that we are not corrupting or causing issues when serializing on a
        // client/server and then trying to deserialize on the other side. This has happened
        // enough times with minor changes that we need tests to verify.
        #[derive(Serialize)]
        struct PartialSize {
            rows: u16,
            cols: u16,
        }
        let buf = rmp_serde::encode::to_vec_named(&PartialSize { rows: 10, cols: 20 }).unwrap();

        let size: PtySize = rmp_serde::decode::from_slice(&buf).unwrap();
        assert_eq!(
            size,
            PtySize {
                rows: 10,
                cols: 20,
                pixel_width: 0,
                pixel_height: 0,
            }
        );
    }

    #[test]
    fn should_be_able_to_deserialize_full_size_from_msgpack() {
        // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
        // verify that we are not corrupting or causing issues when serializing on a
        // client/server and then trying to deserialize on the other side. This has happened
        // enough times with minor changes that we need tests to verify.
        let buf = rmp_serde::encode::to_vec_named(&PtySize {
            rows: 10,
            cols: 20,
            pixel_width: 30,
            pixel_height: 40,
        })
        .unwrap();

        let size: PtySize = rmp_serde::decode::from_slice(&buf).unwrap();
        assert_eq!(
            size,
            PtySize {
                rows: 10,
                cols: 20,
                pixel_width: 30,
                pixel_height: 40,
            }
        );
    }
}
