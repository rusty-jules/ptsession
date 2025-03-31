mod cli;
use clap::Parser;
use cli::*;
use oci_client::Reference;
use ptsession::PtSession;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    env_logger::init();
    let cli = Arguments::parse();

    match cli.command {
        Some(command) => match command {
            Commands::Push(args) => {
                let (reference, session) = parse_args(&args.repository, &args.ptx_file)?;
                cli::oras_push(reference, session, args).await?;
            }
            Commands::Pull { repository, .. } => {
                //let (reference, session) = parse_args(repository, cli.ptx_file)?;
                //cli::oras_pull(reference, session).await?;
            }
            Commands::Info { repository } => {
                //let (reference, session) = parse_args(repository, cli.ptx_file)?;
                //cli::oras_info(reference, session).await?;
            }
        },
        None => {
            //let session = PtSession::from(cli.ptx_file);
            //println!("{}", serde_json::to_string(&session)?);
        }
    }

    Ok(())
}

fn parse_args(
    repository: &String,
    ptx_file: &PathBuf,
) -> Result<(Reference, PtSession), Box<dyn std::error::Error + Send + Sync>> {
    let reference: Reference = repository.parse()?;
    let session = PtSession::from(ptx_file);
    Ok((reference, session))
}
