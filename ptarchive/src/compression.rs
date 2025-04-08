use crate::annotations::MediaType;

use std::convert::TryFrom;

use async_compression::tokio::bufread::{GzipDecoder, XzDecoder, ZstdDecoder};
use async_compression::tokio::bufread::{GzipEncoder, XzEncoder, ZstdEncoder};
use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncRead};

#[derive(Copy, Clone, clap::ValueEnum, Default, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Compression {
    /// Compress files to `.xz` format
    #[default]
    XZ,
    /// Compress files to `.zst` format
    ZSTD,
    /// Compress files to `.gz` format
    GZIP,
    /// None
    None,
}

impl ToString for Compression {
    fn to_string(&self) -> String {
        match self {
            Compression::XZ => "xz",
            Compression::ZSTD => "zstd",
            Compression::GZIP => "gzip",
            Compression::None => "none",
        }
        .to_string()
    }
}

impl<'a> TryFrom<MediaType<'a>> for Compression {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_from(value: MediaType) -> Result<Self, Self::Error> {
        Ok(match value.split_once('+') {
            None => Compression::None,
            Some((_, compression_str)) => match compression_str {
                "xz" => Compression::XZ,
                "zst" | "zstd" => Compression::ZSTD,
                "gz" | "gzip" => Compression::GZIP,
                other => Err(format!("Unsupported compression algorithm: {other}"))?,
            },
        })
    }
}

impl Compression {
    pub fn compressor<R: AsyncBufRead + Unpin + Send + Sync + 'static>(
        &self,
        reader: R,
    ) -> Box<dyn AsyncRead + Unpin + Send + Sync> {
        match self {
            Compression::XZ => Box::new(XzEncoder::new(reader)),
            Compression::ZSTD => Box::new(ZstdEncoder::new(reader)),
            Compression::GZIP => Box::new(GzipEncoder::new(reader)),
            Compression::None => Box::new(reader),
        }
    }

    pub fn decompressor<R: AsyncBufRead + Unpin + Send + 'static>(
        &self,
        reader: R,
    ) -> Box<dyn AsyncRead + Unpin + Send> {
        match self {
            Compression::XZ => Box::new(XzDecoder::new(reader)),
            Compression::ZSTD => Box::new(ZstdDecoder::new(reader)),
            Compression::GZIP => Box::new(GzipDecoder::new(reader)),
            Compression::None => Box::new(reader),
        }
    }
}
