#![allow(unused)]
#![allow(unreachable_code)]

use crate::FindArgs;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::{borrow::Cow, collections::HashSet};

use ignore::types::{Types, TypesBuilder};
use ignore::{DirEntry, Walk, WalkBuilder, WalkState};
use ptsession::PtSession;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt as _};

/// Filter out directories from ignore
fn filter_files(entry: DirEntry) -> Option<(String, PathBuf)> {
    if entry.file_type().unwrap().is_file() {
        let path = entry.into_path();
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap();
        Some((name, path))
    } else {
        None
    }
}

// TODO: ignore filenames (only length & unique id?)
fn by_file_name(
    missing_set: Arc<Mutex<HashSet<String>>>,
    file_name: bool,
) -> impl FnMut(&(String, PathBuf)) -> bool {
    move |(name, path): &(String, PathBuf)| -> bool {
        if file_name {
            return missing_set
                .lock()
                .expect("get missing set lock")
                .contains(name);
        }
        true
    }
}

fn by_file_length(
    missing_lengths: HashMap<String, usize>,
    length: bool,
) -> impl FnMut(&(String, PathBuf)) -> bool {
    move |(name, path): &(String, PathBuf)| -> bool {
        let len = *missing_lengths
            .get(name)
            .expect("missing file to have length in ptsession") as u64;
        let meta_len = std::fs::metadata(path).expect("to get file metadata").len();
        let len_matches = meta_len == len;
        if length {
            len_matches
        } else {
            if !length && !len_matches {
                println!("🚨 Warning: {name} has mismatched file length, but length is being ignored for matching");
            }
            true
        }
    }
}

// TODO: filter by pt unique id
fn by_file_unique_id(unique_id: bool) -> impl FnMut(&(String, PathBuf)) -> bool {
    |(name, path): &(String, PathBuf)| -> bool { unimplemented!() }
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

pub async fn find_files(
    session: &PtSession,
    missing_files: Vec<String>,
    parallelism: usize,
    find_args: FindArgs,
) -> Result<Vec<(String, PathBuf)>, Box<dyn std::error::Error + Send + Sync>> {
    if find_args.ignore_missing {
        return Ok(vec![]);
    }

    // Create a Send + Sync HashSet of the missing file names
    let missing_set = Arc::new(Mutex::new(
        missing_files
            .iter()
            .map(PathBuf::from)
            .filter_map(|p| {
                // NOTE: we're throwing out any filenames that don't conform to UTF-8,
                // but since these names come from PtSession which is already `String` it's ok
                p.file_name().and_then(OsStr::to_str).map(String::from)
            })
            .collect::<HashSet<String>>(),
    ));

    // Create a HashMap of file names to file lengths
    let missing_lengths = session
        .audio_files
        .iter()
        .map(|wav| (wav.file_name.clone(), wav.len))
        .collect::<HashMap<String, usize>>();

    // Get all file extensions from the pt session
    let extensions = missing_files
        .iter()
        .filter_map(|file| {
            PathBuf::from(file)
                .extension()
                .and_then(|os_str| Some(os_str.to_string_lossy().to_string()))
        })
        .collect::<HashSet<String>>();

    // Turn file extensions into ignore glob types
    let globs = extensions
        .iter()
        .map(|ext| format!("{ext}:*.{ext}"))
        .collect::<Vec<String>>();

    let mut types = TypesBuilder::new();
    println!("Searching with globs {globs:?}");
    types.add_def(&globs.join(","))?;
    for ref ext in extensions {
        types.select(ext);
    }

    // FIXME: parse multiple paths and create multiple walkers
    let path = find_args
        .search_path
        .unwrap_or(vec!["../".to_string()])
        .pop()
        .expect("at least one file search path");

    let (tx, mut rx) = mpsc::channel::<DirEntry>(100);

    println!("Searching: {path}");
    WalkBuilder::new(path)
        .max_depth(Some(find_args.depth))
        .types(types.build()?)
        .threads(parallelism)
        .build_parallel()
        .run(|| {
            let tx = tx.clone();
            Box::new(move |result| {
                let r = result.unwrap();
                tx.blocking_send(r);
                WalkState::Continue
            })
        });

    // drop the original tx so the stream closes when the walkers are done
    drop(tx);

    let FindArgs {
        file_name,
        length,
        unique_id,
        ..
    } = find_args;

    let found_files: Vec<(String, PathBuf)> = ReceiverStream::new(rx)
        .filter_map(filter_files)
        .filter(by_file_name(missing_set.clone(), file_name))
        .filter(by_file_length(missing_lengths, length))
        //.filter(by_file_unique_id(unique_id))
        .map(remove_from_missing(missing_set.clone()))
        .map(format_name_to_artifact_path)
        .collect()
        .await;

    let still_missing = missing_set
        .lock()
        .expect("no more contention on missing set");
    if !still_missing.is_empty() {
        for name in still_missing.iter() {
            println!("❌ {name} could not be found");
        }
    }

    Ok(found_files)
}
