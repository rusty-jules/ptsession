use crate::args::InfoArgs;

use std::path::PathBuf;

use indicatif::HumanBytes;
use oci_client::{
    client::{Client, ClientConfig, ClientProtocol},
    secrets::RegistryAuth,
    Reference,
};
use ptsession::PtSession;
use tokio::fs::File;

fn print_text(ptsession: PtSession) {
    println!("Version: {}", ptsession.version);
    println!("Sample Rate: {}", ptsession.session_sample_rate);
    println!("Audio Files: {}", ptsession.audio_files.len());
    let total_size = ptsession
        .audio_files
        .iter()
        .map(|wav| {
            let mut path = PathBuf::from("Audio Files");
            path.push(&wav.file_name);
            std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
        })
        .fold(0, |acc, size| acc + size);
    println!("Total Size: {}", HumanBytes(total_size as u64));
    println!("File Names:");
    ptsession
        .audio_files
        .iter()
        .map(|wav| wav.file_name.as_str())
        .for_each(|name| println!("  {name}"));
}

fn print_ptx(
    ptsession: PtSession,
    args: &InfoArgs,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if args.print.text {
        print_text(ptsession);
    } else if args.print.json {
        let json = if args.pretty {
            serde_json::to_string_pretty(&ptsession)?
        } else {
            serde_json::to_string(&ptsession)?
        };
        println!("{json}");
    } else if args.print.table {
        println!("not supported");
    } else {
        print_text(ptsession);
    }
    Ok(())
}

pub async fn fetch_ptx(
    reference: &Reference,
    _args: &InfoArgs,
) -> Result<PtSession, Box<dyn std::error::Error + Send + Sync>> {
    let client = Client::new(ClientConfig {
        protocol: ClientProtocol::Http,
        ..Default::default()
    });
    let auth = RegistryAuth::Anonymous;

    // TODO: use pull_manifest and get the first arch
    // for now we just get the first because arch doesn't apply to ptx files
    let (manifest, _) = client.pull_image_manifest(&reference, &auth).await?;
    let config_descriptor = manifest.config;

    if config_descriptor.media_type != "application/vnd.avid.ptx" {
        eprintln!(
            "Warning: expected config file mediaType application/vnd.avid.ptx, but found {}",
            config_descriptor.media_type
        );
        eprintln!("Session may not be parsed correctly")
    }

    let (file, path) = tempfile::NamedTempFile::new()?.keep()?;
    let mut ptx = File::from(file);
    eprintln!("Pulling session");
    client
        .pull_blob(&reference, &config_descriptor, &mut ptx)
        .await?;
    let session = PtSession::from(&path);
    tokio::fs::remove_file(path).await?;
    Ok(session)
}

pub async fn info(args: InfoArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ptx_session = if let Ok(reference) = args.file.parse::<Reference>() {
        fetch_ptx(&reference, &args).await?
    } else {
        PtSession::from(PathBuf::from(&args.file))
    };

    print_ptx(ptx_session, &args)
}
