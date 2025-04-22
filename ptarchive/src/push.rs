use crate::annotations::*;
use crate::args::PushArgs;
use crate::client::HttpClient;
use crate::compression::Compression;
use crate::find::find_files;
use crate::meta::file_meta;
use crate::metrics::{BYTES_READ, BYTES_SENT, UPLOAD_SPEED};
use crate::style::{COMPRESSION_STYLE, UPLOAD_STYLE};

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_compression::Level;
use futures_util::{future, Stream, StreamExt, TryStreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use metrics::{register_counter, register_histogram, Counter};
use oci_client::{
    annotations::{ORG_OPENCONTAINERS_IMAGE_CREATED, ORG_OPENCONTAINERS_IMAGE_TITLE},
    client::{ClientConfig, Config, PushResponse},
    errors::OciDistributionError,
    manifest::{OciDescriptor, OciImageManifest, OCI_IMAGE_MEDIA_TYPE},
    secrets::RegistryAuth,
    Reference,
};
use ptsession::{session::Wav, PtSession};
use sha2::{Digest as _, Sha256};
use tempfile::tempfile;
use tokio::fs::File;
use tokio::io::{AsyncBufRead, AsyncRead, AsyncReadExt, BufReader, ReadBuf};
use tokio_util::bytes::Bytes;
use tokio_util::io::ReaderStream;
use tracing::{debug, info, warn, Instrument};

//const BUF_CAPACITY: usize = 64 * 1024; // 64KB

// Stream that calculates digests while reading
struct DigestReader<R>
where
    R: AsyncRead + Unpin,
{
    inner: R,
    hasher: Arc<Mutex<Sha256>>,
    progress: Option<ProgressBar>,
    metric: Option<Counter>,
    size: Arc<AtomicUsize>,
}

impl<R: AsyncRead + Unpin> DigestReader<R> {
    fn new(inner: R, progress: Option<ProgressBar>, metric: Option<Counter>) -> Self {
        Self {
            inner,
            hasher: Arc::new(Mutex::new(Sha256::new())),
            progress,
            metric,
            size: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn digest_handle(&self) -> Arc<Mutex<Sha256>> {
        self.hasher.clone()
    }

    fn size_tracker(&self) -> Arc<AtomicUsize> {
        self.size.clone()
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for DigestReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let original_filled = buf.filled().len();

        // Read from inner
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let new_filled = buf.filled().len();
                let bytes_read = new_filled - original_filled;

                if bytes_read > 0 {
                    // Update digest
                    let mut hasher = self.hasher.lock().unwrap();
                    hasher.update(&buf.filled()[original_filled..]);

                    // Update progress if provided
                    if let Some(progress) = &self.progress {
                        progress.inc(bytes_read as u64);
                    }
                    if let Some(counter) = &self.metric {
                        counter.increment(bytes_read as u64);
                    }

                    // Update size counter
                    self.size.fetch_add(bytes_read, Ordering::SeqCst);
                }

                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl<R: AsyncRead + AsyncBufRead + Unpin> AsyncBufRead for DigestReader<R> {
    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        Pin::new(&mut self.inner).consume(amt)
    }

    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<&[u8]>> {
        let this = self.get_mut();
        Pin::new(&mut this.inner).poll_fill_buf(cx)
    }
}

async fn begin_push_chunked_session(
    client: &HttpClient,
    image: &Reference,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Create POST request to start upload session
    let url = format!(
        "{}://{}/v2/{}/blobs/uploads/",
        "http", // Assuming HTTP
        image.resolve_registry(),
        image.repository()
    );

    let res = client
        .post(&url)
        .await
        .header("Content-Length", 0)
        .send()
        .await?;

    if res.status() == reqwest::StatusCode::ACCEPTED {
        // Extract location header
        let location_header = res
            .headers()
            .get("Location")
            .ok_or_else(|| "No Location header in response".to_string())?;

        let location = location_header.to_str()?;

        // Make sure location is absolute URL
        if location.starts_with("http") {
            Ok(location.to_string())
        } else {
            // Convert relative URL to absolute
            let registry = image.resolve_registry();
            Ok(format!("http://{}{}", registry, location))
        }
    } else {
        Err(format!(
            "Failed to begin upload session: {} - {}",
            res.status(),
            res.text().await?
        )
        .into())
    }
}

async fn push_stream<S>(
    client: &HttpClient,
    location: &str,
    image: &Reference,
    stream: S,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    let res = client
        .patch(location)
        .await
        .header("Content-Type", "application/octet-stream")
        .header("Transfer-Encoding", "chunked")
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await?;

    if res.status() == reqwest::StatusCode::ACCEPTED {
        let location_header = res
            .headers()
            .get("Location")
            .ok_or_else(|| "No Location header in response".to_string())?;

        let location = location_header.to_str()?;

        // Make sure location is absolute URL
        let new_location = if location.starts_with("http") {
            location.to_string()
        } else {
            // Convert relative URL to absolute
            let registry = image.resolve_registry();
            format!("http://{}{}", registry, location)
        };

        Ok(new_location)
    } else {
        Err(format!(
            "Failed to push chunk: {} - {}",
            res.status(),
            res.text().await?
        )
        .into())
    }
}

async fn end_push_chunked_session(
    client: &HttpClient,
    location: &str,
    image: &Reference,
    digest: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let url = reqwest::Url::parse_with_params(location, &[("digest", digest)])?;

    let res = client
        .put(url.clone())
        .await
        .header("Content-Length", 0)
        .send()
        .await?;

    if res.status() == reqwest::StatusCode::CREATED {
        let location_header = res
            .headers()
            .get("Location")
            .ok_or_else(|| "No Location header in response".to_string())?;

        let location = location_header.to_str()?;

        // Make sure location is absolute URL
        if location.starts_with("http") {
            Ok(location.to_string())
        } else {
            // Convert relative URL to absolute
            let registry = image.resolve_registry();
            Ok(format!("http://{}{}", registry, location))
        }
    } else {
        Err(format!(
            "Failed to end upload session: {} - {}",
            res.status(),
            res.text().await?
        )
        .into())
    }
}

async fn compress_and_upload(
    client: &oci_client::Client,
    reference: &Reference,
    file_name: String,
    file_path: &Path,
    compression: Compression,
    level: i32,
    compression_progress: ProgressBar,
    upload_progress: ProgressBar,
    pushed_digests: BTreeMap<String, OciDescriptor>,
    dry_run: bool,
) -> Result<OciDescriptor, Box<dyn std::error::Error + Send + Sync>> {
    // Create a shared http client wrapper for this operation
    let http_client = HttpClient::new(client);
    // Open the file
    let file = File::open(file_path).await?;

    // Get file meta
    let (file, file_size, file_created, file_modified) = file_meta(file).await?;
    compression_progress.set_length(file_size);
    let original_counter = register_counter!(BYTES_READ);

    // Create digest reader to calculate original file digest
    let mut digest_reader = if !dry_run {
        DigestReader::new(file, None, Some(original_counter))
    } else {
        DigestReader::new(File::from_std(tempfile()?), None, Some(original_counter))
    };
    let original_digest_hasher = digest_reader.digest_handle();

    // Read the file into memory and compute the digest
    let mut buf = Vec::with_capacity(file_size as usize);
    digest_reader.read_to_end(&mut buf).await?;
    let original_digest = original_digest_hasher.lock().unwrap().clone().finalize();
    if let Some(descriptor) = pushed_digests.get(&format!("sha256:{:x}", original_digest)) {
        compression_progress.finish_with_message(format!("Exists {file_name}"));
        upload_progress.finish_with_message(format!("Exists {file_name}"));
        info!("file already exists");
        return Ok(descriptor.clone());
    }

    // Create compression encoder
    let encoder = compression.compressor(
        BufReader::new(Cursor::new(buf)),
        Some(Level::Precise(level)),
    );

    // Register counter only after we've determined if the file will be uploaded
    let compressed_counter = register_counter!(BYTES_SENT);
    let upload_speed = register_histogram!(UPLOAD_SPEED);

    // Create digest reader to calculate compressed file digest
    let compressed_digest_reader = DigestReader::new(
        encoder,
        Some(compression_progress),
        Some(compressed_counter),
    );
    let compressed_digest_hasher = compressed_digest_reader.digest_handle();
    let compressed_size_tracker = compressed_digest_reader.size_tracker();

    // Clone progress items for stream op
    let progress_stream = upload_progress.clone();
    let progress_size_tracker = compressed_size_tracker.clone();

    // Create stream
    let start = std::time::Instant::now();
    let chunk_stream = ReaderStream::new(compressed_digest_reader).and_then(move |bytes| {
        let size = progress_size_tracker.load(Ordering::SeqCst) as u64;
        progress_stream.set_length(size);
        progress_stream.inc(bytes.len() as u64);

        let elapsed = start.elapsed().as_secs_f64();
        let megabytes = size as f64 / 1_000_000.0;
        upload_speed.record(megabytes / elapsed);

        futures_util::future::ok(bytes)
    });

    // Start the upload session
    let location = if !dry_run {
        begin_push_chunked_session(&http_client, reference).await?
    } else {
        "".to_string()
    };

    // Set up upload progress bar
    upload_progress.set_message(format!("Uploading {file_name}"));
    info!("uploading file");

    if !dry_run {
        push_stream(&http_client, &location, &reference, chunk_stream).await?;
    } else {
        chunk_stream.for_each(|_chunk| future::ready(())).await;
    }

    // Get final compressed digest and size
    let original_digest = format!(
        "sha256:{:x}",
        original_digest_hasher.lock().unwrap().clone().finalize()
    );
    let compressed_digest = format!(
        "sha256:{:x}",
        compressed_digest_hasher.lock().unwrap().clone().finalize()
    );
    let compressed_size = compressed_size_tracker.load(Ordering::SeqCst);

    // Set final upload progress bar length and position
    upload_progress.set_length(compressed_size as u64);
    upload_progress.set_position(compressed_size as u64);

    // Finish the upload
    if !dry_run {
        let _blob_url =
            end_push_chunked_session(&http_client, &location, reference, &compressed_digest)
                .await?;
    }

    // Mark progress bars as complete
    upload_progress.finish_with_message(format!("Upload complete {file_name}"));
    info!("upload complete");

    // Output OciDescriptor of the layer
    let mut annotations = maplit::btreemap! {
        ORG_OPENCONTAINERS_IMAGE_TITLE.to_string() => file_name,
        ORG_OPENCONTAINERS_IMAGE_CREATED.to_string() => file_created,
        IO_PTSESSION_ORIGINAL_DIGEST.to_string() => original_digest,
        IO_PTSESSION_ORIGINAL_SIZE.to_string() => file_size.to_string(),
        IO_PTSESSION_COMPRESSION_LEVEL.to_string() => level.to_string(),
        IO_DEIS_ORAS_CONTENT_UNPACK.to_string() => "true".to_string()
    };

    if let Some(modified) = file_modified {
        annotations.insert(IO_PTSESSION_TIME_MODIFIED.to_string(), modified);
    }

    let media_type = match compression {
        Compression::None => "audio/vnd.wav".to_string(),
        _ => format!("audio/vnd.wav+{}", compression.to_string()),
    };

    Ok(OciDescriptor {
        urls: None,
        digest: compressed_digest,
        size: compressed_size as i64,
        media_type,
        annotations: Some(annotations),
    })
}

async fn fetch_original_digests(
    client: &oci_client::Client,
    reference: &Reference,
    auth: &RegistryAuth,
) -> Result<BTreeMap<String, OciDescriptor>, Box<dyn std::error::Error + Send + Sync>> {
    let tags = client.list_tags(&reference, &auth, None, None).await;

    // if the repository doesn't exist return empty map of digests
    if let Err(OciDistributionError::RegistryError { .. }) = tags {
        return Ok(BTreeMap::new());
    } else if let Err(e) = tags {
        return Err(e.into());
    }

    futures_util::stream::iter(tags.unwrap().tags.into_iter())
        .map(|tag| {
            let a = auth.clone();
            let c = client.clone();
            let r = reference.clone();

            async move {
                let reference =
                    Reference::with_tag(r.registry().to_string(), r.repository().to_string(), tag);
                match c.pull_image_manifest(&reference, &a).await {
                    Ok((manifest, _)) => Ok(manifest
                        .layers
                        .into_iter()
                        .filter_map(|layer| match &layer.annotations {
                            None => None,
                            Some(annotations) => {
                                match annotations.get(IO_PTSESSION_ORIGINAL_DIGEST) {
                                    None => None,
                                    Some(digest) => Some((digest.clone(), layer)),
                                }
                            }
                        })
                        .collect::<BTreeMap<String, OciDescriptor>>()),
                    Err(e) => match e {
                        OciDistributionError::ImageManifestNotFoundError(_)
                        | OciDistributionError::RegistryError { .. } => Ok(BTreeMap::new()),
                        e => Err(e.into()),
                    },
                }
            }
        })
        .buffer_unordered(10)
        .try_concat()
        .await
}

async fn push_manifest(
    client: &oci_client::Client,
    reference: &Reference,
    auth: &RegistryAuth,
    layers: Vec<OciDescriptor>,
    session: PtSession,
    ptx_file: PathBuf,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let ptx = File::open(&ptx_file).await?;
    let (mut ptx, ptx_size, ptx_created, ptx_modified) = file_meta(ptx).await?;
    let mut ptx_data = Vec::with_capacity(ptx_size as usize);
    ptx.read_to_end(&mut ptx_data).await?;
    drop(ptx);

    let ptx_filename = ptx_file.file_name().unwrap().to_string_lossy().to_string();

    let mut annotations = maplit::btreemap! {
        IO_PTSESSION_SAMPLE_RATE.to_string() => session.session_sample_rate.to_string(),
        ORG_OPENCONTAINERS_IMAGE_TITLE.to_string() => ptx_filename.clone(),
        ORG_OPENCONTAINERS_IMAGE_CREATED.to_string() => ptx_created,
    };

    if let Some(modified) = ptx_modified {
        annotations.insert(IO_PTSESSION_TIME_MODIFIED.to_string(), modified);
    }

    let config = Config {
        data: ptx_data,
        media_type: "application/vnd.avid.ptx".to_string(),
        annotations: Some(annotations),
    };
    let mut metrics_labels = [
        ("file", ptx_filename.clone()),
        ("repository", reference.repository().to_string()),
        ("tag", reference.tag().unwrap().to_string()),
    ];
    register_counter!(BYTES_READ, &metrics_labels).absolute(ptx_size);

    let manifest = OciImageManifest {
        schema_version: 2,
        media_type: Some(OCI_IMAGE_MEDIA_TYPE.to_string()),
        artifact_type: Some("application/vnd.avid.ptsession".to_string()),
        config: OciDescriptor {
            size: ptx_size as i64,
            media_type: config.media_type.clone(),
            digest: config.sha256_digest(),
            annotations: config.annotations.clone(),
            ..Default::default()
        },
        layers,
        annotations: Some(maplit::btreemap! {
            ORG_OPENCONTAINERS_IMAGE_TITLE.to_string() => ptx_filename,
            ORG_OPENCONTAINERS_IMAGE_CREATED.to_string() => chrono::Local::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }),
    };

    if !dry_run {
        debug!("pushing manifest...");
        let res: PushResponse = client
            .push(reference, &[], config, auth, Some(manifest))
            .await?;

        // FIXME: update this if we start compressing ptx files
        register_counter!(BYTES_SENT, &metrics_labels).absolute(ptx_size);

        // get the size and digest of the manifest
        let manifest_head = reqwest::Client::new()
            .request(reqwest::Method::HEAD, &res.manifest_url)
            .send()
            .await;

        match manifest_head {
            Ok(manifest_res) => {
                let headers = manifest_res.headers();
                let length = headers
                    .get("Content-Length")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());
                let digest = headers
                    .get("Docker-Content-Digest")
                    .and_then(|h| h.to_str().ok());
                match (length, digest) {
                    (Some(len), Some(digest)) => {
                        for (tag, val) in metrics_labels.iter_mut() {
                            if *tag == "file" {
                                *val = format!("{}@{digest}", reference.repository())
                            }
                        }
                        register_counter!(BYTES_READ, &metrics_labels).absolute(len);
                        register_counter!(BYTES_SENT, &metrics_labels).absolute(len);
                    }
                    (None, _) => warn!("manifest head request did not include Content-Length"),
                    (_, None) => {
                        warn!("manifest head request did not include Docker-Content-Digest")
                    }
                }
            }
            Err(e) => warn!("failed to head manifest: {e}"),
        }

        info!(file = res.manifest_url, "push complete");
    } else {
        info!("dry run: not pushing manifest");
    }
    Ok(())
}

pub async fn push(
    reference: Reference,
    session: PtSession,
    PushArgs {
        compression,
        level,
        parallelism,
        ptx_file,
        dry_run,
        find_args,
        ..
    }: PushArgs,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (file_names, missing_files): (Vec<String>, Vec<String>) = session
        .audio_files
        .iter()
        .map(|Wav { file_name, .. }: &Wav| format!("Audio Files/{file_name}"))
        .partition(|file_name| {
            if !std::fs::exists(file_name).unwrap() {
                false
            } else {
                true
            }
        });

    let all_files: Vec<(String, PathBuf)> = file_names
        .into_iter()
        .map(|file_name| (file_name.clone(), PathBuf::from(file_name)))
        .chain(
            find_files(&session, missing_files, parallelism, find_args)
                .await?
                .into_iter(),
        )
        .collect();

    let client = oci_client::Client::new(ClientConfig {
        protocol: oci_client::client::ClientProtocol::Http,
        ..Default::default()
    });
    let auth = RegistryAuth::Anonymous;

    // Create a multi-progress bar
    let multi_progress = MultiProgress::new();

    // fetch all original digests
    let original_digests = fetch_original_digests(&client, &reference, &auth).await?;

    // Process all files in parallel with progress reporting
    debug!("processing audio files with parallelism: {}", parallelism);

    let layers: Vec<OciDescriptor> = futures_util::stream::iter(all_files)
        .map(|(file_name, file_path)| {
            let client = client.clone();
            let reference = reference.clone();
            let multi_progress = multi_progress.clone();
            let original_digests = original_digests.clone();

            async move {
                // Create progress bars for this file
                let compression_progress = multi_progress.add(ProgressBar::new(0));
                compression_progress.set_style(ProgressStyle::clone(&*COMPRESSION_STYLE));
                compression_progress.set_message(format!("Compressing {}", file_name));

                let upload_progress = multi_progress.add(ProgressBar::new(0));
                upload_progress.set_style(ProgressStyle::clone(&*UPLOAD_STYLE));
                upload_progress.set_message(format!("Uploading {}", file_name));

                let span = tracing::info_span!(
                    "upload",
                    file = file_path.file_name().unwrap().to_str(),
                    path = file_path.to_str(),
                    compression.type = compression.to_string(),
                    compression.level = level,
                    registry = reference.registry(),
                    repository = reference.repository(),
                    tag = reference.tag(),
                );

                // Process the file
                let result = compress_and_upload(
                    &client,
                    &reference,
                    file_name,
                    &file_path,
                    compression,
                    level,
                    compression_progress,
                    upload_progress,
                    original_digests,
                    dry_run,
                )
                .instrument(span)
                .await?;

                Ok::<_, Box<dyn std::error::Error + Sync + Send>>(result)
            }
        })
        .buffer_unordered(parallelism)
        .try_collect()
        .await?;

    debug!("creating manifest for {} layers", layers.len());
    push_manifest(
        &client, &reference, &auth, layers, session, ptx_file, dry_run,
    )
    .await?;

    Ok(())
}
