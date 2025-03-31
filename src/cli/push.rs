use super::{Compression, PushArgs};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_compression::tokio::write::XzEncoder;
use futures_util::{StreamExt, TryStreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use oci_client::manifest::OciDescriptor;
use oci_client::{
    annotations::{ORG_OPENCONTAINERS_IMAGE_CREATED, ORG_OPENCONTAINERS_IMAGE_TITLE},
    client::{ClientConfig, Config, PushResponse},
    manifest::{OciImageManifest, OCI_IMAGE_MEDIA_TYPE},
    secrets::RegistryAuth,
    Reference,
};
use ptsession::{session::Wav, PtSession};
use sha2::{Digest as _, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::sync::mpsc::Sender;

struct ProcessedBlob {
    original_size: u64,
    original_path: String,
    original_digest: String,
    compressed_digest: String,
    size: u64,
}

// Stream that calculates digests while reading
struct DigestReader<R> {
    inner: R,
    hasher: Sha256,
    progress: Option<ProgressBar>,
}

impl<R: AsyncRead + Unpin> DigestReader<R> {
    fn new(inner: R, progress: Option<ProgressBar>) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            progress,
        }
    }

    fn digest(&self) -> String {
        format!("sha256:{:x}", self.hasher.clone().finalize())
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
                    self.hasher.update(&buf.filled()[original_filled..]);

                    // Update progress if provided
                    if let Some(progress) = &self.progress {
                        progress.inc(bytes_read as u64);
                    }
                }

                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

struct DigestWriter<W> {
    inner: W,
    hasher: Sha256,
    size: Arc<Mutex<u64>>,
}

impl<W: AsyncWrite + Unpin> DigestWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            size: Arc::new(Mutex::new(0)),
        }
    }

    fn digest(&self) -> String {
        format!("sha256:{:x}", self.hasher.clone().finalize())
    }

    fn size(&self) -> u64 {
        *self.size.lock().unwrap()
    }

    fn size_tracker(&self) -> Arc<Mutex<u64>> {
        self.size.clone()
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for DigestWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                if n > 0 {
                    // Update digest with written data
                    self.hasher.update(&buf[..n]);

                    // Update size counter
                    let mut size = self.size.lock().unwrap();
                    *size += n as u64;
                }

                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

struct HttpClient {
    client: reqwest::Client,
}

impl HttpClient {
    fn new(_oci_client: &oci_client::Client) -> Self {
        let client = reqwest::Client::builder()
            //.connection_verbose(true) // Enables detailed connection info for debugging
            .pool_idle_timeout(Some(std::time::Duration::from_secs(300))) // Keep connections alive
            .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
            .pool_max_idle_per_host(32) // Allow plenty of idle connections
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self { client }
    }

    async fn post(&self, url: &str) -> reqwest::RequestBuilder {
        self.client.post(url)
    }

    async fn patch(&self, url: &str) -> reqwest::RequestBuilder {
        self.client.patch(url)
    }

    async fn put(&self, url: reqwest::Url) -> reqwest::RequestBuilder {
        self.client.put(url)
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

async fn push_chunk(
    client: &HttpClient,
    location: &str,
    image: &Reference,
    blob_data: &[u8],
    start_byte: usize,
) -> Result<(String, usize), Box<dyn std::error::Error + Send + Sync>> {
    if blob_data.is_empty() {
        return Err("No data to push".into());
    }

    let end_byte = start_byte + blob_data.len() - 1;

    let res = client
        .patch(location)
        .await
        .header("Content-Range", format!("{}-{}", start_byte, end_byte))
        .header("Content-Length", blob_data.len())
        .header("Content-Type", "application/octet-stream")
        .body(blob_data.to_vec())
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

        Ok((new_location, end_byte + 1))
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

async fn compress_blob<R: AsyncRead + Unpin>(
    file_name: String,
    mut xz_encoder: XzEncoder<DigestWriter<Vec<u8>>>,
    mut digest_reader: DigestReader<R>,
    compressed_tx: Sender<Vec<u8>>,
) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    //let mut xz_encoder = xz_encoder;
    //let mut digest_reader = digest_reader;

    // Create buffer for reading
    let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer

    // Read from file, compress, and send chunks
    loop {
        let n = digest_reader.read(&mut buffer).await?;
        if n == 0 {
            break;
        }

        // Write to encoder
        xz_encoder.write_all(&buffer[..n]).await?;

        // Periodically flush to get compressed chunks
        if n == buffer.len() {
            // This is likely a full buffer, so there might be more data coming
            xz_encoder.flush().await?;

            // Extract compressed data
            let inner_writer = xz_encoder.get_mut();
            let chunk = std::mem::replace(&mut inner_writer.inner, Vec::new());

            if !chunk.is_empty() {
                compressed_tx
                    .send(chunk)
                    .await
                    .map_err(|_| "Channel closed")?;
            }
        }
    }

    // Finish the compression
    xz_encoder.shutdown().await?;

    // Get the final compressed data
    let inner_writer = xz_encoder.into_inner();
    let compressed_digest = inner_writer.digest();
    let final_chunk = inner_writer.inner;

    if !final_chunk.is_empty() {
        compressed_tx
            .send(final_chunk)
            .await
            .map_err(|_| "Channel closed")?;
    }

    if let Some(progress) = &digest_reader.progress {
        progress.finish_with_message(format!("Compression complete {}", file_name));
    }

    // Return the digests
    Ok((compressed_digest, digest_reader.digest()))
}

async fn compress_and_upload_file(
    client: &oci_client::Client,
    reference: &Reference,
    file_path: &str,
    compression_progress: ProgressBar,
    upload_progress: ProgressBar,
) -> Result<ProcessedBlob, Box<dyn std::error::Error + Send + Sync>> {
    // Create a shared http client wrapper for this operation
    let http_client = Arc::new(HttpClient::new(client));
    // Open the file
    let file = File::open(file_path).await?;

    // Get file size
    let file_size = file.metadata().await?.len();
    compression_progress.set_length(file_size);

    // Create digest reader to calculate original file digest
    let digest_reader = DigestReader::new(file, Some(compression_progress));

    // Create vector to store compressed data chunks temporarily
    let compressed_chunks = tokio::sync::mpsc::channel::<Vec<u8>>(4);
    let (compressed_tx, mut compressed_rx) = compressed_chunks;

    // Create in-memory buffer for compressed data
    let buffer = Vec::new();
    let digest_writer = DigestWriter::new(buffer);
    let compressed_size_tracker = digest_writer.size_tracker();

    // Create XZ encoder with digest writer
    let xz_encoder = XzEncoder::new(digest_writer);

    // Spawn task to compress the file
    let compression_task = tokio::spawn(compress_blob(
        file_path.to_string(),
        xz_encoder,
        digest_reader,
        compressed_tx,
    ));

    // Start the upload session
    let mut location = begin_push_chunked_session(&http_client, reference).await?;
    let mut start_byte = 0;

    // Pass the HTTP client wrapper to functions that need it
    //let http_client_ref = http_client.clone();

    // Set up upload progress bar
    upload_progress.set_message(format!("Uploading {file_path}"));

    // Process compressed chunks as they become available
    while let Some(chunk) = compressed_rx.recv().await {
        if !chunk.is_empty() {
            // Push this chunk
            let (new_location, new_start) =
                push_chunk(&http_client, &location, reference, &chunk, start_byte).await?;

            // Update progress
            upload_progress.set_length(*compressed_size_tracker.lock().unwrap());
            upload_progress.set_position(new_start as u64);

            // Update state for next iteration
            location = new_location;
            start_byte = new_start;
        }
    }

    // Wait for compression to complete and get digests
    let (compressed_digest, original_digest) = match compression_task.await? {
        Ok(res) => res,
        Err(e) => return Err(e),
    };

    // Get final compressed size
    let compressed_size = *compressed_size_tracker.lock().unwrap();

    // Set final upload progress bar length and position
    upload_progress.set_length(compressed_size);
    upload_progress.set_position(compressed_size);

    // Finish the upload
    let _blob_url =
        end_push_chunked_session(&http_client, &location, reference, &compressed_digest).await?;

    // Mark progress bars as complete
    upload_progress.finish_with_message(format!("Upload complete {file_path}"));

    Ok(ProcessedBlob {
        original_size: file_size,
        original_path: file_path.to_string(),
        original_digest,
        compressed_digest,
        size: compressed_size,
    })
}

pub async fn oras_push(
    reference: Reference,
    session: PtSession,
    PushArgs {
        compression,
        parallelism,
        ptx_file,
        ..
    }: PushArgs,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file_names = session
        .audio_files
        .iter()
        .map(|Wav { file_name, .. }: &Wav| format!("Audio Files/{file_name}"))
        // TODO: find files
        .filter(|file_name| {
            if !std::fs::exists(file_name).unwrap() {
                println!("❌ {file_name} does not exist");
                false
            } else {
                true
            }
        })
        .collect::<Vec<String>>();

    let client = oci_client::Client::new(ClientConfig {
        protocol: oci_client::client::ClientProtocol::Http,
        ..Default::default()
    });
    let auth = RegistryAuth::Anonymous;

    // Create a multi-progress bar
    let multi_progress = Arc::new(MultiProgress::new());

    // Setup progress styles
    let compression_style = ProgressStyle::default_bar()
        .template("[{elapsed_precise}] {bar:40.cyan/blue} {bytes}/{total_bytes} {msg}")
        .unwrap();

    let upload_style = ProgressStyle::default_bar()
        .template("[{elapsed_precise}] {bar:40.green/red} {bytes}/{total_bytes} {msg}")
        .unwrap();

    // Process all files in parallel with progress reporting
    println!("Processing audio files with parallelism: {}", parallelism);

    // Process files in chunks to control parallelism
    //let mut processed_files = Vec::new();

    let processed_files = futures_util::stream::iter(file_names)
        .map(|file_path| {
            let client = client.clone();
            let reference = reference.clone();
            let multi_progress = multi_progress.clone();
            let compression_style = compression_style.clone();
            let upload_style = upload_style.clone();

            async move {
                let file_name = Path::new(&file_path).to_string_lossy().to_string();

                // Create progress bars for this file
                let compression_progress = multi_progress.add(ProgressBar::new(0));
                compression_progress.set_style(compression_style);
                compression_progress.set_message(format!("Compressing {}", file_name));

                let upload_progress = multi_progress.add(ProgressBar::new(0));
                upload_progress.set_style(upload_style);
                upload_progress.set_message(format!("Uploading {}", file_name));

                // Process the file
                let result = compress_and_upload_file(
                    &client,
                    &reference,
                    &file_path,
                    compression_progress,
                    upload_progress,
                )
                .await?;

                Ok::<_, Box<dyn std::error::Error + Sync + Send>>(result)
            }
        })
        .buffer_unordered(parallelism) // Process up to parallelism files at a time
        .try_collect::<Vec<_>>() // Collect the results
        .await?;

    // Create layers for all processed files
    let layers: Vec<OciDescriptor> = processed_files
        .into_iter()
        .map(|file| {
            //let file_name = Path::new(&file.original_path).to_string_lossy().to_string();
            // Create annotations for this layer
            let annotations = maplit::btreemap! {
                ORG_OPENCONTAINERS_IMAGE_TITLE.to_string() => file.original_path,
                "io.ptsession.original.digest".to_string() => file.original_digest.clone(),
                "io.ptsession.original.size".to_string() => file.original_size.to_string(),
                "io.ptsession.compressed.digest".to_string() => file.compressed_digest.clone(),
                "io.deis.oras.content.unpack".to_string() => "true".to_string()
            };

            // Set mediaType
            let media_type = match compression {
                Compression::None => "audio/vnd.wav".to_string(),
                _ => format!("audio/vnd.wav+{}", compression.to_string()),
            };

            OciDescriptor {
                urls: None,
                digest: file.compressed_digest,
                size: file.size as i64,
                media_type,
                annotations: Some(annotations),
            }
        })
        .collect();

    println!("Creating manifest for {} layers", layers.len());
    let ptx_filename = ptx_file.file_name().unwrap().to_string_lossy().to_string();
    let mut ptx = File::open(ptx_file).await?;
    let ptx_meta = ptx.metadata().await?;
    let ptx_timestamp = ptx_meta.ctime();
    let mut ptx_data = Vec::with_capacity(ptx_meta.len() as usize);
    ptx.read_to_end(&mut ptx_data).await?;
    drop(ptx);
    let ptx_created = chrono::DateTime::from_timestamp(ptx_timestamp, 0)
        .ok_or("could not determine ptx file timestamp")?
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let ptx_size = ptx_data.len();
    let time = chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let config_annotations = maplit::btreemap! {
        ORG_OPENCONTAINERS_IMAGE_TITLE.to_string() => ptx_filename.clone(),
        ORG_OPENCONTAINERS_IMAGE_CREATED.to_string() => ptx_created,
        "io.ptsession.sample_rate".to_string() => session.session_sample_rate.to_string(),
    };
    let manifest_annotations = maplit::btreemap! {
        ORG_OPENCONTAINERS_IMAGE_TITLE.to_string() => ptx_filename,
        ORG_OPENCONTAINERS_IMAGE_CREATED.to_string() => time,
    };
    let config = Config {
        data: ptx_data,
        media_type: "application/vnd.avid.ptx".to_string(),
        annotations: Some(config_annotations.clone()),
    };
    let manifest = OciImageManifest {
        schema_version: 2,
        media_type: Some(OCI_IMAGE_MEDIA_TYPE.to_string()),
        artifact_type: Some("application/vnd.avid.ptsession".to_string()),
        config: OciDescriptor {
            size: ptx_size as i64,
            media_type: config.media_type.clone(),
            digest: config.sha256_digest(),
            annotations: Some(config_annotations),
            ..Default::default()
        },
        layers,
        annotations: Some(manifest_annotations),
    };

    println!("Pushing manifest...");
    let res: PushResponse = client
        .push(&reference, &[], config, &auth, Some(manifest))
        .await?;
    println!("Push completed. Manifest URL: {}", res.manifest_url);

    Ok(())
}
