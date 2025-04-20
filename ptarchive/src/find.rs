use crate::FindArgs;

use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use bwavfile::WaveReader;
use ignore::overrides::OverrideBuilder;
use ignore::{DirEntry, WalkBuilder, WalkState};
use ptsession::PtSession;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt as _};
use tracing::{debug, error, info, warn};

const DIR_ENTRY_CHANNEL_SIZE: usize = 100;

fn to_name_and_path(entry: DirEntry) -> (String, PathBuf) {
    let path = entry.into_path();
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .map(String::from)
        .unwrap();
    (name, path)
}

// TODO: ignore filenames (only length & unique id?)
fn by_filename(missing_set: Arc<Mutex<HashSet<String>>>) -> impl FnMut(&(String, PathBuf)) -> bool {
    move |(name, _): &(String, PathBuf)| -> bool {
        missing_set
            .lock()
            .expect("get missing set lock")
            .contains(name)
    }
}

fn by_file_duration(
    missing_lengths: HashMap<String, usize>,
    no_duration: bool,
) -> impl FnMut(&(String, PathBuf)) -> bool {
    move |(name, path): &(String, PathBuf)| -> bool {
        let ext = path.extension().and_then(OsStr::to_str);

        let span = tracing::trace_span!("by_file_duration", file = name, path = path.to_str(), ext);
        let _guard = span.enter();

        if ext.is_none() {
            warn!("unknown file extension");
            return false;
        }

        match ext.unwrap() {
            "wav" => {
                // get the frame length of the file to match what pro tools stores
                let r = WaveReader::open(path);
                if let Err(e) = r {
                    warn!("failed to open: {e}");
                    return false;
                }

                let fl = r.unwrap().frame_length();
                if let Err(e) = fl {
                    warn!("failed to read frame length: {e}",);
                    return false;
                }

                let len = *missing_lengths
                    .get(name)
                    .expect("missing file to have length in ptsession")
                    as u64;
                let len_matches = fl.unwrap() == len;

                if !no_duration {
                    return len_matches;
                }

                if no_duration && !len_matches {
                    warn!(
                        "wav has mismatched duration, but duration is being ignored for matching"
                    );
                }

                return true;
            }
            _ => {
                error!("cannot match duration by this file type");
                return false;
            }
        }
    }
}

fn by_file_unique_id(
    missing_unique_ids: HashMap<String, String>,
) -> impl FnMut(&(String, PathBuf)) -> bool {
    |(name, path): &(String, PathBuf)| -> bool {
        let ext = path.extension().and_then(OsStr::to_str);

        let span =
            tracing::trace_span!("by_file_unique_id", file = name, path = path.to_str(), ext);
        let _guard = span.enter();

        if ext.is_none() {
            warn!("unknown file extension");
            return false;
        }

        match ext.unwrap() {
            "wav" => {
                let r = WaveReader::open(path);
                if let Err(e) = r {
                    warn!("failed to open file: {e}");
                    return false;
                }

                let bext = r.unwrap().broadcast_extension();
                if let Err(e) = bext {
                    warn!("failed to read bext: {e}");
                    return false;
                }

                match bext.unwrap() {
                    None => {
                        warn!("no bext found, cannot verify unique id");
                        return true;
                    }
                    Some(bext) => {
                        if bext.originator != "Pro Tools" {
                            warn!("wav does not originate from Pro Tools, cannot verify unique id",);
                            return true;
                        }
                        let id = bext.originator_reference;
                        if id == "" {
                            warn!("wav has no originator reference, cannot verify unique id",);
                            return false;
                        }

                        // TODO: get originator references from pro tools session
                        info!("originator reference: {id}");
                        return true;
                    }
                }
            }
            _ => {
                warn!("cannot match by extension for file type");
                return false;
            }
        }
    }
}

fn remove_from_missing(
    missing_set: Arc<Mutex<HashSet<String>>>,
) -> impl FnMut((String, PathBuf)) -> (String, PathBuf) {
    move |(name, path): (String, PathBuf)| -> (String, PathBuf) {
        let span = tracing::trace_span!("remove from missing", file = name, path = path.to_str());
        let _guard = span.enter();
        let existed = missing_set
            .lock()
            .expect("get missing files set lock")
            .remove(&name);
        if !existed {
            debug!("received a file that did not exist in the missing set");
        }
        info!("found file");
        (name, path)
    }
}

fn format_name_to_artifact_path((name, path): (String, PathBuf)) -> (String, PathBuf) {
    // NOTE: not sure if "Audio Files" should be prepended here
    (format!("Audio Files/{name}"), path)
}

