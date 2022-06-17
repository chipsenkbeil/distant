use derive_more::{Display, Error};
use portable_pty::PtySize as PortablePtySize;
use serde::{Deserialize, Serialize};
use std::{num::ParseIntError, str::FromStr};

/// Represents the size associated with a remote PTY
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

impl From<PortablePtySize> for PtySize {
    fn from(size: PortablePtySize) -> Self {
        Self {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.pixel_width,
            pixel_height: size.pixel_height,
        }
    }
}

impl From<PtySize> for PortablePtySize {
    fn from(size: PtySize) -> Self {
        Self {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.pixel_width,
            pixel_height: size.pixel_height,
        }
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
