use crate::compression::Compression;

use std::fmt;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use figment::providers::{Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

static DEFAULT_CONFIG: &str = "~/.ptarchive/config.toml";
static DEFAULT_LOGS: &str = "~/.ptarchive/logs";
static DEFAULT_CACHE: &str = "~/.ptarchive/cache.db";
static DEFAULT_COMPRESSION_LEVEL: i32 = 7;
static DEFAULT_PARALLELISM: usize = 5;

#[derive(Parser, Default, Debug, Serialize, Deserialize)]
#[command(version, about = "Archive Pro Tools session as ORAS artifacts.", long_about = None)]
pub struct Arguments {
    /// Config file location
    #[arg(short, long, default_value = DEFAULT_CONFIG)]
    pub config: PathBuf,

    #[clap(flatten)]
    pub metrics: Option<MetricsOptions>,

    #[clap(flatten)]
    pub logs: Option<Logs>,

    #[clap(flatten)]
    pub cache: Option<Cache>,

    /// Subcommand
    #[command(subcommand)]
    pub command: Commands,
}

impl Arguments {
    pub fn get() -> Result<Arguments, Box<dyn std::error::Error + Send + Sync>> {
        let cli = Arguments::parse();
        let raw_cfg_path = cli.config.to_string_lossy().to_string();
        let cfg_path = shellexpand::tilde(&raw_cfg_path);

        let mut config = Figment::new().merge(Serialized::defaults(cli));

        if std::fs::exists(cfg_path.as_ref())? {
            config = config.merge(Toml::file(cfg_path.as_ref()));
        }

        let mut config = config
            .extract::<Arguments>()
            .inspect_err(|e| eprintln!("config: {e}"))?;

        config.merge();
        Ok(config)
    }

    // merge options from the config file with individual command arguments
    // FIXME: figure out a better way to do this...
    // the problem is that we want both `--json` as a cli argument and
    // log.format = "json" in the config file, which gets tricky with subcommands
    // having the same argument
    pub fn merge(&mut self) {
        let json = self
            .logs
            .as_ref()
            .map(|l| l.format.is_json())
            .unwrap_or(false);
        match &mut self.command {
            Commands::Push(args) => {
                if !json && args.global_opts.json {
                    if let Some(ref mut logs) = self.logs {
                        logs.format = InfoPrintArgs::Json;
                    } else {
                        self.logs = Some(Logs {
                            format: InfoPrintArgs::Json,
                            file: None,
                        })
                    }
                } else {
                    args.global_opts.json = json;
                }
            }
            Commands::Pull(args) => {
                if !json && args.global_opts.json {
                    if let Some(ref mut logs) = self.logs {
                        logs.format = InfoPrintArgs::Json;
                    } else {
                        self.logs = Some(Logs {
                            format: InfoPrintArgs::Json,
                            file: None,
                        })
                    }
                } else {
                    args.global_opts.json = json;
                }
            }
            Commands::Info(args) => {
                if !json && args.global_opts.json {
                    if let Some(ref mut logs) = self.logs {
                        logs.format = InfoPrintArgs::Json;
                    } else {
                        self.logs = Some(Logs {
                            format: InfoPrintArgs::Json,
                            file: None,
                        })
                    }
                } else {
                    args.global_opts.json = json;
                }
            }
        }
    }
}

#[derive(Debug, Default, Args, Serialize, Deserialize)]
pub struct GlobalOpts {
    /// Whether to output json
    #[arg(short, long, action)]
    pub json: bool,
}

#[derive(Clone, Debug, Args, Serialize, Deserialize, Default)]
// TODO: add log level
pub struct Logs {
    #[arg(long = "logs-format", default_value_t = InfoPrintArgs::Text, hide(true))]
    pub format: InfoPrintArgs,

    #[clap(flatten)]
    pub file: Option<LogFile>,
}

impl Logs {
    fn default_directory() -> String {
        shellexpand::tilde(DEFAULT_LOGS).to_string()
    }
}

#[derive(Clone, Debug, Args, Serialize, Deserialize, Default)]
pub struct LogFile {
    #[clap(flatten)]
    pub filename: LogFilename,

    #[arg(long = "logs-file-directory", default_value = DEFAULT_LOGS, hide(true))]
    #[serde(default = "Logs::default_directory")]
    pub directory: String,
}

#[derive(Clone, Debug, Args, Serialize, Deserialize, Default)]
pub struct LogFilename {
    #[arg(long = "logs-filename-method", default_value_t = LogFilenameMethod::Hash, hide(true))]
    pub method: LogFilenameMethod,
}

#[derive(Copy, Clone, clap::ValueEnum, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFilenameMethod {
    #[default]
    Hash,
}

impl fmt::Display for LogFilenameMethod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Self::Hash => "hash",
        };
        write!(f, "{s}")
    }
}

#[derive(Copy, Clone, clap::ValueEnum, Default, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricsFlavor {
    Influxdb,
    #[default]
    None,
}

impl fmt::Display for MetricsFlavor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Self::Influxdb => "influxdb",
            Self::None => "none",
        };
        write!(f, "{s}")
    }
}

#[derive(Clone, Debug, Args, Deserialize, Serialize)]
#[serde(default)]
pub struct Cache {
    #[arg(long = "cache-path", default_value = DEFAULT_CACHE, hide = true)]
    pub path: String,
}

impl Default for Cache {
    fn default() -> Self {
        Cache {
            path: DEFAULT_CACHE.to_string(),
        }
    }
}

