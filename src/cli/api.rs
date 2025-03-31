use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Arguments {
    /// Subcommand
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[command(flatten)]
    global_opts: GlobalOpts,
}

#[derive(Debug, Args)]
pub struct GlobalOpts {
    /// Whether to output json
    #[arg(short, long, value_name = "json")]
    pub json: Option<bool>,
}

#[derive(Debug, Args)]
pub struct PushArgs {
    /// OCI repository to push to
    #[arg(value_name = "repository")]
    pub repository: String,

    /// The Pro Tools Session to Read
    #[arg(value_name = "ptx")]
    pub ptx_file: PathBuf,

    /// Ignore missing session files
    // TODO: make exclusive with arguments below
    #[arg(short, long, default_value_t = false, value_name = "ignore-missing")]
    ignore_missing: bool,

    /// File paths to search for files in
    #[arg(short, long, value_delimiter = ' ', num_args = 1.., value_name = "search-paths", )]
    search_path: Option<Vec<String>>,

    /// Find by filename
    #[arg(short, long, default_value_t = true, value_name = "filename")]
    file_name: bool,

    /// Find by unique id
    #[arg(short, long, default_value_t = true, value_name = "unique-id")]
    unique_id: bool,

    /// Find by length
    #[arg(short, long, default_value_t = true, value_name = "length")]
    length: bool,

    /// Compression algorithm for uploaded files
    #[arg(short, long, default_value_t = Compression::XZ, value_name = "compression")]
    pub compression: Compression,

    /// Maximum number of uploads
    #[arg(short, long, default_value_t = 5, value_name = "parallelism")]
    pub parallelism: usize,
}

#[derive(Subcommand)]
#[command(version, about, long_about = None)]
pub enum Commands {
    Push(PushArgs),
    Pull {
        #[arg(value_name = "repository")]
        repository: String,

        #[arg(value_name = "decompress")]
        decompress: bool,
    },
    Info {
        #[arg(value_name = "repository")]
        repository: String,
    },
}

#[derive(Clone, clap::ValueEnum, Default, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Compression {
    /// Compress files to `.xz` format
    #[default]
    XZ,
    /// Compress files to `.zstd` format
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
