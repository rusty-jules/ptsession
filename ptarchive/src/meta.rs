use std::os::unix::fs::MetadataExt;

use chrono::DateTime;
use tokio::fs::File;

#[cfg(target_os = "macos")]
pub async fn file_meta(
    file: File,
) -> Result<(File, u64, String, Option<String>), Box<dyn std::error::Error + Send + Sync>> {
    use std::{os::macos::fs::MetadataExt, time::UNIX_EPOCH};
    let std_file = file.into_std().await;

    let file_meta = std_file.metadata()?;
    let file_size = file_meta.size();
    let birthtime = file_meta.st_birthtime();
    let modified = file_meta.modified();

    // drop nanoseconds
    let file_created = DateTime::from_timestamp(birthtime, 0)
        .ok_or("could not determine file timestamp")?
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let file_modified = modified
        .map(|modified| {
            DateTime::from_timestamp(
                modified
                    .duration_since(UNIX_EPOCH)
                    .expect("system time since unix epoch")
                    .as_secs() as i64,
                0,
            )
            .expect("could not parse modified timestamp")
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .ok();

    // WARNING: as said in the docs, File::from_std could block,
    // but tokio doesn't seem to have darwin MetadataExt so we
    // don't have much of a choice
    Ok((
        File::from_std(std_file),
        file_size,
        file_created,
        file_modified,
    ))
}

#[cfg(target_os = "linux")]
pub async fn file_meta(
    file: File,
) -> Result<(File, u64, String), Box<dyn std::error::Error + Send + Sync>> {
    let file_meta = file.metadata().await?;
    let file_size = file_meta.len();
    let file_changed = file_meta.ctime();
    let file_created = DateTime::from_timestamp(file_changed, 0)
        .ok_or("could not determine file timestamp")?
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    Ok((file, file_size, file_created))
}

#[cfg(target_os = "windows")]
pub fn file_meta(file: &File) {
    unimplemented!()
}