fn start_walkers(
    parallelism: usize,
    find_args: &FindArgs,
    missing_set: Arc<Mutex<HashSet<String>>>,
    tx: mpsc::Sender<DirEntry>,
) {
    // split up configured parallelism among the walkers,
    // setting min 1 since passing 0 allows WalkBuilder to
    // select its own number of threads
    let walk_par = (parallelism / find_args.search_paths.len()).min(1);
    let walk_depth = Some(find_args.depth).and_then(|d| if d == 0 { None } else { Some(d) });
    let len = missing_set.lock().unwrap().len();
    for path in find_args.search_paths.iter() {
        let span = tracing::trace_span!("start_walkers", path);
        let _guard = span.enter();
        info!("searching for {len} missing audio files");
        // build up the missing file allow list
        let mut overrides = OverrideBuilder::new(path);
        for file in missing_set.lock().unwrap().iter() {
            overrides.add(file).expect("override pattern");
        }
        WalkBuilder::new(path)
            .overrides(overrides.build().expect("overrides allowlist"))
            .max_depth(walk_depth)
            .threads(walk_par)
            .build_parallel()
            .run(|| {
                let tx = tx.clone();
                let missing_set = missing_set.clone();
                Box::new(move |result| match result {
                    Ok(entry) => {
                        // don't send directories to the receiver stream
                        if let Some(ft) = entry.file_type() {
                            if ft.is_file() {
                                if let Err(e) = tx.blocking_send(entry) {
                                    warn!("{e}");
                                }
                            }
                        }
                        // exit early if we've found all files
                        if missing_set.lock().expect("lock missing set").is_empty() {
                            WalkState::Quit
                        } else {
                            WalkState::Continue
                        }
                    }
                    Err(e) => match e {
                        // don't panic on permission errors, just log them
                        e if e.clone().into_io_error().is_some() => {
                            warn!("{e}");
                            WalkState::Skip
                        }
                        e => {
                            warn!("{e}");
                            WalkState::Continue
                        }
                    },
                })
            });
    }
    // NOTE: original tx is dropped here, which is important for the rx stream to close
}

async fn start_stream(
    FindArgs {
        no_filename,
        no_duration,
        unique_id,
        ..
    }: &FindArgs,
    missing_set: Arc<Mutex<HashSet<String>>>,
    missing_lengths: HashMap<String, usize>,
    missing_unique_ids: HashMap<String, String>,
    rx: mpsc::Receiver<DirEntry>,
) -> Vec<(String, PathBuf)> {
    let mut files_stream: Pin<Box<dyn Stream<Item = (String, PathBuf)>>> =
        Box::pin(ReceiverStream::new(rx).map(to_name_and_path));

    if !*no_filename {
        files_stream = Box::pin(files_stream.filter(by_filename(missing_set.clone())));
    }

    files_stream = Box::pin(files_stream.filter(by_file_duration(missing_lengths, *no_duration)));

    if *unique_id {
        files_stream = Box::pin(files_stream.filter(by_file_unique_id(missing_unique_ids)));
    }

    files_stream
        .map(remove_from_missing(missing_set.clone()))
        .map(format_name_to_artifact_path)
        .collect::<Vec<(String, PathBuf)>>()
        .await
}

pub async fn find_files(
    session: &PtSession,
    missing_files: Vec<String>,
    parallelism: usize,
    find_args: FindArgs,
) -> Result<Vec<(String, PathBuf)>, Box<dyn std::error::Error + Send + Sync>> {
    if find_args.ignore_missing || missing_files.is_empty() {
        if !missing_files.is_empty() {
            info!("ignoring {} missing audio files", missing_files.len());
        }
        return Ok(vec![]);
    }

    let mut missing_set = missing_files
        .into_iter()
        .map(PathBuf::from)
        // NOTE: we're throwing out any filenames that don't conform to UTF-8,
        // but since these names come from PtSession which is already `String` it's ok
        .filter_map(|p| p.file_name().and_then(OsStr::to_str).map(String::from))
        .collect::<HashSet<String>>();

    if find_args.regions_only {
        let region_audio_files = session
            .audio_regions
            .iter()
            .filter_map(|r| r.wav.as_ref())
            .map(|w| w.file_name.as_str())
            .collect::<HashSet<&str>>();
        let len_prev = missing_set.len();
        missing_set = missing_set
            .into_iter()
            .filter(|f| region_audio_files.contains(&f.as_str()))
            .collect::<HashSet<String>>();
        if missing_set.len() < len_prev {
            info!(
                "ignoring {} audio files with no regions in the session",
                len_prev - missing_set.len()
            )
        }
    }

    if missing_set.is_empty() {
        return Ok(vec![]);
    }

    let missing_set = Arc::new(Mutex::new(missing_set));

    // create a HashMap of file names to file durations
    let missing_lengths = session
        .audio_files
        .iter()
        .map(|wav| (wav.file_name.clone(), wav.len))
        .collect::<HashMap<String, usize>>();

    // TODO: get missing unique ids
    let missing_unique_ids = HashMap::new();

    // Kick off the search
    let (tx, rx) = mpsc::channel::<DirEntry>(DIR_ENTRY_CHANNEL_SIZE);
    start_walkers(parallelism, &find_args, missing_set.clone(), tx);
    let found_files = start_stream(
        &find_args,
        missing_set.clone(),
        missing_lengths,
        missing_unique_ids,
        rx,
    )
    .await;

    let still_missing = missing_set
        .lock()
        .expect("no more contention on missing set");

    if !still_missing.is_empty() {
        for name in still_missing.iter() {
            warn!(file = name, "could not be found");
        }
        if find_args.fail_missing {
            error!("failing on missing audio files");
            std::process::exit(1);
        }
    }

    Ok(found_files)
}
