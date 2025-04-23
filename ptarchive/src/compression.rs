use crate::annotations::MediaType;

use std::convert::TryFrom;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;

use async_compression::tokio::bufread::{GzipDecoder, XzDecoder, ZstdDecoder};
use async_compression::tokio::bufread::{GzipEncoder, XzEncoder, ZstdEncoder};
use async_compression::Level;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncRead};

#[derive(Copy, Clone, clap::ValueEnum, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    /// Compress files to `.xz` format
    XZ,
    /// Compress files to `.zst` format
    #[default]
    ZSTD,
    /// Compress files to `.gz` format
    GZIP,
    /// None
    None,
}

// The benefits of enum variants over Box<dyn Trait> are many when:
//
// - all possible variants are known at compile time
// - only one variant is used per execution
// - the variant's methods are in the hot path and frequently called
// - the size difference of the variant structs are neglibile
//
// This comes at the cost of code amount and thus legibility.
impl Compression {
    pub fn compressor<R: AsyncBufRead + Unpin + Send + Sync + 'static>(
        self,
        reader: R,
        level: Option<async_compression::Level>,
    ) -> Compressor<R> {
        Compressor::from((self, reader, level))
    }

    pub fn decompressor<R: AsyncBufRead + Unpin + Send + 'static>(
        self,
        reader: R,
    ) -> Decompressor<R> {
        Decompressor::from((self, reader))
    }

    #[allow(dead_code)]
    pub fn dyn_compressor<R: AsyncBufRead + Unpin + Send + Sync + 'static>(
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

    #[allow(dead_code)]
    pub fn dyn_decompressor<R: AsyncBufRead + Unpin + Send + 'static>(
        self,
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

impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Compression::XZ => "xz",
            Compression::ZSTD => "zstd",
            Compression::GZIP => "gzip",
            Compression::None => "none",
        };
        write!(f, "{s}")
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

pub enum CompressorEncoder<R: AsyncRead + AsyncBufRead + Unpin> {
    XZ(XzEncoder<R>),
    ZSTD(ZstdEncoder<R>),
    GZIP(GzipEncoder<R>),
    None(R),
}

pub struct Compressor<R>
where
    R: AsyncRead + AsyncBufRead + Unpin,
{
    encoder: CompressorEncoder<R>,
}

impl<R> AsyncRead for Compressor<R>
where
    R: AsyncRead + AsyncBufRead + Unpin,
{
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut self.encoder {
            CompressorEncoder::XZ(encoder) => Pin::new(encoder).poll_read(cx, buf),
            CompressorEncoder::ZSTD(encoder) => Pin::new(encoder).poll_read(cx, buf),
            CompressorEncoder::GZIP(encoder) => Pin::new(encoder).poll_read(cx, buf),
            CompressorEncoder::None(encoder) => Pin::new(encoder).poll_read(cx, buf),
        }
    }
}

impl<R: AsyncRead + AsyncBufRead + Unpin> From<(Compression, R, Option<Level>)> for Compressor<R> {
    fn from((compression, reader, level): (Compression, R, Option<Level>)) -> Self {
        match compression {
            Compression::XZ => Self {
                encoder: CompressorEncoder::XZ(XzEncoder::with_quality(
                    reader,
                    level.unwrap_or(Level::Default),
                )),
            },
            Compression::ZSTD => Self {
                encoder: CompressorEncoder::ZSTD(ZstdEncoder::with_quality(
                    reader,
                    level.unwrap_or(Level::Default),
                )),
            },
            Compression::GZIP => Self {
                encoder: CompressorEncoder::GZIP(GzipEncoder::with_quality(
                    reader,
                    level.unwrap_or(Level::Default),
                )),
            },
            Compression::None => Self {
                encoder: CompressorEncoder::None(reader),
            },
        }
    }
}

impl<R: AsyncRead + AsyncBufRead + Unpin> From<(Compression, R)> for Compressor<R> {
    fn from((compression, reader): (Compression, R)) -> Self {
        Compressor::from((compression, reader, None))
    }
}

pub struct Decompressor<R>
where
    R: AsyncRead + AsyncBufRead + Unpin,
{
    decoder: DecompressorDecoder<R>,
}

impl<R> AsyncRead for Decompressor<R>
where
    R: AsyncRead + AsyncBufRead + Unpin,
{
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut self.decoder {
            DecompressorDecoder::XZ(decoder) => Pin::new(decoder).poll_read(cx, buf),
            DecompressorDecoder::ZSTD(decoder) => Pin::new(decoder).poll_read(cx, buf),
            DecompressorDecoder::GZIP(decoder) => Pin::new(decoder).poll_read(cx, buf),
            DecompressorDecoder::None(decoder) => Pin::new(decoder).poll_read(cx, buf),
        }
    }
}

pub enum DecompressorDecoder<R>
where
    R: AsyncRead + AsyncBufRead + Unpin,
{
    XZ(XzDecoder<R>),
    ZSTD(ZstdDecoder<R>),
    GZIP(GzipDecoder<R>),
    None(R),
}

impl<R> Deref for Decompressor<R>
where
    R: AsyncRead + AsyncBufRead + Unpin + Send + Sync + 'static,
{
    type Target = dyn AsyncRead + Unpin + Send + Sync;

    fn deref(&self) -> &Self::Target {
        match &self.decoder {
            DecompressorDecoder::XZ(decoder) => decoder,
            DecompressorDecoder::ZSTD(decoder) => decoder,
            DecompressorDecoder::GZIP(decoder) => decoder,
            DecompressorDecoder::None(decoder) => decoder,
        }
    }
}

impl<R> DerefMut for Decompressor<R>
where
    R: AsyncRead + AsyncBufRead + Unpin + Send + Sync + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        match &mut self.decoder {
            DecompressorDecoder::XZ(decoder) => decoder,
            DecompressorDecoder::ZSTD(decoder) => decoder,
            DecompressorDecoder::GZIP(decoder) => decoder,
            DecompressorDecoder::None(decoder) => decoder,
        }
    }
}

impl<R: AsyncRead + AsyncBufRead + Unpin> From<(Compression, R)> for Decompressor<R> {
    fn from((compression, reader): (Compression, R)) -> Self {
        match compression {
            Compression::XZ => Self {
                decoder: DecompressorDecoder::XZ(XzDecoder::new(reader)),
            },
            Compression::ZSTD => Self {
                decoder: DecompressorDecoder::ZSTD(ZstdDecoder::new(reader)),
            },
            Compression::GZIP => Self {
                decoder: DecompressorDecoder::GZIP(GzipDecoder::new(reader)),
            },
            Compression::None => Self {
                decoder: DecompressorDecoder::None(reader),
            },
        }
    }
}
