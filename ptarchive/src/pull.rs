use crate::annotations::*;
use crate::args::PullArgs;
use crate::client::HttpClient;
use crate::compression::Compression;
use crate::style::{DOWNLOAD_STYLE, FAILED_STYLE, FINISH_STYLE};

use std::convert::TryFrom;
use std::fs::FileTimes;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::DateTime;
use futures_util::{stream, StreamExt, TryStreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use oci_client::annotations::ORG_OPENCONTAINERS_IMAGE_CREATED;
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
    compression: Compression,
    _unpack: bool,
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

    // set the file created time
    let file = file.into_std().await;
    let now = SystemTime::now();
    let annotations = layer
        .annotations
        .as_ref()
        .expect("layer annotations expected");
    let created: SystemTime = annotations
        .get(ORG_OPENCONTAINERS_IMAGE_CREATED)
        .ok_or("org.opencontainers.image.created annotation expected")
        .map_or(now, |created| {
            DateTime::parse_from_rfc3339(created)
                .expect("parse org.opencontainers.image.created as rfc3339")
                .into()
        });
    let modified: Option<SystemTime> =
        annotations.get(IO_PTSESSION_TIME_MODIFIED).map(|modified| {
            DateTime::parse_from_rfc3339(modified)
                .expect("parse io.ptsession.time.modified as rfc3339")
                .into()
        });

    let filetimes = if cfg!(target_os = "macos") {
        use std::os::macos::fs::FileTimesExt;
        FileTimes::new()
            .set_created(created)
            .set_modified(modified.unwrap_or(now))
    } else if cfg!(target_os = "linux") {
        FileTimes::new().set_modified(modified.unwrap_or(created))
    } else if cfg!(target_os = "windows") {
        unimplemented!();
    } else {
        unimplemented!()
    };

    file.set_times(filetimes)?;

    download_progress.set_style(ProgressStyle::clone(&*FINISH_STYLE));
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
    args: &PullArgs,
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

    // add config as a layer to pull
    let mut layers = manifest.layers;
    layers.push(manifest.config);
    let _: Vec<()> = stream::iter(layers)
        .map(|layer| {
            // clone inputs
            let client = client.clone();
            let reference = reference.clone();
            let target_dir = args.target_dir.clone();
            let multi_progress = multi_progress.clone();

            // parse layer annotations
            let file_name = layer
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.get(ORG_OPENCONTAINERS_IMAGE_TITLE))
                .expect("layer has title")
                .clone();
            let unpack: bool = layer
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.get(IO_DEIS_ORAS_CONTENT_UNPACK))
                .and_then(|unpack| unpack.parse().ok())
                .unwrap_or(false);

            // parse layer config
            let size = layer.size as u64;
            let compression = Compression::try_from(MediaType(layer.media_type.as_str()));

            // start download
            async move {
                let download_progress = multi_progress.add(ProgressBar::new(0));

                if let Err(err) = compression {
                    download_progress.set_style(ProgressStyle::clone(&*FAILED_STYLE));
                    download_progress.finish_with_message(format!(
                        "Unknown compression type for {file_name}: {}",
                        err.to_string()
                    ));
                    return Err(err);
                }

                download_progress.set_style(ProgressStyle::clone(&*DOWNLOAD_STYLE));
                download_progress.set_length(size);

                pull_and_decompress(
                    &client,
                    &reference,
                    &layer,
                    &file_name,
                    target_dir,
                    download_progress,
                    compression.unwrap(),
                    unpack,
                )
                .await
            }
        })
        .buffer_unordered(parallelism)
        .try_collect()
        .await?;

    Ok(())
}
