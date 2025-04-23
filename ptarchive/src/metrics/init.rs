use std::fs::File;
use std::ops::Not;
use std::path::PathBuf;

use crate::{Arguments, GlobalOpts, LogFile, LogFilenameMethod, MetricsFlavor, MetricsOptions};

use metrics_tracing_context::label_filter::Allowlist;
use metrics_tracing_context::{MetricsLayer, TracingContextLayer};
use metrics_util::layers::Layer;
use sha2::Digest;
use tracing::Level;
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::util::SubscriberInitExt;

/// Interval millis for metrics exporting
pub const EXPORT_MILLIS: u64 = 500;

impl LogFile {
    fn get_file(&self, ptx_file: &String) -> File {
        let filename = match self.filename.method {
            LogFilenameMethod::Hash => {
                let mut hasher = sha2::Sha256::new();
                hasher.update(ptx_file);
                format!("{:x}.log", hasher.finalize())
            }
        };
        let mut directory = PathBuf::from(shellexpand::tilde(&self.directory).as_ref());
        std::fs::create_dir_all(&directory).expect("create log file directory");
        directory.push(filename);
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(directory)
            .expect("create log file")
    }

    fn get_boxed_file(&self, ptx_file: Option<&String>) -> Box<dyn std::io::Write> {
        match ptx_file {
            Some(f) => Box::new(self.get_file(f)),
            None => Box::new(std::io::stdout()),
        }
    }
}

pub fn init(
    Arguments { logs, metrics, .. }: &Arguments,
    ptx_file: Option<String>,
    opts: &GlobalOpts,
) {
    let logs = logs.clone().unwrap_or_default();
    let json = logs.format.is_json() || opts.json;
    let filter = Targets::new()
        .with_target("ptarchive", Level::TRACE)
        //.with_target("ptsession", Level::TRACE)
        .with_target("metrics_exporter_influx", Level::WARN);
    let json_sub = json.then(|| {
        json_subscriber::fmt::layer()
            .with_target(false)
            .flatten_event(true)
            .flatten_current_span_on_top_level(true)
            .flatten_span_list_on_top_level(true)
            .with_writer(move || match logs.file.clone() {
                Some(lf) => lf.get_boxed_file(ptx_file.as_ref()),
                None => Box::new(std::io::stdout()),
            })
            .with_filter(filter.clone())
    });
    let metrics_layer = metrics.is_some().then(|| MetricsLayer::new());

    let indicatif_layer = IndicatifLayer::new();
    let text = json.not().then(|| {
        tracing_subscriber::fmt::layer()
            .without_time()
            .with_file(false)
            .with_target(false)
            .with_line_number(false)
            //.pretty()
            .with_writer(indicatif_layer.get_stderr_writer())
            .with_filter(filter)
    });
    let ind = json.not().then(|| indicatif_layer);

    tracing_subscriber::registry()
        .with(json_sub)
        .with(metrics_layer)
        .with(text)
        .with(ind)
        .init();

    if let Some(MetricsOptions {
        flavor,
        endpoint,
        database,
        table,
        ..
    }) = metrics
    {
        match flavor {
            MetricsFlavor::Influxdb => {
                let endpoint = endpoint.as_ref().expect("metrics endpoint required");
                let database = database.clone().expect("metrics database required");
                let table = table.clone();
                let (recorder, exporter) = metrics_exporter_influx::InfluxBuilder::new()
                    .with_duration(std::time::Duration::from_millis(EXPORT_MILLIS))
                    .add_global_tag("service", "ptarchive")
                    .with_influx_api(endpoint.as_str(), database, None, None, table)
                    .expect("connect to influxdb")
                    .build()
                    .unwrap();

                let allow = Allowlist::new(["hdd.name", "session", "file", "repository", "tag"]);
                let recorder = TracingContextLayer::new(allow).layer(recorder);

                metrics::set_boxed_recorder(Box::new(recorder)).unwrap();
                tokio::spawn(exporter);
            }
            MetricsFlavor::None => {}
        }
    }
}
