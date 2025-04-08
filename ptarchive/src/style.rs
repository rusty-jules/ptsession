use std::fmt::Write;
use std::sync::LazyLock;

use indicatif::{ProgressState, ProgressStyle};

static UPLOAD_BAR: &str = "[{elapsed_precise}] {bar:40.cyan/blue} {bytes}/{total_bytes} {msg}";
static DOWNLOAD_BAR_YELLOW: &str = "[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes:>12}/{total_bytes:<12} {check:.yellow} {msg} ({eta})";
static DOWNLOAD_BAR_GREEN: &str =
    "[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes:>12}/{total_bytes:<12} {check:.green} {msg}";
static DOWNLOAD_BAR_RED: &str =
    "[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes:>12}/{total_bytes:<12} {check:.red} {msg}";
static HASH_PROGRESS: &str = "#>-";
static CHECK_MARK: &str = "✓";
static X_MARK: &str = "✖︎";

pub static COMPRESSION_STYLE: LazyLock<ProgressStyle> =
    LazyLock::new(|| ProgressStyle::default_bar().template(UPLOAD_BAR).unwrap());

pub static UPLOAD_STYLE: LazyLock<ProgressStyle> =
    LazyLock::new(|| ProgressStyle::default_bar().template(UPLOAD_BAR).unwrap());

pub static DOWNLOAD_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template(DOWNLOAD_BAR_YELLOW)
        .expect("correct progress style")
        .progress_chars(HASH_PROGRESS)
        .with_key("eta", |state: &ProgressState, w: &mut dyn Write| {
            write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap()
        })
        .with_key("check", |state: &ProgressState, w: &mut dyn Write| {
            let icon = if state.fraction() >= 1.0 {
                CHECK_MARK
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
});

pub static FINISH_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template(DOWNLOAD_BAR_GREEN)
        .expect("correct progress style")
        .progress_chars(HASH_PROGRESS)
        .with_key("check", |state: &ProgressState, w: &mut dyn Write| {
            let icon = if state.fraction() >= 1.0 {
                CHECK_MARK
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
});

pub static FAILED_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template(DOWNLOAD_BAR_RED)
        .expect("correct progress style")
        .progress_chars(HASH_PROGRESS)
        .with_key("check", |_state: &ProgressState, w: &mut dyn Write| {
            write!(w, "{X_MARK}").unwrap()
        })
});
