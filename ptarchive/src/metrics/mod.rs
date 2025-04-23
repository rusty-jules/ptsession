mod init;
mod volume;

pub use init::*;
pub use volume::*;

pub static BYTES_READ: &str = "bytes_read";
pub static BYTES_WRITTEN: &str = "bytes_written";
pub static TOTAL_BYTES_READ: &str = "total_bytes_read";
pub static TOTAL_BYTES_WRITTEN: &str = "total_bytes_written";
pub static FILES_PROCESSED: &str = "files_processed";
pub static FILE_FAILURES: &str = "file_failures";
pub static UPLOAD_SPEED: &str = "upload_speed";
