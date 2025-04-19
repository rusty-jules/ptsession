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

const DIR_ENTRY_CHANNEL_SIZE: usize = 100;

fn to_name_and_path(entry: DirEntry) -> (String, PathBuf) {
    let path = entry.into_path();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
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

        if ext.is_none() {
            eprintln!("Unknown file extension for {name}");
            return false;
        }

        match ext.unwrap() {
            "wav" => {
                // get the frame length of the file to match what pro tools stores
                let r = WaveReader::open(path);
                if let Err(e) = r {
                    eprintln!("Failed to open {}: {e}", path.display());
                    return false;
                }

                let fl = r.unwrap().frame_length();
                if let Err(e) = fl {
                    eprintln!("Failed to read frame length of {}: {e}", path.display());
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
                    println!("🚨 Warning: {name} has mismatched file length, but length is being ignored for matching");
                }

                return true;
            }
            ext => {
                eprintln!("Cannot match by extension for file type {ext}");
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

        if ext.is_none() {
            eprintln!("Unknown file extension for {name}");
            return false;
        }

        match ext.unwrap() {
            "wav" => {
                let r = WaveReader::open(path);
                if let Err(e) = r {
                    eprintln!("❌ Failed to open {}: {e}", path.display());
                    return false;
                }

                let bext = r.unwrap().broadcast_extension();
                if let Err(e) = bext {
                    eprintln!("❌ Failed to read {} bext: {e}", path.display());
                    return false;
                }

                match bext.unwrap() {
                    None => {
                        eprintln!("⚠️ {} has no bext, cannot verify unique id", path.display());
                        return true;
                    }
                    Some(bext) => {
                        if bext.originator != "Pro Tools" {
                            eprintln!(
                                "⚠️ {} does not originate from Pro Tools, cannot verify unique id",
                                path.display()
                            );
                            return true;
                        }
                        let id = bext.originator_reference;
                        if id == "" {
                            eprintln!(
                                "❌ {} has no originator reference, cannot verify unique id",
                                path.display()
                            );
                            return false;
                        }

                        // TODO: get originator references from pro tools session
                        println!("{} originator reference: {}", path.display(), id);
                        return true;
                    }
                }
            }
            ext => {
                eprintln!(
                    "❌ {} cannot match by extension for file type {ext}",
                    path.display()
                );
                return false;
            }
        }
    }
}

fn remove_from_missing(
    missing_set: Arc<Mutex<HashSet<String>>>,
) -> impl FnMut((String, PathBuf)) -> (String, PathBuf) {
    move |(name, path): (String, PathBuf)| -> (String, PathBuf) {
        let existed = missing_set
            .lock()
            .expect("get missing files set lock")
            .remove(&name);
        if !existed {
            println!("Received a file that did not exist in the missing set");
        }
        println!("✅ Found file: {}", path.display());
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
    for path in find_args.search_paths.iter() {
        println!("Searching: {path}");
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
                                    eprintln!("{e}");
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
                            eprintln!("{e}");
                            WalkState::Skip
                        }
                        e => {
                            eprintln!("{e}");
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
        return Ok(vec![]);
    }

    let missing_files = missing_files
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<PathBuf>>();

    // Create a Send + Sync HashSet of the missing file names
    let missing_set = Arc::new(Mutex::new(
        missing_files
            .iter()
            // NOTE: we're throwing out any filenames that don't conform to UTF-8,
            // but since these names come from PtSession which is already `String` it's ok
            .filter_map(|p| p.file_name().and_then(OsStr::to_str).map(String::from))
            .collect::<HashSet<String>>(),
    ));

    // Create a HashMap of file names to file lengths
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
            println!("❌ {name} could not be found");
        }
        if find_args.fail_missing {
            println!("Failing");
            std::process::exit(1);
        }
    }

    Ok(found_files)
}
