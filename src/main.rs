use clap::Parser;
use ptsession::PtSession;
use serde_json;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Whether to output json
    #[arg(short, long, value_name = "json")]
    json: Option<bool>,

    /// The Pro Tools Session to Read
    #[arg(value_name = "ptx")]
    ptx_file: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let cli = Args::parse();

    let session = PtSession::from(cli.ptx_file);
    println!("{}", serde_json::to_string(&session)?);

    Ok(())
}
