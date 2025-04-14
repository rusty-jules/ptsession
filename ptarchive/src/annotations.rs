use std::ops::Deref;

/// Layer [`mediaType`](https://github.com/opencontainers/image-spec/blob/main/layer.md)
pub struct MediaType<'a>(pub &'a str);

impl<'a> Deref for MediaType<'a> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

/// Digest of the a blob before compression
pub const IO_PTSESSION_ORIGINAL_DIGEST: &str = "io.ptsession.original.digest";
/// Size of a blob before compression
pub const IO_PTSESSION_ORIGINAL_SIZE: &str = "io.ptsession.original.size";
/// Sample rate of a pro tools session config
pub const IO_PTSESSION_SAMPLE_RATE: &str = "io.ptsession.sample_rate";
/// Compression level for enabling recomputation of compressed digests
pub const IO_PTSESSION_COMPRESSION_LEVEL: &str = "io.ptsession.compression.level";
/// Date the file was last modified
pub const IO_PTSESSION_TIME_MODIFIED: &str = "io.ptsession.time.modified";
/// Whether to decompress a blob on pull
pub const IO_DEIS_ORAS_CONTENT_UNPACK: &str = "io.deis.oras.content.unpack";
