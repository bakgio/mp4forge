#![no_main]

mod support;

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mp4forge::cli::probe::{
    ProbeFormat, ProbeReportOptions, build_codec_detailed_report_with_options,
    build_detailed_report_with_options, build_media_characteristics_report_with_options,
    build_report_with_options, write_codec_detailed_report, write_detailed_report,
    write_media_characteristics_report, write_report,
};
use mp4forge::probe::{
    ProbeOptions, SegmentInfo, TrackInfo, average_sample_bitrate, average_segment_bitrate,
    find_idr_frames, max_sample_bitrate, max_segment_bitrate, probe_bytes_with_options,
    probe_codec_detailed_bytes_with_options, probe_codec_detailed_with_options,
    probe_detailed_bytes_with_options, probe_detailed_with_options, probe_fra,
    probe_fra_codec_detailed, probe_fra_detailed, probe_fra_media_characteristics,
    probe_media_characteristics_bytes_with_options, probe_media_characteristics_with_options,
    probe_with_options,
};

use support::{FuzzInput, seeded_any_mp4_bytes};

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let bytes = seeded_any_mp4_bytes(&mut input);
    let options = take_report_options(&mut input);
    let format = take_probe_format(&mut input);

    match input.take_u8() % 4 {
        0 => exercise_coarse_probe_surface(&bytes, options, format),
        1 => exercise_detailed_probe_surface(&bytes, options, format),
        2 => exercise_codec_detailed_probe_surface(&bytes, options, format),
        _ => exercise_media_characteristics_probe_surface(&bytes, options, format),
    }
});

fn exercise_coarse_probe_surface(bytes: &[u8], options: ProbeReportOptions, format: ProbeFormat) {
    if let Ok(summary) = probe_with_options(&mut Cursor::new(bytes), options.probe) {
        exercise_track_metrics(bytes, &summary.tracks, &summary.segments);
    }

    let _ = probe_bytes_with_options(bytes, options.probe);
    let _ = probe_fra(&mut Cursor::new(bytes));

    if let Ok(report) = build_report_with_options(&mut Cursor::new(bytes), options) {
        let mut rendered = Vec::new();
        let _ = write_report(&mut rendered, &report, format);
    }
}

fn exercise_detailed_probe_surface(bytes: &[u8], options: ProbeReportOptions, format: ProbeFormat) {
    let _ = probe_detailed_with_options(&mut Cursor::new(bytes), options.probe);
    let _ = probe_detailed_bytes_with_options(bytes, options.probe);
    let _ = probe_fra_detailed(&mut Cursor::new(bytes));

    if let Ok(report) = build_detailed_report_with_options(&mut Cursor::new(bytes), options) {
        let mut rendered = Vec::new();
        let _ = write_detailed_report(&mut rendered, &report, format);
    }
}

fn exercise_codec_detailed_probe_surface(
    bytes: &[u8],
    options: ProbeReportOptions,
    format: ProbeFormat,
) {
    let _ = probe_codec_detailed_with_options(&mut Cursor::new(bytes), options.probe);
    let _ = probe_codec_detailed_bytes_with_options(bytes, options.probe);
    let _ = probe_fra_codec_detailed(&mut Cursor::new(bytes));

    if let Ok(report) = build_codec_detailed_report_with_options(&mut Cursor::new(bytes), options) {
        let mut rendered = Vec::new();
        let _ = write_codec_detailed_report(&mut rendered, &report, format);
    }
}

fn exercise_media_characteristics_probe_surface(
    bytes: &[u8],
    options: ProbeReportOptions,
    format: ProbeFormat,
) {
    let _ = probe_media_characteristics_with_options(&mut Cursor::new(bytes), options.probe);
    let _ = probe_media_characteristics_bytes_with_options(bytes, options.probe);
    let _ = probe_fra_media_characteristics(&mut Cursor::new(bytes));

    if let Ok(report) =
        build_media_characteristics_report_with_options(&mut Cursor::new(bytes), options)
    {
        let mut rendered = Vec::new();
        let _ = write_media_characteristics_report(&mut rendered, &report, format);
    }
}

fn exercise_track_metrics(bytes: &[u8], tracks: &[TrackInfo], segments: &[SegmentInfo]) {
    for track in tracks {
        let _ = average_sample_bitrate(&track.samples, track.timescale);
        let _ = max_sample_bitrate(&track.samples, track.timescale, 1);
        let _ = average_segment_bitrate(segments, track.track_id, track.timescale);
        let _ = max_segment_bitrate(segments, track.track_id, track.timescale);

        if track.avc.is_some() && !track.samples.is_empty() && !track.chunks.is_empty() {
            let _ = find_idr_frames(&mut Cursor::new(bytes), track);
        }
    }
}

fn take_probe_format(input: &mut FuzzInput<'_>) -> ProbeFormat {
    if input.take_bool() {
        ProbeFormat::Json
    } else {
        ProbeFormat::Yaml
    }
}

fn take_probe_options(input: &mut FuzzInput<'_>) -> ProbeOptions {
    ProbeOptions {
        expand_samples: input.take_bool(),
        expand_chunks: input.take_bool(),
        include_segments: input.take_bool(),
    }
}

fn take_report_options(input: &mut FuzzInput<'_>) -> ProbeReportOptions {
    ProbeReportOptions {
        probe: take_probe_options(input),
        include_bitrate: input.take_bool(),
        include_idr_frame_count: input.take_bool(),
    }
}
