use std::ops::Not;

use crate::GlobalOpts;

use metrics_tracing_context::label_filter::Allowlist;
use metrics_tracing_context::{MetricsLayer, TracingContextLayer};
use metrics_util::layers::Layer;
use tracing::Level;
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::util::SubscriberInitExt;

/// Interval millis for metrics exporting
pub const EXPORT_MILLIS: u64 = 500;

pub fn init(opts: &GlobalOpts) {
    let filter = Targets::new().with_target("ptarchive", Level::DEBUG);
    let json = opts.json.then(|| {
        json_subscriber::fmt::layer()
            .with_target(false)
            .flatten_event(true)
            .flatten_current_span_on_top_level(true)
            .flatten_span_list_on_top_level(true)
            //.with_writer(std::io::stderr)
            .with_filter(filter.clone())
    });
    let metrics_layer = opts.json.then(|| MetricsLayer::new());

    let indicatif_layer = IndicatifLayer::new();
    let text = opts.json.not().then(|| {
        tracing_subscriber::fmt::layer()
            .without_time()
            .with_file(false)
            .with_target(false)
            .with_line_number(false)
            //.pretty()
            .with_writer(indicatif_layer.get_stderr_writer())
            .with_filter(filter)
    });
    let ind = opts.json.not().then(|| indicatif_layer);

    tracing_subscriber::registry()
        .with(json)
        .with(metrics_layer)
        .with(text)
        .with(ind)
        .init();

    if opts.json {
        let (recorder, exporter) = metrics_exporter_influx::InfluxBuilder::new()
            .with_duration(std::time::Duration::from_millis(EXPORT_MILLIS))
            .add_global_tag("service", "ptarchive")
            // TODO: allow configuration of metrics and log file location
            .with_writer(std::fs::File::create("ptarchive.metrics").unwrap())
            .build()
            .unwrap();

        let allow = Allowlist::new(["hdd.name", "session", "file"]);
        let recorder = TracingContextLayer::new(allow).layer(recorder);

        metrics::set_boxed_recorder(Box::new(recorder)).unwrap();
        tokio::spawn(exporter);
    }
}
