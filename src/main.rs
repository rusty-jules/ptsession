mod cli;
use clap::Parser;
use cli::*;
use oci_client::Reference;
use ptsession::PtSession;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::init();
    let cli = Arguments::parse();

    match cli.command {
        Commands::Push(args) => {
            cli::oras_push(
                Reference::from_str(&args.repository)?,
                PtSession::from(&args.ptx_file),
                args,
            )
            .await?
        }
        Commands::Pull(args) => {}
        Commands::Info(args) => cli::info(args).await?,
    }

    Ok(())
}
