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
use figment::providers::{Format, Serialized, Toml};
use figment::Figment;
use oci_client::Reference;
use ptsession::PtSession;
use tracing::{error, info, info_span, Instrument};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Arguments::parse();
    let cfg_path = cli.config.to_string_lossy().to_string();

    let config = Figment::new()
        .merge(Serialized::defaults(cli))
        .merge(Toml::file(shellexpand::tilde(&cfg_path).as_ref()))
        .extract::<Arguments>()
        .inspect_err(|e| eprintln!("config: {e}"))?;

    match &config.command {
        Commands::Push(args) => {
            metrics::init(
                &config,
                Some(args.ptx_file.to_string_lossy().to_string()),
                &args.global_opts,
            );
            let session = args.ptx_file.to_str();
            let reference =
                Reference::from_str(&args.repository).inspect_err(|e| error!(session, "{e}"))?;
            let VolumeInfo {
                mount_name,
                device_serial,
            } = metrics::get_volume_info(&args.ptx_file).inspect_err(|e| error!(session, "{e}"))?;
            let span = info_span!(
                "push",
                hdd.name = mount_name,
                hdd.serial = device_serial,
                session,
                compression.type = %args.compression.compressor,
                compression.level = args.compression.level,
                registry = reference.registry(),
                repository = reference.repository(),
                tag = reference.tag(),
                path = args.ptx_file.canonicalize()?.to_str(),
            );
            info!("ptarchive push start");
            match args.global_opts.json {
                true => {
                    push(reference, PtSession::from(&args.ptx_file), args)
                        .instrument(span)
                        .await?;
                    if config.metrics.is_some() {
                        // allow metrics to flush
                        std::thread::sleep(std::time::Duration::from_millis(
                            metrics::EXPORT_MILLIS + 100,
                        ));
                    }
                }
                false => push(reference, PtSession::from(&args.ptx_file), args).await?,
            }
        }
        Commands::Pull(args) => {
            metrics::init(&config, None, &args.global_opts);
            pull(Reference::from_str(&args.repository)?, args).await?;
        }
        Commands::Info(args) => {
            metrics::init(&config, Some(args.ptx_file.clone()), &args.global_opts);
            info(args).await?;
        }
    }

    Ok(())
}