#[derive(Clone, Debug, Args, Default, Serialize, Deserialize)]
pub struct MetricsOptions {
    #[arg(long = "metrics-flavor", default_value_t = MetricsFlavor::None, hide = true)]
    pub flavor: MetricsFlavor,

    #[arg(long = "metrics-endpoint", hide = true)]
    pub endpoint: Option<String>,

    #[arg(long = "metrics-database", hide = true)]
    pub database: Option<String>,

    #[arg(long = "metrics-table", hide = true)]
    pub table: Option<String>,

    #[arg(long = "metrics-token", hide = true)]
    pub token: Option<String>,
}

#[derive(Debug, Args, Default, Serialize, Deserialize)]
pub struct CompressionOpts {
    /// Compression algorithm for uploaded files
    #[arg(short, long, default_value_t = Compression::ZSTD, value_name = "compression")]
    #[serde(default)]
    pub compressor: Compression,

    /// Compression level to use
    #[arg(short, long, default_value_t = DEFAULT_COMPRESSION_LEVEL, value_name = "level")]
    #[serde(default = "PushArgs::default_compression_level")]
    pub level: i32,
}

#[derive(Debug, Args, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PushArgs {
    /// OCI repository to push to
    #[arg(value_name = "repository")]
    pub repository: String,

    /// The Pro Tools session to read
    #[arg(value_name = "ptx")]
    pub ptx_file: PathBuf,

    #[command(flatten)]
    pub compression: CompressionOpts,

    /// Maximum number of concurrent uploads
    #[arg(short, long, default_value_t = DEFAULT_PARALLELISM, value_name = "parallelism")]
    #[serde(default = "PushArgs::default_parallelism")]
    pub parallelism: usize,

    /// Discover all files but don't push
    #[arg(long, action)]
    #[serde(skip)]
    pub dry_run: bool,

    #[command(flatten)]
    #[serde(flatten)]
    pub find_args: FindArgs,

    #[command(flatten)]
    #[serde(skip)]
    pub global_opts: GlobalOpts,

    //#[arg(short, long, default_value = DEFAULT_CACHE)]
    //pub cache: String,
    /// Do not cache pushed file digests in sqlite database
    #[arg(long, action)]
    pub no_cache: bool,
}

impl PushArgs {
    fn default_compression_level() -> i32 {
        DEFAULT_COMPRESSION_LEVEL
    }
    fn default_parallelism() -> usize {
        DEFAULT_PARALLELISM
    }
}

#[derive(Debug, Args, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FindArgs {
    /// Ignore audio files not found during search on push.
    /// Use --fail-missing to stop if missing files cannot be found.
    #[arg(short, long, action, value_name = "ignore-missing")]
    pub ignore_missing: bool,

    /// Only find files that have active clips in the session.
    /// When this is set, files that would otherwise trigger
    /// --fail-missing will not stop the push.
    #[arg(short, long, action)]
    pub regions_only: bool,

    /// File paths to search for files in
    #[arg(short, long, default_value = ".", num_args = 1.., value_name = "search-paths")]
    pub search_paths: Vec<String>,

    /// If search_paths is not passed, the maximum folder depth of folders to search for files in
    /// above and below and target session file's folder
    #[arg(short, long, default_value_t = 1, value_name = "depth")]
    pub depth: usize,

    /// Don't match files by filename. Not recommended.
    #[arg(long, action)]
    pub no_filename: bool,

    /// Don't match files by file duration. Not recommended.
    #[arg(long, action)]
    pub no_duration: bool,

    /// Find files by their Pro Tools unique id. Not currently implemented.
    #[arg(short, long, action)]
    pub unique_id: bool,

    /// Fail if missing files are not found
    #[arg(short, long, action)]
    pub fail_missing: bool,
}

#[derive(Debug, Args, Serialize, Deserialize)]
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
    #[arg(short, long, default_value_t = DEFAULT_PARALLELISM, value_name = "parallelism")]
    pub parallelism: usize,

    #[command(flatten)]
    pub global_opts: GlobalOpts,
}

#[derive(Debug, Args, Default, Serialize, Deserialize)]
pub struct InfoArgs {
    /// File path or OCI artifact reference (image url) to pro tools session
    #[arg(value_name = "ptx file or oci reference")]
    pub ptx_file: String,

    /// Pretty print json
    #[arg(short, long, default_value_t = false)]
    pub pretty: bool,

    #[arg(short, long, default_value_t = InfoPrintArgs::Text, value_name = "format")]
    pub format: InfoPrintArgs,

    #[command(flatten)]
    pub global_opts: GlobalOpts,
}

#[derive(Copy, Clone, clap::ValueEnum, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InfoPrintArgs {
    /// Output text
    #[default]
    Text,
    /// Output table
    Table,
    /// Output json
    Json,
}

impl InfoPrintArgs {
    pub fn is_json(&self) -> bool {
        *self == InfoPrintArgs::Json
    }
}

impl fmt::Display for InfoPrintArgs {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            InfoPrintArgs::Text => "text",
            InfoPrintArgs::Table => "table",
            InfoPrintArgs::Json => "json",
        };
        write!(f, "{s}")
    }
}

#[derive(Subcommand, Debug, Serialize, Deserialize)]
#[command(version, about, long_about = None)]
#[serde(rename_all = "lowercase")]
pub enum Commands {
    /// Push a pro tools session and its audio files to an oci repository as an oras artifact
    Push(PushArgs),
    /// Pull a pro tools session from an oci repository
    Pull(PullArgs),
    /// Print info of a local or remote pro tools session
    Info(InfoArgs),
}

impl Default for Commands {
    fn default() -> Self {
        Commands::Info(InfoArgs::default())
    }
}
