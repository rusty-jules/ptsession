use crate::annotations::*;
use crate::args::PullArgs;
use crate::client::HttpClient;
use crate::compression::Compression;

use std::convert::TryFrom;
use std::fmt::Write;
use std::path::PathBuf;
use std::str::FromStr;

use futures_util::{stream, StreamExt, TryStreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressState, ProgressStyle};
use oci_client::manifest::OciDescriptor;
use oci_client::{
    annotations::ORG_OPENCONTAINERS_IMAGE_TITLE,
    client::{ClientConfig, ClientProtocol},
    manifest::OciImageManifest,
    secrets::RegistryAuth,
    Client, Reference,
};
use tokio::io::AsyncWriteExt;
use tokio_util::io::StreamReader;

async fn pull_and_decompress(
    client: &HttpClient,
    reference: &Reference,
    layer: &OciDescriptor,
    file_name: &String,
    target_dir: Option<PathBuf>,
    download_progress: ProgressBar,
    finish_style: ProgressStyle,
    compression: Compression,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // setup target file
    let mut file_path = target_dir.unwrap_or(PathBuf::new());
    file_path.push(PathBuf::from(file_name));
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::File::create_new(file_path).await?;

    // start the download
    download_progress.set_message(file_name.clone());
    let progress = download_progress.clone();
    let download_stream = StreamReader::new(
        client
            .pull_blob_stream(reference, layer)
            .await?
            .stream
            .and_then(move |bytes| {
                if bytes.len() > 0 {
                    progress.inc(bytes.len() as u64);
                }
                futures_util::future::ok(bytes)
            }),
    );

    let mut decoder = compression.decompressor(download_stream);

    // write stream to file
    let _ = tokio::io::copy(&mut decoder, &mut file).await?;
    file.flush().await?;
    download_progress.set_style(finish_style);
    download_progress.finish();

    Ok(())
}

fn validate_manifest(
    manifest: &OciImageManifest,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if manifest.artifact_type.is_none()
        || manifest.artifact_type.as_ref().unwrap() != "application/vnd.avid.ptsession"
    {
        Err("artifactType is not application/vnd.avid.ptsession, refusing to pull")?
    }

    Ok(())
}

pub async fn pull(
    reference: Reference,
    args: PullArgs,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let auth = RegistryAuth::Anonymous;
    let client = Client::new(ClientConfig {
        protocol: ClientProtocol::Http,
        ..Default::default()
    });

    let (manifest, _) = client.pull_image_manifest(&reference, &auth).await?;
    validate_manifest(&manifest)?;

    let parallelism = args.parallelism;
    let client = HttpClient::new(&client);

    let multi_progress = MultiProgress::new();

    let download_style = ProgressStyle::with_template(
        "[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes:>12}/{total_bytes:<12} {check:.yellow} {msg} ({eta})",
    )
    .expect("correct progress style")
    .with_key("eta", |state: &ProgressState, w: &mut dyn Write| {
        write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap()
    })
    .with_key("check", |state: &indicatif::ProgressState, w: &mut dyn Write| {
        let icon = if state.fraction() >= 1.0 {
            "✓"
        } else {
            match state.elapsed().as_millis() as u64 % 4 {
                0 => "⠋",
                1 => "⠙",
                2 => "⠹",
                _ => "⠸",
            }
        };
        write!(w, "{icon}").unwrap()
    })
    .progress_chars("#>-");

    let finish_style = ProgressStyle::with_template(
        "[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes:>12}/{total_bytes:<12} {check:.green} {msg}",
    )
    .expect("correct progress style")
    .with_key("check", |state: &indicatif::ProgressState, w: &mut dyn Write| {
        let icon = if state.fraction() >= 1.0 {
            "✓"
        } else {
            match state.elapsed().as_millis() as u64 % 4 {
                0 => "⠋",
                1 => "⠙",
                2 => "⠹",
                _ => "⠸",
            }
        };
        write!(w, "{icon}").unwrap()
    })
    .progress_chars("#>-");

    let failed_style = ProgressStyle::with_template(
        "[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes:>12}/{total_bytes:<12} {check:.red} {msg}",
    )
    .expect("correct progress style")
    .with_key(
        "check",
        |_state: &indicatif::ProgressState, w: &mut dyn Write| write!(w, "✖︎").unwrap(),
    )
    .progress_chars("#>-");

    // add config as a layer to pull
    let mut layers = manifest.layers;
    layers.push(manifest.config);
    let _: Vec<()> = stream::iter(layers)
        .map(|mut layer| {
            // clone inputs
            let client = client.clone();
            let reference = reference.clone();
            let target_dir = args.target_dir.clone();
            let multi_progress = multi_progress.clone();
            let download_style = download_style.clone();
            let finish_style = finish_style.clone();
            let failed_style = failed_style.clone();

            // parse layer annotations
            let mut annotations = layer.annotations.take().expect("layer as annotations");
            let file_name = annotations
                .remove(ORG_OPENCONTAINERS_IMAGE_TITLE)
                .expect("layer as title");
            let unpack: Result<bool, <bool as FromStr>::Err> = annotations
                .remove(IO_DEIS_ORAS_CONTENT_UNPACK)
                .unwrap_or("false".to_string())
                .parse();

            // parse layer config
            let size = layer.size as u64;
            let compression = Compression::try_from(MediaType(layer.media_type.as_str()));

            // start download
            async move {
                let download_progress = multi_progress.add(ProgressBar::new(0));

                if unpack.is_err() {
                    let err = unpack.unwrap_err();
                    download_progress.set_style(failed_style);
                    download_progress.finish_with_message(format!(
                        "Unknown unpack annotation value for {file_name}: {}",
                        err.to_string()
                    ));
                    return Err(err.into());
                }

                if compression.is_err() {
                    let err = compression.unwrap_err();
                    download_progress.set_style(failed_style);
                    download_progress.finish_with_message(format!(
                        "Unknown compression type for {file_name}: {}",
                        err.to_string()
                    ));
                    return Err(err);
                }

                download_progress.set_style(download_style);
                download_progress.set_length(size);

                let result = pull_and_decompress(
                    &client,
                    &reference,
                    &layer,
                    &file_name,
                    target_dir,
                    download_progress,
                    finish_style,
                    compression.unwrap(),
                )
                .await?;

                Ok::<_, Box<dyn std::error::Error + Sync + Send>>(result)
            }
        })
        .buffer_unordered(parallelism)
        .try_collect()
        .await?;

    Ok(())
}
