// SPDX-License-Identifier: MPL-2.0
use std::fmt;
#[derive(Debug)]
pub enum CodecError {
    Parse(String),
    UnsupportedFormat(String),
    Other(String),
}
pub type Result<T> = std::result::Result<T, CodecError>;
impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(s) | Self::UnsupportedFormat(s) | Self::Other(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for CodecError {}
impl From<std::io::Error> for CodecError {
    fn from(e: std::io::Error) -> Self {
        Self::Other(e.to_string())
    }
}
