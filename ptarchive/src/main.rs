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
use crate::metrics::VolumeInfo;
use crate::pull::*;
use crate::push::*;

use std::str::FromStr;

use clap::Parser;
use oci_client::Reference;
use ptsession::PtSession;
use tracing::Instrument;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match Arguments::parse().command {
        Commands::Push(args) => {
            metrics::init(&args.global_opts);
            let VolumeInfo {
                mount_name,
                device_serial,
            } = metrics::get_volume_info(&args.ptx_file)?;
            let span = tracing::info_span!(
                "push",
                hdd.name = mount_name,
                hdd.serial = device_serial,
                session = args.ptx_file.to_str()
            );
            let json = args.global_opts.json;
            let cmd = push(
                Reference::from_str(&args.repository)?,
                PtSession::from(&args.ptx_file),
                args,
            );
            if json {
                cmd.instrument(span).await?
            } else {
                cmd.await?
            }

            // allow metrics to flush
            std::thread::sleep(std::time::Duration::from_millis(
                metrics::EXPORT_MILLIS + 100,
            ));
        }
        Commands::Pull(args) => {
            metrics::init(&args.global_opts);
            pull(Reference::from_str(&args.repository)?, args).await?;
        }
        Commands::Info(args) => {
            metrics::init(&args.global_opts);
            info(args).await?;
        }
    }

    Ok(())
}
