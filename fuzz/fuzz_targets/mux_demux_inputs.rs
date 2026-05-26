#![no_main]

mod support;

use std::fs;

use libfuzzer_sys::fuzz_target;
use mp4forge::mux::inspect::{
    DirectIngestReportFormat, collect_packet_report_warnings, collect_track_report_warnings,
    inspect_direct_ingest_packets, inspect_direct_ingest_path, write_packet_report, write_report,
};
use mp4forge::mux::{MuxRequest, MuxTrackSpec, mux_to_path};
use tempfile::tempdir;

use support::FuzzInput;

const MAX_INPUT_LEN: usize = 128 * 1024;

const SUFFIXES: [&str; 42] = [
    ".aac", ".adts", ".latm", ".mp3", ".ac3", ".ec3", ".ac4", ".dts", ".thd", ".mhas", ".iamf",
    ".amr", ".awb", ".h263", ".m4v", ".m2v", ".h264", ".264", ".h265", ".hevc", ".vvc", ".obu",
    ".ivf", ".vp8", ".vp9", ".jpg", ".jpeg", ".png", ".bmp", ".j2k", ".y4m", ".prores", ".wav",
    ".aiff", ".flac", ".ogg", ".avi", ".ts", ".mpeg", ".mpg", ".qcp", ".caf",
];

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let mut input = FuzzInput::new(data);
    let suffix = SUFFIXES[input.take_usize(SUFFIXES.len() - 1)];
    let payload_start = input.take_usize(data.len().saturating_sub(1));
    let payload_end = data.len().min(payload_start.saturating_add(MAX_INPUT_LEN));
    let payload = &data[payload_start..payload_end];
    if payload.is_empty() {
        return;
    }

    let Ok(dir) = tempdir() else {
        return;
    };
    let input_path = dir.path().join(format!("input{suffix}"));
    if fs::write(&input_path, payload).is_err() {
        return;
    }

    match input.take_u8() % 4 {
        0 => {
            if let Ok(report) = inspect_direct_ingest_path(&input_path) {
                let _ = collect_track_report_warnings(&report);
                let mut rendered = Vec::new();
                let _ = write_report(&mut rendered, &report, take_report_format(&mut input));
            }
        }
        1 => {
            if let Ok(report) = inspect_direct_ingest_packets(&input_path) {
                let _ = collect_packet_report_warnings(&report);
                let mut rendered = Vec::new();
                let _ = write_packet_report(
                    &mut rendered,
                    &report,
                    take_packet_report_format(&mut input),
                );
            }
        }
        2 => {
            let output_path = dir.path().join("muxed.mp4");
            let request = MuxRequest::new(vec![MuxTrackSpec::path(&input_path)]);
            let _ = mux_to_path(&request, &output_path);
        }
        _ => {
            let _ = inspect_direct_ingest_path(&input_path);
            let _ = inspect_direct_ingest_packets(&input_path);
        }
    }
});

fn take_report_format(input: &mut FuzzInput<'_>) -> DirectIngestReportFormat {
    match input.take_u8() % 3 {
        0 => DirectIngestReportFormat::Json,
        1 => DirectIngestReportFormat::Yaml,
        _ => DirectIngestReportFormat::Nhml,
    }
}

fn take_packet_report_format(input: &mut FuzzInput<'_>) -> DirectIngestReportFormat {
    match input.take_u8() % 3 {
        0 => DirectIngestReportFormat::Json,
        1 => DirectIngestReportFormat::Yaml,
        _ => DirectIngestReportFormat::Nhnt,
    }
}
