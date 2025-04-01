use std::path::PathBuf;

use oci_client::{
    client::{Client, ClientConfig, ClientProtocol},
    secrets::RegistryAuth,
    Reference,
};
use ptsession::PtSession;
use tokio::fs::File;

use super::InfoArgs;

fn print_ptx(
    ptsession: PtSession,
    args: &InfoArgs,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let json = if args.pretty {
        serde_json::to_string_pretty(&ptsession)?
    } else {
        serde_json::to_string(&ptsession)?
    };
    println!("{json}");
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
