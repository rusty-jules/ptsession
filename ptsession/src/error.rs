use std::error::Error;
use std::io::Error as IoError;
use std::fmt;

#[derive(Debug)]
pub enum PtError {
    Decrypt(IoError),
    BitCode,
    Endianness,
    Version(String),
    Parse,
    Io(IoError),
}

impl fmt::Display for PtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use PtError::*;
        match self {
            Decrypt(err) => write!(f, "Could not decrypt file: {}", err),
            BitCode => write!(f, "Could not verify BitCode. Not a Pro Tools file."),
            Endianness => write!(f, "Could not parse the endianness. Expected 0 or 1"),
            Version(err) => write!(f, "Pro Tools version not supported. Only support 5 - 12. {}", err),
            Parse => write!(f, "Error parsing blocks"),
            Io(err) => write!(f, "IO Error: {}", err),
        }
    }
}

impl Error for PtError {}
