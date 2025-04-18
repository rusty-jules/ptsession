use crate::compression::Compression;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Arguments {
    /// Subcommand
    #[command(subcommand)]
    pub command: Commands,

    #[command(flatten)]
    global_opts: GlobalOpts,
}

#[derive(Debug, Args)]
pub struct GlobalOpts {
    /// Whether to output json
    #[arg(short, long, action, value_name = "json")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PushArgs {
    /// OCI repository to push to
    #[arg(value_name = "repository")]
    pub repository: String,

    /// The Pro Tools Session to Read
    #[arg(value_name = "ptx")]
    pub ptx_file: PathBuf,

    /// Compression algorithm for uploaded files
    #[arg(short, long, default_value_t = Compression::ZSTD, value_name = "compression")]
    pub compression: Compression,

    /// Compression level to use
    #[arg(long, default_value_t = 7, value_name = "level")]
    pub level: i32,

    /// Maximum number of concurrent uploads
    #[arg(short, long, default_value_t = 5, value_name = "parallelism")]
    pub parallelism: usize,

    #[command(flatten)]
    pub find_args: FindArgs,
}

#[derive(Debug, Args)]
pub struct FindArgs {
    /// Ignore missing session files
    // TODO: make exclusive with arguments below
    #[arg(short, long, action, value_name = "ignore-missing")]
    pub ignore_missing: bool,

    /// File paths to search for files in
    #[arg(short, long, default_value = ".", num_args = 1.., value_name = "search-paths")]
    pub search_paths: Vec<String>,

    /// If search_paths is not passed, the maximum folder depth of folders to search for files in
    /// above and below and target session file's folder
    #[arg(short, long, default_value_t = 1, value_name = "depth")]
    pub depth: usize,

    /// Find files by matching filenames
    #[arg(short, long, default_value_t = true, value_name = "filename")]
    pub file_name: bool,

    /// Find files by matching file lengths
    #[arg(short, long, default_value_t = false, value_name = "length")]
    pub length: bool,

    /// Find files by their Pro Tools unique id
    #[arg(short, long, default_value_t = false, value_name = "unique-id")]
    pub unique_id: bool,
}

#[derive(Debug, Args)]
pub struct PullArgs {
    /// OCI repository to pull from
    #[arg(value_name = "repository")]
    pub repository: String,

    /// Whether to decompress files
    #[arg(short, long, default_value_t = true, value_name = "decompress")]
    pub decompress: bool,

    /// The directory to pull files into, will be created if it does not exist
    #[arg(short, long, value_name = "target-dir")]
    pub target_dir: Option<PathBuf>,

    /// Maximum number of concurrent downloads
    #[arg(short, long, default_value_t = 5, value_name = "parallelism")]
    pub parallelism: usize,
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// File path or OCI artifact reference (image url) to pro tools session
    #[arg(value_name = "ptx file or oci reference")]
    pub file: String,

    /// Pretty print json
    #[arg(short, long, default_value_t = false)]
    pub pretty: bool,

    #[command(flatten)]
    pub print: InfoPrintArgs,
}

#[derive(Debug, Args)]
#[group(multiple = false)]
pub struct InfoPrintArgs {
    /// Output text
    #[arg(long, action)]
    pub text: bool,

    /// Output table
    #[arg(short, long, action)]
    pub table: bool,

    /// Output json
    #[arg(short, long, action)]
    pub json: bool,
}

#[derive(Subcommand)]
#[command(version, about, long_about = None)]
pub enum Commands {
    /// Push a pro tools session and its audio files to an oci repository as an oras artifact
    Push(PushArgs),
    /// Pull a pro tools session from an oci repository
    Pull(PullArgs),
    /// Print info of a local or remote pro tools session
    Info(InfoArgs),
}
