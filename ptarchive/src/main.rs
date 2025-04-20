mod annotations;
mod args;
mod client;
mod compression;
mod find;
mod info;
mod meta;
mod metrics;
mod pull;
mod push;
mod style;

use crate::args::*;
use crate::info::*;
use crate::pull::*;
use crate::push::*;

use clap::Parser;
use oci_client::Reference;
use ptsession::PtSession;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::init();
    let cli = Arguments::parse();

    match cli.command {
        Commands::Push(args) => {
            push(
                Reference::from_str(&args.repository)?,
                PtSession::from(&args.ptx_file),
                args,
            )
            .await?
        }
        Commands::Pull(args) => pull(Reference::from_str(&args.repository)?, args).await?,
        Commands::Info(args) => info(args).await?,
    }

    Ok(())
}
