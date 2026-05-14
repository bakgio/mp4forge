//! Fragmented-file split command support.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{write_error_line, write_warning_lines};
use crate::FourCc;
use crate::boxes::iso14496_12::{Tfhd, Tkhd};
use crate::extract::{ExtractError, extract_boxes_with_payload};
use crate::header::{BoxInfo, HeaderError};
use crate::probe::{
    DetailedProbeInfo, DetailedTrackInfo, FragmentedTrackWarningDiagnostics, ProbeError,
    TrackCodecFamily, fragmented_track_warning_diagnostics, probe_detailed,
};
use crate::walk::BoxPath;
use crate::writer::{Writer, WriterError};

const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const MFRA: FourCc = FourCc::from_bytes(*b"mfra");
const PSSH: FourCc = FourCc::from_bytes(*b"pssh");
const SIDX: FourCc = FourCc::from_bytes(*b"sidx");
const TRAK: FourCc = FourCc::from_bytes(*b"trak");
const TKHD: FourCc = FourCc::from_bytes(*b"tkhd");
const TFHD: FourCc = FourCc::from_bytes(*b"tfhd");
const AVC1: FourCc = FourCc::from_bytes(*b"avc1");
const HEV1: FourCc = FourCc::from_bytes(*b"hev1");
const HVC1: FourCc = FourCc::from_bytes(*b"hvc1");
const DVHE: FourCc = FourCc::from_bytes(*b"dvhe");
const DVH1: FourCc = FourCc::from_bytes(*b"dvh1");
const AV01: FourCc = FourCc::from_bytes(*b"av01");
const VP08: FourCc = FourCc::from_bytes(*b"vp08");
const VP09: FourCc = FourCc::from_bytes(*b"vp09");
const MP4A: FourCc = FourCc::from_bytes(*b"mp4a");
const OPUS: FourCc = FourCc::from_bytes(*b"Opus");
const AC_3: FourCc = FourCc::from_bytes(*b"ac-3");
const EC_3: FourCc = FourCc::from_bytes(*b"ec-3");
const AC_4: FourCc = FourCc::from_bytes(*b"ac-4");
const ALAC: FourCc = FourCc::from_bytes(*b"alac");
const DTSC: FourCc = FourCc::from_bytes(*b"dtsc");
const DTSE: FourCc = FourCc::from_bytes(*b"dtse");
const DTSH: FourCc = FourCc::from_bytes(*b"dtsh");
const DTSL: FourCc = FourCc::from_bytes(*b"dtsl");
const DTSM: FourCc = FourCc::from_bytes(*b"dtsm");
const DTSX: FourCc = FourCc::from_bytes(*b"dtsx");
const DTSY: FourCc = FourCc::from_bytes(*b"dtsy");
const FLAC: FourCc = FourCc::from_bytes(*b"fLaC");
const IAMF: FourCc = FourCc::from_bytes(*b"iamf");
const MHA1: FourCc = FourCc::from_bytes(*b"mha1");
const MHA2: FourCc = FourCc::from_bytes(*b"mha2");
const MHM1: FourCc = FourCc::from_bytes(*b"mhm1");
const MHM2: FourCc = FourCc::from_bytes(*b"mhm2");
const IPCM: FourCc = FourCc::from_bytes(*b"ipcm");
const FPCM: FourCc = FourCc::from_bytes(*b"fpcm");

const VIDEO_DIR: &str = "video";
const AUDIO_DIR: &str = "audio";
const VIDEO_ENC_DIR: &str = "video_enc";
const AUDIO_ENC_DIR: &str = "audio_enc";
const INIT_FILE_NAME: &str = "init.mp4";
const PLAYLIST_FILE_NAME: &str = "playlist.m3u8";
const MANIFEST_FILE_NAME: &str = "manifest.mpd";
const HLS_PLAYLIST_EXTENSION: &str = ".m3u8";
const DASH_MANIFEST_EXTENSION: &str = ".mpd";
const DEFAULT_DASH_MIN_BUFFER_TIME_MICROS: u64 = 2_000_000;
const DEFAULT_DASH_MINIMUM_UPDATE_PERIOD_MICROS: u64 = 5_000_000;

/// Output-manifest families supported by the additive divide surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DivideManifestSelection {
    Hls,
    Dash,
    Both,
}

/// DASH manifest modes supported by the additive divide surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashManifestMode {
    Static,
    Dynamic,
}

/// DASH manifest layout families supported by the additive divide surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashManifestLayout {
    Template,
    List,
}

/// DASH manifest profile signaling supported by the additive divide surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashManifestProfile {
    Main,
    Live,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HlsPlaylistType {
    Vod,
    Event,
    Live,
}

/// Additive divide output controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivideOutputOptions {
    pub manifest_selection: DivideManifestSelection,
    pub dash_manifest_mode: DashManifestMode,
    pub dash_manifest_layout: DashManifestLayout,
    pub dash_manifest_profile: DashManifestProfile,
    pub default_language: Option<String>,
    pub hls_base_url: Option<String>,
    pub hls_playlist_type: Option<HlsPlaylistType>,
    pub hls_start_time_offset_micros: Option<i64>,
    pub hls_program_date_time: bool,
    pub hls_master_playlist_name: Option<String>,
    pub hls_media_playlist_name: Option<String>,
    pub dash_period_id: Option<String>,
    pub dash_period_start_micros: Option<u64>,
    pub dash_base_urls: Vec<String>,
    pub dash_manifest_name: Option<String>,
    pub dash_location: Option<String>,
    pub dash_min_buffer_time_micros: Option<u64>,
    pub dash_minimum_update_period_micros: Option<u64>,
    pub dash_suggested_presentation_delay_micros: Option<u64>,
    pub dash_time_shift_buffer_depth_micros: Option<u64>,
    pub dash_availability_start_time: Option<String>,
    pub dash_publish_time: Option<String>,
    pub dash_utc_timing_scheme: Option<String>,
    pub dash_utc_timing_value: Option<String>,
    pub dash_session_load_path: Option<PathBuf>,
    pub dash_session_save_path: Option<PathBuf>,
    dash_session_next_segment_indices: BTreeMap<u32, usize>,
    manifest_selection_explicit: bool,
    dash_manifest_mode_explicit: bool,
    dash_manifest_layout_explicit: bool,
    dash_manifest_profile_explicit: bool,
}

impl Default for DivideOutputOptions {
    fn default() -> Self {
        Self {
            manifest_selection: DivideManifestSelection::Both,
            dash_manifest_mode: DashManifestMode::Static,
            dash_manifest_layout: DashManifestLayout::Template,
            dash_manifest_profile: DashManifestProfile::Main,
            default_language: None,
            hls_base_url: None,
            hls_playlist_type: None,
            hls_start_time_offset_micros: None,
            hls_program_date_time: false,
            hls_master_playlist_name: None,
            hls_media_playlist_name: None,
            dash_period_id: None,
            dash_period_start_micros: None,
            dash_base_urls: Vec::new(),
            dash_manifest_name: None,
            dash_location: None,
            dash_min_buffer_time_micros: None,
            dash_minimum_update_period_micros: None,
            dash_suggested_presentation_delay_micros: None,
            dash_time_shift_buffer_depth_micros: None,
            dash_availability_start_time: None,
            dash_publish_time: None,
            dash_utc_timing_scheme: None,
            dash_utc_timing_value: None,
            dash_session_load_path: None,
            dash_session_save_path: None,
            dash_session_next_segment_indices: BTreeMap::new(),
            manifest_selection_explicit: false,
            dash_manifest_mode_explicit: false,
            dash_manifest_layout_explicit: false,
            dash_manifest_profile_explicit: false,
        }
    }
}

/// Runs the divide subcommand with `args`, writing files under `OUTPUT_DIR`.
pub fn run<E>(args: &[String], stderr: &mut E) -> i32
where
    E: Write,
{
    let mut stdout = io::sink();
    run_with_output(args, &mut stdout, stderr)
}

/// Runs the divide subcommand with `args`, writing validation output to `stdout` when requested
/// and errors to `stderr`.
pub fn run_with_output<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    match run_inner(args, stdout, stderr) {
        Ok(()) => 0,
        Err(DivideError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = write_error_line(stderr, &error, error.diagnostic_context());
            1
        }
    }
}

/// Writes the divide subcommand usage text.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(
        writer,
        "USAGE: mp4forge divide [OPTIONS] INPUT.mp4 OUTPUT_DIR"
    )?;
    writeln!(writer, "       mp4forge divide -validate INPUT.mp4")?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  -validate    Validate the fragmented divide layout without writing output files"
    )?;
    writeln!(
        writer,
        "  -warnings    Emit warning-grade diagnostics to stderr after a successful run"
    )?;
    writeln!(
        writer,
        "  -manifest <hls|dash|both>  Manifest families to write (default: both)"
    )?;
    writeln!(
        writer,
        "  -default-language <tag>  Prefer this audio language in HLS defaults and DASH main-role signaling"
    )?;
    writeln!(
        writer,
        "  -hls-base-url <url>  Prefix HLS playlist, init, and media segment URIs"
    )?;
    writeln!(
        writer,
        "  -hls-playlist-type <vod|event|live>  HLS playlist style (default: vod)"
    )?;
    writeln!(
        writer,
        "  -hls-start-time-offset <seconds>  Add EXT-X-START with a signed seconds offset"
    )?;
    writeln!(
        writer,
        "  -hls-program-date-time  Add EXT-X-PROGRAM-DATE-TIME to HLS media playlists"
    )?;
    writeln!(
        writer,
        "  -hls-master-playlist-name <name.m3u8>  Override the root HLS master playlist file name"
    )?;
    writeln!(
        writer,
        "  -hls-media-playlist-name <name.m3u8>  Override per-track HLS media playlist file names"
    )?;
    writeln!(
        writer,
        "  -dash-mode <static|dynamic>  DASH manifest mode (default: static)"
    )?;
    writeln!(
        writer,
        "  -dash-layout <template|list>  DASH manifest layout (default: template)"
    )?;
    writeln!(
        writer,
        "  -dash-profile <main|live>  DASH profile signaling (default: main)"
    )?;
    writeln!(
        writer,
        "  -dash-base-url <url>  Add one DASH BaseURL element (repeatable)"
    )?;
    writeln!(
        writer,
        "  -dash-manifest-name <name.mpd>  Override the root DASH manifest file name"
    )?;
    writeln!(
        writer,
        "  -dash-session-load <path>  Reload saved DASH session controls and next-period continuity"
    )?;
    writeln!(
        writer,
        "  -dash-session-save <path>  Save DASH session controls and next-period continuity"
    )?;
    writeln!(writer)?;
    writeln!(
        writer,
        "Successful output writes the selected retained HLS playlist tree and/or additive MPD manifest."
    )?;
    writeln!(
        writer,
        "DASH metadata such as Period ids, timing descriptors, and dynamic refresh attributes use built-in defaults."
    )?;
    writeln!(writer)?;
    writeln!(
        writer,
        "Currently supports fragmented inputs with up to one video track from AVC, HEVC, Dolby Vision on HEVC, AV1, VP8, or VP9"
    )?;
    writeln!(
        writer,
        "and one or more audio tracks from MP4A-based audio, Opus, AC-3, E-AC-3, AC-4, ALAC, DTS-family entries, FLAC, IAMF, MPEG-H, or PCM,"
    )?;
    writeln!(
        writer,
        "including encrypted wrappers that preserve those original sample-entry formats. Subtitle and text tracks remain unsupported."
    )
}

#[derive(Debug)]
struct ParsedDivideArgs<'a> {
    validate_only: bool,
    emit_warnings: bool,
    input_path: &'a Path,
    output_dir: Option<&'a Path>,
    output_options: DivideOutputOptions,
}

fn run_inner<W, E>(args: &[String], stdout: &mut W, stderr: &mut E) -> Result<(), DivideError>
where
    W: Write,
    E: Write,
{
    let parsed = parse_args(args)?;
    let output_options =
        resolve_divide_output_options(parsed.output_options, parsed.validate_only)?;
    let mut input = File::open(parsed.input_path)?;
    let (warning_plans, warning_lines) = if parsed.emit_warnings {
        let plans = validate_divide_track_plans(&mut input)?;
        let warnings = collect_fragmented_warning_lines(&mut input, &plans, &output_options)?;
        input.seek(SeekFrom::Start(0))?;
        (Some(plans), Some(warnings))
    } else {
        (None, None)
    };

    if parsed.validate_only {
        let report = if let Some(plans) = warning_plans.as_ref() {
            build_divide_validation_report(plans)
        } else {
            validate_divide_reader(&mut input)?
        };
        write_validation_report(stdout, &report)?;
        if let Some(warnings) = warning_lines.as_ref() {
            write_warning_lines(stderr, warnings)?;
        }
        return Ok(());
    }

    input.seek(SeekFrom::Start(0))?;
    divide_reader_with_options(
        &mut input,
        parsed.output_dir.ok_or(DivideError::UsageRequested)?,
        output_options.clone(),
    )?;

    if let Some(warnings) = warning_lines.as_ref() {
        write_warning_lines(stderr, warnings)?;
    }

    Ok(())
}

fn parse_args(args: &[String]) -> Result<ParsedDivideArgs<'_>, DivideError> {
    let mut validate_only = false;
    let mut emit_warnings = false;
    let mut output_options = DivideOutputOptions::default();
    let mut positional = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-validate" | "--validate" => {
                validate_only = true;
                index += 1;
            }
            "-warnings" | "--warnings" => {
                emit_warnings = true;
                index += 1;
            }
            "-manifest" | "--manifest" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-manifest`".to_string(),
                    ));
                };
                output_options.manifest_selection = match value.as_str() {
                    "hls" => DivideManifestSelection::Hls,
                    "dash" => DivideManifestSelection::Dash,
                    "both" => DivideManifestSelection::Both,
                    other => {
                        return Err(invalid_input(format!(
                            "unsupported divide manifest selection: {other}"
                        )));
                    }
                };
                output_options.manifest_selection_explicit = true;
                index += 1;
            }
            "-default-language" | "--default-language" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-default-language`".to_string(),
                    ));
                };
                output_options.default_language = Some(parse_divide_language_tag(value)?);
                index += 1;
            }
            "-hls-base-url" | "--hls-base-url" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-hls-base-url`".to_string(),
                    ));
                };
                output_options.hls_base_url = Some(value.clone());
                index += 1;
            }
            "-hls-playlist-type" | "--hls-playlist-type" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-hls-playlist-type`".to_string(),
                    ));
                };
                output_options.hls_playlist_type = Some(match value.as_str() {
                    "vod" => HlsPlaylistType::Vod,
                    "event" => HlsPlaylistType::Event,
                    "live" => HlsPlaylistType::Live,
                    other => {
                        return Err(invalid_input(format!(
                            "unsupported divide HLS playlist type: {other}"
                        )));
                    }
                });
                index += 1;
            }
            "-hls-start-time-offset" | "--hls-start-time-offset" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-hls-start-time-offset`".to_string(),
                    ));
                };
                output_options.hls_start_time_offset_micros =
                    Some(parse_hls_start_time_offset_micros(value)?);
                index += 1;
            }
            "-hls-program-date-time" | "--hls-program-date-time" => {
                output_options.hls_program_date_time = true;
                index += 1;
            }
            "-hls-master-playlist-name" | "--hls-master-playlist-name" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-hls-master-playlist-name`".to_string(),
                    ));
                };
                output_options.hls_master_playlist_name = Some(value.clone());
                index += 1;
            }
            "-hls-media-playlist-name" | "--hls-media-playlist-name" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-hls-media-playlist-name`".to_string(),
                    ));
                };
                output_options.hls_media_playlist_name = Some(value.clone());
                index += 1;
            }
            "-dash-mode" | "--dash-mode" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-dash-mode`".to_string(),
                    ));
                };
                output_options.dash_manifest_mode = match value.as_str() {
                    "static" => DashManifestMode::Static,
                    "dynamic" => DashManifestMode::Dynamic,
                    other => {
                        return Err(invalid_input(format!(
                            "unsupported divide DASH mode: {other}"
                        )));
                    }
                };
                output_options.dash_manifest_mode_explicit = true;
                index += 1;
            }
            "-dash-layout" | "--dash-layout" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-dash-layout`".to_string(),
                    ));
                };
                output_options.dash_manifest_layout = match value.as_str() {
                    "template" => DashManifestLayout::Template,
                    "list" => DashManifestLayout::List,
                    other => {
                        return Err(invalid_input(format!(
                            "unsupported divide DASH layout: {other}"
                        )));
                    }
                };
                output_options.dash_manifest_layout_explicit = true;
                index += 1;
            }
            "-dash-profile" | "--dash-profile" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-dash-profile`".to_string(),
                    ));
                };
                output_options.dash_manifest_profile = match value.as_str() {
                    "main" => DashManifestProfile::Main,
                    "live" => DashManifestProfile::Live,
                    other => {
                        return Err(invalid_input(format!(
                            "unsupported divide DASH profile: {other}"
                        )));
                    }
                };
                output_options.dash_manifest_profile_explicit = true;
                index += 1;
            }
            "-dash-base-url" | "--dash-base-url" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-dash-base-url`".to_string(),
                    ));
                };
                output_options.dash_base_urls.push(value.clone());
                index += 1;
            }
            "-dash-manifest-name" | "--dash-manifest-name" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-dash-manifest-name`".to_string(),
                    ));
                };
                output_options.dash_manifest_name = Some(value.clone());
                index += 1;
            }
            "-dash-session-load" | "--dash-session-load" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-dash-session-load`".to_string(),
                    ));
                };
                output_options.dash_session_load_path = Some(PathBuf::from(value));
                index += 1;
            }
            "-dash-session-save" | "--dash-session-save" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(invalid_input(
                        "missing value for divide option `-dash-session-save`".to_string(),
                    ));
                };
                output_options.dash_session_save_path = Some(PathBuf::from(value));
                index += 1;
            }
            value if removed_dash_option_message(value).is_some() => {
                return Err(invalid_input(
                    removed_dash_option_message(value)
                        .expect("checked above")
                        .to_string(),
                ));
            }
            "-h" | "--help" => return Err(DivideError::UsageRequested),
            value if value.starts_with('-') => {
                return Err(invalid_input(format!("unknown divide option: {value}")));
            }
            value => {
                positional.push(Path::new(value));
                index += 1;
            }
        }
    }

    match (validate_only, positional.as_slice()) {
        (true, [input_path]) => Ok(ParsedDivideArgs {
            validate_only,
            emit_warnings,
            input_path,
            output_dir: None,
            output_options,
        }),
        (false, [input_path, output_dir]) => Ok(ParsedDivideArgs {
            validate_only,
            emit_warnings,
            input_path,
            output_dir: Some(output_dir),
            output_options,
        }),
        _ => Err(DivideError::UsageRequested),
    }
}

fn removed_dash_option_message(option: &str) -> Option<&'static str> {
    match option {
        "-dash-period-id" | "--dash-period-id" => Some(
            "divide option `-dash-period-id` was removed; mp4forge now derives DASH Period identifiers internally when needed",
        ),
        "-dash-period-start" | "--dash-period-start" => Some(
            "divide option `-dash-period-start` was removed; mp4forge now derives DASH Period start timing internally",
        ),
        "-dash-location" | "--dash-location" => Some(
            "divide option `-dash-location` was removed; mp4forge now omits DASH Location unless the library API sets one explicitly",
        ),
        "-dash-min-buffer-time" | "--dash-min-buffer-time" => Some(
            "divide option `-dash-min-buffer-time` was removed; mp4forge now uses built-in DASH minBufferTime defaults",
        ),
        "-dash-minimum-update-period" | "--dash-minimum-update-period" => Some(
            "divide option `-dash-minimum-update-period` was removed; mp4forge now uses built-in DASH minimumUpdatePeriod defaults",
        ),
        "-dash-suggested-presentation-delay" | "--dash-suggested-presentation-delay" => Some(
            "divide option `-dash-suggested-presentation-delay` was removed; mp4forge now uses built-in DASH suggestedPresentationDelay defaults",
        ),
        "-dash-time-shift-buffer-depth" | "--dash-time-shift-buffer-depth" => Some(
            "divide option `-dash-time-shift-buffer-depth` was removed; mp4forge now uses built-in DASH timeShiftBufferDepth defaults",
        ),
        "-dash-availability-start-time" | "--dash-availability-start-time" => Some(
            "divide option `-dash-availability-start-time` was removed; mp4forge now derives DASH availabilityStartTime internally",
        ),
        "-dash-publish-time" | "--dash-publish-time" => Some(
            "divide option `-dash-publish-time` was removed; mp4forge now derives DASH publishTime internally",
        ),
        "-dash-utc-timing-scheme" | "--dash-utc-timing-scheme" => Some(
            "divide option `-dash-utc-timing-scheme` was removed; mp4forge now omits DASH UTCTiming unless the library API sets it explicitly",
        ),
        "-dash-utc-timing-value" | "--dash-utc-timing-value" => Some(
            "divide option `-dash-utc-timing-value` was removed; mp4forge now omits DASH UTCTiming unless the library API sets it explicitly",
        ),
        _ => None,
    }
}

#[derive(Default)]
struct DashSessionState {
    next_period_id: Option<String>,
    next_period_start_micros: Option<u64>,
    manifest_selection: Option<DivideManifestSelection>,
    dash_manifest_mode: Option<DashManifestMode>,
    dash_manifest_layout: Option<DashManifestLayout>,
    dash_manifest_profile: Option<DashManifestProfile>,
    dash_base_urls: Vec<String>,
    dash_location: Option<String>,
    dash_min_buffer_time_micros: Option<u64>,
    dash_minimum_update_period_micros: Option<u64>,
    dash_suggested_presentation_delay_micros: Option<u64>,
    dash_time_shift_buffer_depth_micros: Option<u64>,
    dash_availability_start_time: Option<String>,
    dash_publish_time: Option<String>,
    dash_utc_timing_scheme: Option<String>,
    dash_utc_timing_value: Option<String>,
    next_segment_indices: BTreeMap<u32, usize>,
}

struct DashManifestOutcome {
    total_duration_micros: u64,
    period_start_micros: u64,
}

fn resolve_divide_output_options(
    mut output_options: DivideOutputOptions,
    validate_only: bool,
) -> Result<DivideOutputOptions, DivideError> {
    validate_divide_output_request_shape(&output_options, validate_only)?;
    if let Some(path) = output_options.dash_session_load_path.clone() {
        let state = load_dash_session_state(&path)?;
        apply_dash_session_state(&mut output_options, &state);
    }
    validate_divide_output_request_shape(&output_options, validate_only)?;
    validate_dash_utc_timing_pair(&output_options)?;
    Ok(output_options)
}

fn apply_dash_session_state(output_options: &mut DivideOutputOptions, state: &DashSessionState) {
    if output_options.dash_period_id.is_none() {
        output_options.dash_period_id = state.next_period_id.clone();
    }
    if output_options.dash_period_start_micros.is_none() {
        output_options.dash_period_start_micros = state.next_period_start_micros;
    }
    if !output_options.manifest_selection_explicit
        && let Some(manifest_selection) = state.manifest_selection
    {
        output_options.manifest_selection = manifest_selection;
    }
    if !output_options.dash_manifest_mode_explicit
        && let Some(dash_manifest_mode) = state.dash_manifest_mode
    {
        output_options.dash_manifest_mode = dash_manifest_mode;
    }
    if !output_options.dash_manifest_layout_explicit
        && let Some(dash_manifest_layout) = state.dash_manifest_layout
    {
        output_options.dash_manifest_layout = dash_manifest_layout;
    }
    if !output_options.dash_manifest_profile_explicit
        && let Some(dash_manifest_profile) = state.dash_manifest_profile
    {
        output_options.dash_manifest_profile = dash_manifest_profile;
    }
    if output_options.dash_base_urls.is_empty() {
        output_options.dash_base_urls = state.dash_base_urls.clone();
    }
    if output_options.dash_location.is_none() {
        output_options.dash_location = state.dash_location.clone();
    }
    if output_options.dash_min_buffer_time_micros.is_none() {
        output_options.dash_min_buffer_time_micros = state.dash_min_buffer_time_micros;
    }
    if output_options.dash_minimum_update_period_micros.is_none() {
        output_options.dash_minimum_update_period_micros = state.dash_minimum_update_period_micros;
    }
    if output_options
        .dash_suggested_presentation_delay_micros
        .is_none()
    {
        output_options.dash_suggested_presentation_delay_micros =
            state.dash_suggested_presentation_delay_micros;
    }
    if output_options.dash_time_shift_buffer_depth_micros.is_none() {
        output_options.dash_time_shift_buffer_depth_micros =
            state.dash_time_shift_buffer_depth_micros;
    }
    if output_options.dash_availability_start_time.is_none() {
        output_options.dash_availability_start_time = state.dash_availability_start_time.clone();
    }
    if output_options.dash_publish_time.is_none() {
        output_options.dash_publish_time = state.dash_publish_time.clone();
    }
    if output_options.dash_utc_timing_scheme.is_none() {
        output_options.dash_utc_timing_scheme = state.dash_utc_timing_scheme.clone();
    }
    if output_options.dash_utc_timing_value.is_none() {
        output_options.dash_utc_timing_value = state.dash_utc_timing_value.clone();
    }
    if output_options.dash_session_next_segment_indices.is_empty() {
        output_options.dash_session_next_segment_indices = state.next_segment_indices.clone();
    }
}

fn validate_dash_utc_timing_pair(output_options: &DivideOutputOptions) -> Result<(), DivideError> {
    match (
        output_options.dash_utc_timing_scheme.as_ref(),
        output_options.dash_utc_timing_value.as_ref(),
    ) {
        (Some(_), None) | (None, Some(_)) => Err(invalid_input(
            "divide DASH UTCTiming requires both `-dash-utc-timing-scheme` and `-dash-utc-timing-value`".to_string(),
        )),
        _ => Ok(()),
    }
}

fn validate_divide_output_request_shape(
    output_options: &DivideOutputOptions,
    validate_only: bool,
) -> Result<(), DivideError> {
    validate_divide_output_name_constraints(output_options)?;
    validate_hls_manifest_selection(output_options)?;
    validate_dash_manifest_selection(output_options)?;
    validate_dash_mode_constraints(output_options)?;
    validate_dash_session_constraints(output_options, validate_only)?;
    Ok(())
}

fn validate_divide_output_name_constraints(
    output_options: &DivideOutputOptions,
) -> Result<(), DivideError> {
    if let Some(name) = output_options.hls_master_playlist_name.as_deref() {
        validate_divide_output_file_name(
            name,
            "-hls-master-playlist-name",
            HLS_PLAYLIST_EXTENSION,
        )?;
    }
    if let Some(name) = output_options.hls_media_playlist_name.as_deref() {
        validate_divide_output_file_name(name, "-hls-media-playlist-name", HLS_PLAYLIST_EXTENSION)?;
    }
    if let Some(name) = output_options.dash_manifest_name.as_deref() {
        validate_divide_output_file_name(name, "-dash-manifest-name", DASH_MANIFEST_EXTENSION)?;
    }
    Ok(())
}

fn validate_divide_output_file_name(
    value: &str,
    option: &str,
    required_extension: &str,
) -> Result<(), DivideError> {
    if value.is_empty()
        || Path::new(value).components().count() != 1
        || !value.ends_with(required_extension)
    {
        return Err(invalid_input(format!(
            "divide option `{option}` requires a plain `{required_extension}` file name: `{value}`"
        )));
    }
    Ok(())
}

fn validate_hls_manifest_selection(
    output_options: &DivideOutputOptions,
) -> Result<(), DivideError> {
    if output_options.manifest_selection != DivideManifestSelection::Dash {
        return Ok(());
    }

    let unsupported_option = [
        (output_options.hls_base_url.is_some(), "-hls-base-url"),
        (
            output_options.hls_playlist_type.is_some(),
            "-hls-playlist-type",
        ),
        (
            output_options.hls_start_time_offset_micros.is_some(),
            "-hls-start-time-offset",
        ),
        (
            output_options.hls_program_date_time,
            "-hls-program-date-time",
        ),
        (
            output_options.hls_master_playlist_name.is_some(),
            "-hls-master-playlist-name",
        ),
        (
            output_options.hls_media_playlist_name.is_some(),
            "-hls-media-playlist-name",
        ),
    ]
    .into_iter()
    .find_map(|(present, option)| present.then_some(option));

    if let Some(option) = unsupported_option {
        return Err(invalid_input(format!(
            "divide manifest selection `dash` does not support `{option}`; use `-manifest hls` or `-manifest both`"
        )));
    }

    Ok(())
}

fn validate_dash_manifest_selection(
    output_options: &DivideOutputOptions,
) -> Result<(), DivideError> {
    if output_options.manifest_selection != DivideManifestSelection::Hls {
        return Ok(());
    }

    if output_options.dash_manifest_mode_explicit {
        return Err(invalid_input(
            "divide manifest selection `hls` does not support `-dash-mode`; use `-manifest dash` or `-manifest both`".to_string(),
        ));
    }
    if output_options.dash_manifest_layout_explicit {
        return Err(invalid_input(
            "divide manifest selection `hls` does not support `-dash-layout`; use `-manifest dash` or `-manifest both`".to_string(),
        ));
    }
    if output_options.dash_manifest_profile_explicit {
        return Err(invalid_input(
            "divide manifest selection `hls` does not support `-dash-profile`; use `-manifest dash` or `-manifest both`".to_string(),
        ));
    }

    let unsupported_option = [
        (output_options.dash_period_id.is_some(), "-dash-period-id"),
        (
            output_options.dash_period_start_micros.is_some(),
            "-dash-period-start",
        ),
        (!output_options.dash_base_urls.is_empty(), "-dash-base-url"),
        (
            output_options.dash_manifest_name.is_some(),
            "-dash-manifest-name",
        ),
        (output_options.dash_location.is_some(), "-dash-location"),
        (
            output_options.dash_min_buffer_time_micros.is_some(),
            "-dash-min-buffer-time",
        ),
        (
            output_options.dash_minimum_update_period_micros.is_some(),
            "-dash-minimum-update-period",
        ),
        (
            output_options
                .dash_suggested_presentation_delay_micros
                .is_some(),
            "-dash-suggested-presentation-delay",
        ),
        (
            output_options.dash_time_shift_buffer_depth_micros.is_some(),
            "-dash-time-shift-buffer-depth",
        ),
        (
            output_options.dash_availability_start_time.is_some(),
            "-dash-availability-start-time",
        ),
        (
            output_options.dash_publish_time.is_some(),
            "-dash-publish-time",
        ),
        (
            output_options.dash_utc_timing_scheme.is_some(),
            "-dash-utc-timing-scheme",
        ),
        (
            output_options.dash_utc_timing_value.is_some(),
            "-dash-utc-timing-value",
        ),
        (
            output_options.dash_session_load_path.is_some(),
            "-dash-session-load",
        ),
        (
            output_options.dash_session_save_path.is_some(),
            "-dash-session-save",
        ),
    ]
    .into_iter()
    .find_map(|(present, option)| present.then_some(option));

    if let Some(option) = unsupported_option {
        return Err(invalid_input(format!(
            "divide manifest selection `hls` does not support `{option}`; use `-manifest dash` or `-manifest both`"
        )));
    }

    Ok(())
}

fn validate_dash_mode_constraints(output_options: &DivideOutputOptions) -> Result<(), DivideError> {
    if output_options.dash_manifest_mode == DashManifestMode::Dynamic {
        return Ok(());
    }

    let dynamic_only_option = [
        (
            output_options.dash_minimum_update_period_micros.is_some(),
            "-dash-minimum-update-period",
        ),
        (
            output_options
                .dash_suggested_presentation_delay_micros
                .is_some(),
            "-dash-suggested-presentation-delay",
        ),
        (
            output_options.dash_time_shift_buffer_depth_micros.is_some(),
            "-dash-time-shift-buffer-depth",
        ),
        (
            output_options.dash_availability_start_time.is_some(),
            "-dash-availability-start-time",
        ),
        (
            output_options.dash_publish_time.is_some(),
            "-dash-publish-time",
        ),
    ]
    .into_iter()
    .find_map(|(present, option)| present.then_some(option));

    if let Some(option) = dynamic_only_option {
        return Err(invalid_input(format!(
            "divide DASH mode `static` does not support `{option}`; use `-dash-mode dynamic`"
        )));
    }

    if output_options.dash_manifest_profile == DashManifestProfile::Live {
        return Err(invalid_input(
            "divide DASH profile `live` requires `-dash-mode dynamic`".to_string(),
        ));
    }

    Ok(())
}

fn validate_dash_session_constraints(
    output_options: &DivideOutputOptions,
    validate_only: bool,
) -> Result<(), DivideError> {
    if validate_only && output_options.dash_session_load_path.is_some() {
        return Err(invalid_input(
            "divide validation mode does not support `-dash-session-load`".to_string(),
        ));
    }
    if validate_only && output_options.dash_session_save_path.is_some() {
        return Err(invalid_input(
            "divide validation mode does not support `-dash-session-save`".to_string(),
        ));
    }

    if let (Some(load_path), Some(save_path)) = (
        output_options.dash_session_load_path.as_ref(),
        output_options.dash_session_save_path.as_ref(),
    ) && load_path == save_path
    {
        return Err(invalid_input(format!(
            "divide DASH session load and save paths must differ: `{}`",
            load_path.display()
        )));
    }

    Ok(())
}

fn load_dash_session_state(path: &Path) -> Result<DashSessionState, DivideError> {
    let contents = fs::read_to_string(path)?;
    let mut state = DashSessionState::default();
    for (line_index, raw_line) in contents.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(invalid_input(format!(
                "invalid divide session state line {} in `{}`",
                line_index + 1,
                path.display()
            )));
        };
        match key {
            "next_period_id" => state.next_period_id = Some(value.to_string()),
            "next_period_start_micros" => {
                state.next_period_start_micros =
                    Some(parse_session_micros(value, path, line_index + 1)?)
            }
            "manifest_selection" => {
                state.manifest_selection = Some(parse_dash_session_manifest_selection(
                    value,
                    path,
                    line_index + 1,
                )?)
            }
            "dash_manifest_mode" => {
                state.dash_manifest_mode =
                    Some(parse_dash_session_mode(value, path, line_index + 1)?)
            }
            "dash_manifest_layout" => {
                state.dash_manifest_layout =
                    Some(parse_dash_session_layout(value, path, line_index + 1)?)
            }
            "dash_manifest_profile" => {
                state.dash_manifest_profile =
                    Some(parse_dash_session_profile(value, path, line_index + 1)?)
            }
            "dash_base_url" => state.dash_base_urls.push(value.to_string()),
            "dash_location" => state.dash_location = Some(value.to_string()),
            "dash_min_buffer_time_micros" => {
                state.dash_min_buffer_time_micros =
                    Some(parse_session_micros(value, path, line_index + 1)?)
            }
            "dash_minimum_update_period_micros" => {
                state.dash_minimum_update_period_micros =
                    Some(parse_session_micros(value, path, line_index + 1)?)
            }
            "dash_suggested_presentation_delay_micros" => {
                state.dash_suggested_presentation_delay_micros =
                    Some(parse_session_micros(value, path, line_index + 1)?)
            }
            "dash_time_shift_buffer_depth_micros" => {
                state.dash_time_shift_buffer_depth_micros =
                    Some(parse_session_micros(value, path, line_index + 1)?)
            }
            "dash_availability_start_time" => {
                state.dash_availability_start_time = Some(value.to_string())
            }
            "dash_publish_time" => state.dash_publish_time = Some(value.to_string()),
            "dash_utc_timing_scheme" => state.dash_utc_timing_scheme = Some(value.to_string()),
            "dash_utc_timing_value" => state.dash_utc_timing_value = Some(value.to_string()),
            _ if key.starts_with("next_segment_index_track_") => {
                let track_id = key["next_segment_index_track_".len()..]
                    .parse::<u32>()
                    .map_err(|_| {
                        invalid_input(format!(
                            "invalid divide session state line {} in `{}`",
                            line_index + 1,
                            path.display()
                        ))
                    })?;
                let next_segment_index = value.parse::<usize>().map_err(|_| {
                    invalid_input(format!(
                        "invalid divide session state value on line {} in `{}`",
                        line_index + 1,
                        path.display()
                    ))
                })?;
                state
                    .next_segment_indices
                    .insert(track_id, next_segment_index);
            }
            _ => {}
        }
    }
    Ok(state)
}

fn parse_session_micros(value: &str, path: &Path, line_number: usize) -> Result<u64, DivideError> {
    value.parse::<u64>().map_err(|_| {
        invalid_input(format!(
            "invalid divide session state value on line {} in `{}`",
            line_number,
            path.display()
        ))
    })
}

fn parse_hls_start_time_offset_micros(value: &str) -> Result<i64, DivideError> {
    let seconds = value.parse::<f64>().map_err(|_| {
        invalid_input(format!(
            "invalid divide HLS start time offset seconds: {value}"
        ))
    })?;
    if !seconds.is_finite() {
        return Err(invalid_input(format!(
            "invalid divide HLS start time offset seconds: {value}"
        )));
    }
    let micros = (seconds * 1_000_000.0).round();
    if micros < i64::MIN as f64 || micros > i64::MAX as f64 {
        return Err(invalid_input(format!(
            "divide HLS start time offset is out of range: {value}"
        )));
    }
    Ok(micros as i64)
}

fn parse_dash_session_mode(
    value: &str,
    path: &Path,
    line_number: usize,
) -> Result<DashManifestMode, DivideError> {
    match value {
        "static" => Ok(DashManifestMode::Static),
        "dynamic" => Ok(DashManifestMode::Dynamic),
        _ => Err(invalid_input(format!(
            "invalid divide session state value on line {} in `{}`",
            line_number,
            path.display()
        ))),
    }
}

fn parse_dash_session_manifest_selection(
    value: &str,
    path: &Path,
    line_number: usize,
) -> Result<DivideManifestSelection, DivideError> {
    match value {
        "hls" => Ok(DivideManifestSelection::Hls),
        "dash" => Ok(DivideManifestSelection::Dash),
        "both" => Ok(DivideManifestSelection::Both),
        _ => Err(invalid_input(format!(
            "invalid divide session state value on line {} in `{}`",
            line_number,
            path.display()
        ))),
    }
}

fn parse_dash_session_layout(
    value: &str,
    path: &Path,
    line_number: usize,
) -> Result<DashManifestLayout, DivideError> {
    match value {
        "template" => Ok(DashManifestLayout::Template),
        "list" => Ok(DashManifestLayout::List),
        _ => Err(invalid_input(format!(
            "invalid divide session state value on line {} in `{}`",
            line_number,
            path.display()
        ))),
    }
}

fn parse_dash_session_profile(
    value: &str,
    path: &Path,
    line_number: usize,
) -> Result<DashManifestProfile, DivideError> {
    match value {
        "main" => Ok(DashManifestProfile::Main),
        "live" => Ok(DashManifestProfile::Live),
        _ => Err(invalid_input(format!(
            "invalid divide session state value on line {} in `{}`",
            line_number,
            path.display()
        ))),
    }
}

fn save_dash_session_state(path: &Path, state: &DashSessionState) -> Result<(), DivideError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    if let Some(next_period_id) = state.next_period_id.as_deref() {
        writeln!(file, "next_period_id={next_period_id}")?;
    }
    if let Some(next_period_start_micros) = state.next_period_start_micros {
        writeln!(file, "next_period_start_micros={next_period_start_micros}")?;
    }
    write_optional_state_string(
        &mut file,
        "manifest_selection",
        state.manifest_selection.map(divide_manifest_selection_name),
    )?;
    write_optional_state_string(
        &mut file,
        "dash_manifest_mode",
        state.dash_manifest_mode.map(dash_manifest_mode_name),
    )?;
    write_optional_state_string(
        &mut file,
        "dash_manifest_layout",
        state.dash_manifest_layout.map(dash_manifest_layout_name),
    )?;
    write_optional_state_string(
        &mut file,
        "dash_manifest_profile",
        state.dash_manifest_profile.map(dash_manifest_profile_name),
    )?;
    for base_url in &state.dash_base_urls {
        writeln!(file, "dash_base_url={base_url}")?;
    }
    write_optional_state_string(&mut file, "dash_location", state.dash_location.as_deref())?;
    write_optional_state_u64(
        &mut file,
        "dash_min_buffer_time_micros",
        state.dash_min_buffer_time_micros,
    )?;
    write_optional_state_u64(
        &mut file,
        "dash_minimum_update_period_micros",
        state.dash_minimum_update_period_micros,
    )?;
    write_optional_state_u64(
        &mut file,
        "dash_suggested_presentation_delay_micros",
        state.dash_suggested_presentation_delay_micros,
    )?;
    write_optional_state_u64(
        &mut file,
        "dash_time_shift_buffer_depth_micros",
        state.dash_time_shift_buffer_depth_micros,
    )?;
    write_optional_state_string(
        &mut file,
        "dash_availability_start_time",
        state.dash_availability_start_time.as_deref(),
    )?;
    write_optional_state_string(
        &mut file,
        "dash_publish_time",
        state.dash_publish_time.as_deref(),
    )?;
    write_optional_state_string(
        &mut file,
        "dash_utc_timing_scheme",
        state.dash_utc_timing_scheme.as_deref(),
    )?;
    write_optional_state_string(
        &mut file,
        "dash_utc_timing_value",
        state.dash_utc_timing_value.as_deref(),
    )?;
    for (track_id, next_segment_index) in &state.next_segment_indices {
        writeln!(
            file,
            "next_segment_index_track_{track_id}={next_segment_index}"
        )?;
    }
    Ok(())
}

fn write_optional_state_string(
    file: &mut File,
    key: &str,
    value: Option<&str>,
) -> Result<(), DivideError> {
    if let Some(value) = value {
        writeln!(file, "{key}={value}")?;
    }
    Ok(())
}

fn write_optional_state_u64(
    file: &mut File,
    key: &str,
    value: Option<u64>,
) -> Result<(), DivideError> {
    if let Some(value) = value {
        writeln!(file, "{key}={value}")?;
    }
    Ok(())
}

fn dash_manifest_mode_name(mode: DashManifestMode) -> &'static str {
    match mode {
        DashManifestMode::Static => "static",
        DashManifestMode::Dynamic => "dynamic",
    }
}

fn divide_manifest_selection_name(selection: DivideManifestSelection) -> &'static str {
    match selection {
        DivideManifestSelection::Hls => "hls",
        DivideManifestSelection::Dash => "dash",
        DivideManifestSelection::Both => "both",
    }
}

fn dash_manifest_layout_name(layout: DashManifestLayout) -> &'static str {
    match layout {
        DashManifestLayout::Template => "template",
        DashManifestLayout::List => "list",
    }
}

fn dash_manifest_profile_name(profile: DashManifestProfile) -> &'static str {
    match profile {
        DashManifestProfile::Main => "main",
        DashManifestProfile::Live => "live",
    }
}

/// Splits a fragmented MP4 reader into per-track outputs under `output_dir`.
///
/// The current `divide` surface supports fragmented inputs with at most one video track from AVC,
/// HEVC, Dolby Vision on HEVC, AV1, VP8, or VP9 and one or more audio tracks from MP4A-based audio, Opus,
/// AC-3, E-AC-3, AC-4, ALAC, DTS-family entries, FLAC, IAMF, MPEG-H, or PCM, including
/// encrypted `encv` and `enca` wrappers when the original format stays within that accepted
/// family set. Subtitle and text tracks remain unsupported in the current divide output model.
pub fn divide_reader<R>(reader: &mut R, output_dir: &Path) -> Result<(), DivideError>
where
    R: Read + Seek,
{
    divide_reader_with_options(reader, output_dir, DivideOutputOptions::default())
}

/// Splits a fragmented MP4 reader into per-track outputs under `output_dir` with additive
/// manifest controls.
pub fn divide_reader_with_options<R>(
    reader: &mut R,
    output_dir: &Path,
    output_options: DivideOutputOptions,
) -> Result<(), DivideError>
where
    R: Read + Seek,
{
    let plans = validate_divide_track_plans(reader)?;
    let mut tracks = build_track_outputs(&plans, output_dir, &output_options)?;

    reader.seek(SeekFrom::Start(0))?;
    write_init_segments(reader, &mut tracks)?;
    reader.seek(SeekFrom::Start(0))?;
    write_media_segments(reader, &mut tracks)?;
    write_output_manifests(output_dir, &tracks, output_options)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackKind {
    Video,
    Audio,
    EncryptedVideo,
    EncryptedAudio,
}

/// High-level role assigned to one active track in the currently supported divide layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DivideTrackRole {
    Video,
    Audio,
}

/// Validation summary for one active fragmented track accepted by `divide`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivideValidationTrack {
    /// Track identifier selected from `tkhd` and used by fragmented runs.
    pub track_id: u32,
    /// Role assigned by the current divide layout rules.
    pub role: DivideTrackRole,
    /// Whether the selected track uses an encrypted sample-entry wrapper.
    pub encrypted: bool,
    /// Normalized codec family derived from the sample entry or protected original format.
    pub codec_family: TrackCodecFamily,
    /// Sample-entry box type selected from `stsd`, including encrypted wrappers such as `encv`.
    pub sample_entry_type: Option<FourCc>,
    /// Original-format sample-entry type from `frma` when the track is protected.
    pub original_format: Option<FourCc>,
    /// Number of fragmented media segments currently associated with the track.
    pub segment_count: usize,
}

/// Additive divide preflight report returned when the fragmented layout is currently supported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivideValidationReport {
    /// Active fragmented tracks accepted by the current divide layout rules.
    pub tracks: Vec<DivideValidationTrack>,
}

struct TrackLayout {
    role: DivideTrackRole,
    kind: TrackKind,
    codecs: String,
    language: Option<String>,
    audio_channels: Option<u16>,
    sample_rate: Option<u16>,
    width: Option<u16>,
    height: Option<u16>,
}

struct TrackOutput {
    track_id: u32,
    kind: TrackKind,
    codecs: String,
    language: Option<String>,
    audio_channels: Option<u16>,
    sample_rate: Option<u16>,
    width: Option<u16>,
    height: Option<u16>,
    segment_durations: Vec<f64>,
    bandwidth: u64,
    relative_dir: String,
    output_dir: PathBuf,
    init_writer: Writer<File>,
    first_segment_index: usize,
    next_segment_index: usize,
}

struct ValidatedTrackPlan {
    validation: DivideValidationTrack,
    layout: TrackLayout,
    segment_durations: Vec<f64>,
}

/// Validates whether `reader` matches the fragmented layout currently supported by
/// [`divide_reader`] without creating any output files.
///
/// On success, the returned report lists the active fragmented tracks that would participate in
/// the divide output. On failure, the returned [`DivideError`] explains why the current layout is
/// unsupported.
pub fn validate_divide_reader<R>(reader: &mut R) -> Result<DivideValidationReport, DivideError>
where
    R: Read + Seek,
{
    let plans = validate_divide_track_plans(reader)?;
    Ok(build_divide_validation_report(&plans))
}

fn build_divide_validation_report(plans: &[ValidatedTrackPlan]) -> DivideValidationReport {
    DivideValidationReport {
        tracks: plans.iter().map(|plan| plan.validation.clone()).collect(),
    }
}

fn validate_divide_track_plans<R>(reader: &mut R) -> Result<Vec<ValidatedTrackPlan>, DivideError>
where
    R: Read + Seek,
{
    reader.seek(SeekFrom::Start(0))?;
    let summary = probe_detailed(reader)?;
    collect_track_plans(&summary)
}

fn build_track_outputs(
    plans: &[ValidatedTrackPlan],
    output_dir: &Path,
    output_options: &DivideOutputOptions,
) -> Result<BTreeMap<u32, TrackOutput>, DivideError> {
    let mut tracks = BTreeMap::new();
    let multiple_audio_tracks = plans
        .iter()
        .filter(|plan| plan.layout.role == DivideTrackRole::Audio)
        .count()
        > 1;

    for plan in plans {
        let relative_dir = track_relative_dir(
            plan.layout.kind,
            plan.validation.track_id,
            multiple_audio_tracks,
        );
        let track_dir = output_dir.join(&relative_dir);
        fs::create_dir_all(&track_dir)?;
        let init_writer = Writer::new(File::create(track_dir.join(INIT_FILE_NAME))?);
        let first_segment_index = output_options
            .dash_session_next_segment_indices
            .get(&plan.validation.track_id)
            .copied()
            .unwrap_or(0);

        tracks.insert(
            plan.validation.track_id,
            TrackOutput {
                track_id: plan.validation.track_id,
                kind: plan.layout.kind,
                codecs: plan.layout.codecs.clone(),
                language: plan.layout.language.clone(),
                audio_channels: plan.layout.audio_channels,
                sample_rate: plan.layout.sample_rate,
                width: plan.layout.width,
                height: plan.layout.height,
                segment_durations: plan.segment_durations.clone(),
                bandwidth: 0,
                relative_dir,
                output_dir: track_dir,
                init_writer,
                first_segment_index,
                next_segment_index: first_segment_index,
            },
        );
    }

    Ok(tracks)
}

fn collect_track_plans(
    summary: &DetailedProbeInfo,
) -> Result<Vec<ValidatedTrackPlan>, DivideError> {
    let active_track_ids = summary
        .segments
        .iter()
        .map(|segment| segment.track_id)
        .collect::<BTreeSet<_>>();
    let known_track_ids = summary
        .tracks
        .iter()
        .map(|track| track.summary.track_id)
        .collect::<BTreeSet<_>>();

    if let Some(track_id) = active_track_ids.difference(&known_track_ids).next() {
        return Err(DivideError::UnknownTrack(*track_id));
    }

    let mut tracks = BTreeMap::new();
    let mut selected_video_track_id = None;

    for track in &summary.tracks {
        if !active_track_ids.contains(&track.summary.track_id) {
            continue;
        }
        let layout = track_layout(track)?;
        match layout.role {
            DivideTrackRole::Video => {
                if let Some(existing_track_id) =
                    selected_video_track_id.replace(track.summary.track_id)
                {
                    return Err(invalid_input(format!(
                        "{}; found multiple fragmented video tracks ({existing_track_id} and {}).",
                        supported_scope_message(),
                        track.summary.track_id
                    )));
                }
            }
            DivideTrackRole::Audio => {}
        }

        let segment_durations = summary
            .segments
            .iter()
            .filter(|segment| segment.track_id == track.summary.track_id)
            .map(|segment| {
                if track.summary.timescale == 0 {
                    0.0
                } else {
                    segment.duration as f64 / f64::from(track.summary.timescale)
                }
            })
            .collect::<Vec<_>>();

        tracks.insert(
            track.summary.track_id,
            ValidatedTrackPlan {
                validation: DivideValidationTrack {
                    track_id: track.summary.track_id,
                    role: layout.role,
                    encrypted: track.summary.encrypted,
                    codec_family: track.codec_family,
                    sample_entry_type: track.sample_entry_type,
                    original_format: track.original_format,
                    segment_count: segment_durations.len(),
                },
                layout,
                segment_durations,
            },
        );
    }

    let plans = tracks.into_values().collect::<Vec<_>>();
    if plans.is_empty() {
        return Err(DivideError::NoSupportedTracks);
    }

    Ok(plans)
}

fn authoritative_track_format(track: &DetailedTrackInfo) -> Option<FourCc> {
    track.original_format.or(track.sample_entry_type)
}

fn video_track_kind(track: &DetailedTrackInfo) -> TrackKind {
    if track.summary.encrypted {
        TrackKind::EncryptedVideo
    } else {
        TrackKind::Video
    }
}

fn audio_track_kind(track: &DetailedTrackInfo) -> TrackKind {
    if track.summary.encrypted {
        TrackKind::EncryptedAudio
    } else {
        TrackKind::Audio
    }
}

fn track_layout(track: &DetailedTrackInfo) -> Result<TrackLayout, DivideError> {
    match authoritative_track_format(track) {
        Some(AVC1) => {
            let avc = track.summary.avc.as_ref().ok_or_else(|| {
                invalid_input(format!(
                    "track {} is missing the AVC decoder configuration needed for divide playlist signaling.",
                    track.summary.track_id
                ))
            })?;
            Ok(TrackLayout {
                role: DivideTrackRole::Video,
                kind: video_track_kind(track),
                codecs: format!(
                    "avc1.{:02x}{:02x}{:02x}",
                    avc.profile, avc.profile_compatibility, avc.level
                ),
                language: normalized_track_language(track),
                audio_channels: None,
                sample_rate: None,
                width: track.display_width.or(Some(avc.width)),
                height: track.display_height.or(Some(avc.height)),
            })
        }
        Some(HEV1 | HVC1 | DVHE | DVH1 | AV01 | VP08 | VP09) => Ok(TrackLayout {
            role: DivideTrackRole::Video,
            kind: video_track_kind(track),
            codecs: track_codec_label(track),
            language: normalized_track_language(track),
            audio_channels: None,
            sample_rate: None,
            width: track.display_width,
            height: track.display_height,
        }),
        Some(MP4A) => Ok(TrackLayout {
            role: DivideTrackRole::Audio,
            kind: audio_track_kind(track),
            codecs: track.summary.mp4a.as_ref().map_or_else(
                || track_codec_label(track),
                |mp4a| mp4a_codec_string(mp4a.object_type_indication, mp4a.audio_object_type),
            ),
            language: normalized_track_language(track),
            audio_channels: track
                .channel_count
                .or_else(|| track.summary.mp4a.as_ref().map(|mp4a| mp4a.channel_count))
                .filter(|value| *value != 0),
            sample_rate: track.sample_rate.filter(|value| *value != 0),
            width: None,
            height: None,
        }),
        Some(
            OPUS | AC_3 | EC_3 | AC_4 | ALAC | DTSC | DTSE | DTSH | DTSL | DTSM | DTSX | DTSY
            | FLAC | IAMF | MHA1 | MHA2 | MHM1 | MHM2 | IPCM | FPCM,
        ) => Ok(TrackLayout {
            role: DivideTrackRole::Audio,
            kind: audio_track_kind(track),
            codecs: track_codec_label(track),
            language: normalized_track_language(track),
            audio_channels: track.channel_count.filter(|value| *value != 0),
            sample_rate: track.sample_rate.filter(|value| *value != 0),
            width: None,
            height: None,
        }),
        _ => Err(invalid_input(format!(
            "track {} uses unsupported codec `{}`; {}",
            track.summary.track_id,
            track_codec_label(track),
            supported_scope_message()
        ))),
    }
}

fn write_init_segments<R>(
    reader: &mut R,
    tracks: &mut BTreeMap<u32, TrackOutput>,
) -> Result<(), DivideError>
where
    R: Read + Seek,
{
    loop {
        let start = reader.stream_position()?;
        let info = match BoxInfo::read(reader) {
            Ok(info) => info,
            Err(HeaderError::Io(error)) if clean_root_eof(reader, start, &error)? => break,
            Err(error) => return Err(error.into()),
        };

        match info.box_type() {
            FTYP | PSSH | SIDX => {
                let bytes = read_raw_box_bytes(reader, &info)?;
                for track in tracks.values_mut() {
                    track.init_writer.write_all(&bytes)?;
                }
            }
            MOOV => write_moov(reader, &info, tracks)?,
            MOOF | MDAT | MFRA => {
                info.seek_to_end(reader)?;
            }
            _ => {
                info.seek_to_end(reader)?;
            }
        }
    }

    Ok(())
}

fn write_moov<R>(
    reader: &mut R,
    info: &BoxInfo,
    tracks: &mut BTreeMap<u32, TrackOutput>,
) -> Result<(), DivideError>
where
    R: Read + Seek,
{
    let placeholder =
        BoxInfo::new(info.box_type(), info.header_size()).with_header_size(info.header_size());
    for track in tracks.values_mut() {
        track.init_writer.start_box(placeholder)?;
    }

    info.seek_to_payload(reader)?;
    let mut remaining_size = info.payload_size()?;
    while remaining_size >= info.header_size().min(8) {
        let child = BoxInfo::read(reader)?;
        remaining_size = remaining_size.saturating_sub(child.size());

        if child.box_type() == TRAK {
            let track_id = trak_track_id(reader, &child)?;
            if let Some(track) = tracks.get_mut(&track_id) {
                let bytes = read_raw_box_bytes(reader, &child)?;
                track.init_writer.write_all(&bytes)?;
            } else {
                child.seek_to_end(reader)?;
            }
        } else {
            let bytes = read_raw_box_bytes(reader, &child)?;
            for track in tracks.values_mut() {
                track.init_writer.write_all(&bytes)?;
            }
        }
    }

    for track in tracks.values_mut() {
        track.init_writer.end_box()?;
    }

    info.seek_to_end(reader)?;
    Ok(())
}

fn write_media_segments<R>(
    reader: &mut R,
    tracks: &mut BTreeMap<u32, TrackOutput>,
) -> Result<(), DivideError>
where
    R: Read + Seek,
{
    let mut pending_segment: Option<(u32, File)> = None;

    loop {
        let start = reader.stream_position()?;
        let info = match BoxInfo::read(reader) {
            Ok(info) => info,
            Err(HeaderError::Io(error)) if clean_root_eof(reader, start, &error)? => break,
            Err(error) => return Err(error.into()),
        };

        match info.box_type() {
            MOOF => {
                let track_id = moof_track_id(reader, &info)?;
                let track = tracks
                    .get_mut(&track_id)
                    .ok_or(DivideError::UnknownTrack(track_id))?;
                let segment_path = track
                    .output_dir
                    .join(segment_file_name(track.next_segment_index));
                track.next_segment_index += 1;
                let mut file = File::create(segment_path)?;
                copy_box_stream(reader, &mut file, &info)?;
                pending_segment = Some((track_id, file));
            }
            MDAT => {
                let Some((track_id, mut file)) = pending_segment.take() else {
                    return Err(DivideError::UnexpectedMdat);
                };
                let track = tracks
                    .get_mut(&track_id)
                    .ok_or(DivideError::UnknownTrack(track_id))?;
                let segment_index = track
                    .next_segment_index
                    .checked_sub(1)
                    .ok_or(DivideError::UnexpectedMdat)?;
                if let Some(duration) = track.segment_durations.get(segment_index).copied()
                    && duration > 0.0
                {
                    let bandwidth = ((info.size() as f64) * 8.0 / duration) as u64;
                    track.bandwidth = track.bandwidth.max(bandwidth);
                }
                copy_box_stream(reader, &mut file, &info)?;
            }
            MFRA => {
                info.seek_to_end(reader)?;
            }
            _ => {
                info.seek_to_end(reader)?;
            }
        }
    }

    Ok(())
}

fn write_output_manifests(
    output_dir: &Path,
    tracks: &BTreeMap<u32, TrackOutput>,
    output_options: DivideOutputOptions,
) -> Result<(), DivideError> {
    let dash_outcome = match output_options.manifest_selection {
        DivideManifestSelection::Hls => {
            write_hls_playlists(output_dir, tracks, &output_options)?;
            None
        }
        DivideManifestSelection::Dash => {
            Some(write_dash_manifest(output_dir, tracks, &output_options)?)
        }
        DivideManifestSelection::Both => {
            write_hls_playlists(output_dir, tracks, &output_options)?;
            Some(write_dash_manifest(output_dir, tracks, &output_options)?)
        }
    };
    if let (Some(path), Some(outcome)) = (
        output_options.dash_session_save_path.as_deref(),
        dash_outcome.as_ref(),
    ) {
        let state = build_dash_session_state(&output_options, outcome, tracks);
        save_dash_session_state(path, &state)?;
    }
    Ok(())
}

fn build_dash_session_state(
    output_options: &DivideOutputOptions,
    outcome: &DashManifestOutcome,
    tracks: &BTreeMap<u32, TrackOutput>,
) -> DashSessionState {
    DashSessionState {
        next_period_id: next_dash_period_id(output_options.dash_period_id.as_deref()),
        next_period_start_micros: Some(
            outcome
                .period_start_micros
                .saturating_add(outcome.total_duration_micros),
        ),
        manifest_selection: Some(output_options.manifest_selection),
        dash_manifest_mode: Some(output_options.dash_manifest_mode),
        dash_manifest_layout: Some(output_options.dash_manifest_layout),
        dash_manifest_profile: Some(output_options.dash_manifest_profile),
        dash_base_urls: output_options.dash_base_urls.clone(),
        dash_location: output_options.dash_location.clone(),
        dash_min_buffer_time_micros: output_options.dash_min_buffer_time_micros,
        dash_minimum_update_period_micros: output_options.dash_minimum_update_period_micros,
        dash_suggested_presentation_delay_micros: output_options
            .dash_suggested_presentation_delay_micros,
        dash_time_shift_buffer_depth_micros: output_options.dash_time_shift_buffer_depth_micros,
        dash_availability_start_time: output_options.dash_availability_start_time.clone(),
        dash_publish_time: output_options.dash_publish_time.clone(),
        dash_utc_timing_scheme: output_options.dash_utc_timing_scheme.clone(),
        dash_utc_timing_value: output_options.dash_utc_timing_value.clone(),
        next_segment_indices: tracks
            .iter()
            .map(|(track_id, track)| (*track_id, track.next_segment_index))
            .collect(),
    }
}

fn next_dash_period_id(current: Option<&str>) -> Option<String> {
    let current = current?;
    let suffix_start = current
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(index, _)| index)?;
    let (prefix, digits) = current.split_at(suffix_start);
    let width = digits.len();
    let value = digits.parse::<u64>().ok()?.saturating_add(1);
    Some(format!("{prefix}{value:0width$}"))
}

fn write_hls_playlists(
    output_dir: &Path,
    tracks: &BTreeMap<u32, TrackOutput>,
    output_options: &DivideOutputOptions,
) -> Result<(), DivideError> {
    let hls_master_playlist_name = effective_hls_master_playlist_name(output_options);
    let hls_media_playlist_name = effective_hls_media_playlist_name(output_options);
    let audio_tracks = tracks
        .values()
        .filter(|track| matches!(track.kind, TrackKind::Audio | TrackKind::EncryptedAudio))
        .collect::<Vec<_>>();
    let default_audio_track_id =
        select_default_audio_track_id(&audio_tracks, output_options.default_language.as_deref());
    let video = tracks
        .values()
        .find(|track| matches!(track.kind, TrackKind::Video | TrackKind::EncryptedVideo));
    let hls_program_date_time_base = output_options.hls_program_date_time.then(SystemTime::now);

    if let Some(video) = video {
        let mut master = File::create(output_dir.join(hls_master_playlist_name))?;
        writeln!(master, "#EXTM3U")?;
        let multiple_audio_tracks = audio_tracks.len() > 1;
        for audio in &audio_tracks {
            let media_playlist_uri = hls_uri(
                output_options.hls_base_url.as_deref(),
                &format!("{}/{}", audio.relative_dir, hls_media_playlist_name),
            );
            write!(
                master,
                "#EXT-X-MEDIA:TYPE=AUDIO,URI=\"{}\",GROUP-ID=\"audio\",NAME=\"{}\",AUTOSELECT=YES",
                media_playlist_uri,
                if multiple_audio_tracks {
                    format!("audio-{}", audio.track_id)
                } else {
                    "audio".to_string()
                }
            )?;
            if multiple_audio_tracks {
                write!(
                    master,
                    ",DEFAULT={}",
                    if Some(audio.track_id) == default_audio_track_id {
                        "YES"
                    } else {
                        "NO"
                    }
                )?;
            }
            if let Some(language) = audio.language.as_deref()
                && !language.eq_ignore_ascii_case("und")
            {
                write!(master, ",LANGUAGE=\"{}\"", language)?;
            }
            if let Some(channels) = audio.audio_channels {
                write!(master, ",CHANNELS=\"{channels}\"")?;
            }
            writeln!(master)?;
        }

        write!(
            master,
            "#EXT-X-STREAM-INF:BANDWIDTH={},CODECS=\"{}\"",
            video.bandwidth,
            master_playlist_codecs(video, &audio_tracks)
        )?;
        if let (Some(width), Some(height)) = (video.width, video.height) {
            write!(master, ",RESOLUTION={}x{}", width, height)?;
        }
        if !audio_tracks.is_empty() {
            write!(master, ",AUDIO=\"audio\"")?;
        }
        writeln!(master)?;
        writeln!(
            master,
            "{}",
            hls_uri(
                output_options.hls_base_url.as_deref(),
                &format!("{}/{}", video.relative_dir, hls_media_playlist_name),
            )
        )?;
    }

    let hls_playlist_type = output_options
        .hls_playlist_type
        .unwrap_or(HlsPlaylistType::Vod);
    for track in tracks.values() {
        let mut media = File::create(track.output_dir.join(hls_media_playlist_name))?;
        writeln!(media, "#EXTM3U")?;
        writeln!(media, "#EXT-X-VERSION:7")?;
        let max_duration = track
            .segment_durations
            .iter()
            .fold(0.0_f64, |max, value| max.max(*value));
        writeln!(
            media,
            "#EXT-X-TARGETDURATION:{}",
            max_duration.ceil() as u64
        )?;
        match hls_playlist_type {
            HlsPlaylistType::Vod => writeln!(media, "#EXT-X-PLAYLIST-TYPE:VOD")?,
            HlsPlaylistType::Event => writeln!(media, "#EXT-X-PLAYLIST-TYPE:EVENT")?,
            HlsPlaylistType::Live => {}
        }
        if track.first_segment_index != 0 {
            writeln!(media, "#EXT-X-MEDIA-SEQUENCE:{}", track.first_segment_index)?;
        }
        if let Some(start_time_offset_micros) = output_options.hls_start_time_offset_micros {
            writeln!(
                media,
                "#EXT-X-START:TIME-OFFSET={}",
                hls_time_offset_attr(start_time_offset_micros)
            )?;
        }
        writeln!(
            media,
            "#EXT-X-MAP:URI=\"{}\"",
            hls_track_media_uri(
                output_options.hls_base_url.as_deref(),
                &track.relative_dir,
                INIT_FILE_NAME,
            )
        )?;
        let mut next_program_date_time = hls_program_date_time_base;
        for (index, duration) in track.segment_durations.iter().enumerate() {
            if let Some(program_date_time) = next_program_date_time {
                writeln!(
                    media,
                    "#EXT-X-PROGRAM-DATE-TIME:{}",
                    format_hls_program_date_time(program_date_time)?
                )?;
                next_program_date_time =
                    Some(program_date_time + Duration::from_micros(seconds_to_micros(*duration)));
            }
            writeln!(media, "#EXTINF:{duration:.6},")?;
            writeln!(
                media,
                "{}",
                hls_track_media_uri(
                    output_options.hls_base_url.as_deref(),
                    &track.relative_dir,
                    &segment_file_name(track.first_segment_index.saturating_add(index))
                )
            )?;
        }
        if hls_playlist_type == HlsPlaylistType::Vod {
            writeln!(media, "#EXT-X-ENDLIST")?;
        }
    }
    Ok(())
}

fn write_dash_manifest(
    output_dir: &Path,
    tracks: &BTreeMap<u32, TrackOutput>,
    output_options: &DivideOutputOptions,
) -> Result<DashManifestOutcome, DivideError> {
    let dash_manifest_name = effective_dash_manifest_name(output_options);
    let audio_tracks = tracks
        .values()
        .filter(|track| matches!(track.kind, TrackKind::Audio | TrackKind::EncryptedAudio))
        .collect::<Vec<_>>();
    let preferred_audio_track_id = select_preferred_language_audio_track_id(
        &audio_tracks,
        output_options.default_language.as_deref(),
    );
    let total_duration = tracks
        .values()
        .map(|track| track.segment_durations.iter().sum::<f64>())
        .fold(0.0_f64, f64::max);
    let min_buffer_time = tracks
        .values()
        .flat_map(|track| track.segment_durations.iter().copied())
        .fold(0.0_f64, f64::max);
    let total_duration_micros = seconds_to_micros(total_duration);
    let min_buffer_time_micros = output_options
        .dash_min_buffer_time_micros
        .unwrap_or_else(|| default_dash_min_buffer_time_micros(min_buffer_time));
    let minimum_update_period_micros = output_options
        .dash_minimum_update_period_micros
        .unwrap_or(DEFAULT_DASH_MINIMUM_UPDATE_PERIOD_MICROS);
    let suggested_presentation_delay_micros = output_options
        .dash_suggested_presentation_delay_micros
        .unwrap_or(0);
    let period_start_micros = output_options.dash_period_start_micros.unwrap_or(0);
    let profile = dash_profile_urn(output_options.dash_manifest_profile);
    let auto_publish_time = format_dash_utc_timestamp(SystemTime::now())?;
    let availability_start_time = output_options
        .dash_availability_start_time
        .as_deref()
        .unwrap_or(auto_publish_time.as_str());
    let publish_time = output_options
        .dash_publish_time
        .as_deref()
        .unwrap_or(auto_publish_time.as_str());

    let mut manifest = File::create(output_dir.join(dash_manifest_name))?;
    writeln!(manifest, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
    match output_options.dash_manifest_mode {
        DashManifestMode::Static => {
            writeln!(
                manifest,
                "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" profiles=\"{}\" mediaPresentationDuration=\"{}\" minBufferTime=\"{}\">",
                profile,
                dash_duration_attr_from_micros(total_duration_micros),
                dash_duration_attr_from_micros(min_buffer_time_micros)
            )?;
        }
        DashManifestMode::Dynamic => {
            write!(
                manifest,
                "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"dynamic\" profiles=\"{}\" minBufferTime=\"{}\" minimumUpdatePeriod=\"{}\" availabilityStartTime=\"{}\" publishTime=\"{}\"",
                profile,
                dash_duration_attr_from_micros(min_buffer_time_micros),
                dash_duration_attr_from_micros(minimum_update_period_micros),
                dash_escape_attr(availability_start_time),
                dash_escape_attr(publish_time)
            )?;
            if suggested_presentation_delay_micros > 0 {
                write!(
                    manifest,
                    " suggestedPresentationDelay=\"{}\"",
                    dash_duration_attr_from_micros(suggested_presentation_delay_micros)
                )?;
            }
            if let Some(time_shift_buffer_depth_micros) =
                output_options.dash_time_shift_buffer_depth_micros
            {
                write!(
                    manifest,
                    " timeShiftBufferDepth=\"{}\"",
                    dash_duration_attr_from_micros(time_shift_buffer_depth_micros)
                )?;
            }
            writeln!(manifest, ">")?;
        }
    }
    if let Some(location) = output_options.dash_location.as_deref() {
        writeln!(
            manifest,
            "  <Location>{}</Location>",
            dash_escape_attr(location)
        )?;
    }
    for base_url in &output_options.dash_base_urls {
        writeln!(
            manifest,
            "  <BaseURL>{}</BaseURL>",
            dash_escape_attr(base_url)
        )?;
    }
    if let (Some(utc_timing_scheme), Some(utc_timing_value)) = (
        output_options.dash_utc_timing_scheme.as_deref(),
        output_options.dash_utc_timing_value.as_deref(),
    ) {
        writeln!(
            manifest,
            "  <UTCTiming schemeIdUri=\"{}\" value=\"{}\" />",
            dash_escape_attr(utc_timing_scheme),
            dash_escape_attr(utc_timing_value)
        )?;
    }
    match output_options.dash_manifest_mode {
        DashManifestMode::Static => {
            write!(manifest, "  <Period")?;
            if let Some(period_id) = output_options.dash_period_id.as_deref() {
                write!(manifest, " id=\"{}\"", dash_escape_attr(period_id))?;
            }
            if output_options.dash_period_start_micros.is_some() {
                write!(
                    manifest,
                    " start=\"{}\"",
                    dash_duration_attr_from_micros(period_start_micros)
                )?;
            }
            writeln!(
                manifest,
                " duration=\"{}\">",
                dash_duration_attr_from_micros(total_duration_micros)
            )?;
        }
        DashManifestMode::Dynamic => {
            write!(manifest, "  <Period")?;
            if let Some(period_id) = output_options.dash_period_id.as_deref() {
                write!(manifest, " id=\"{}\"", dash_escape_attr(period_id))?;
            }
            write!(
                manifest,
                " start=\"{}\">",
                dash_duration_attr_from_micros(period_start_micros)
            )?;
            writeln!(manifest)?;
        }
    }
    for (track_id, track) in tracks {
        match track.kind {
            TrackKind::Video | TrackKind::EncryptedVideo => {
                write_dash_representation(
                    &mut manifest,
                    *track_id,
                    track,
                    "video",
                    "video/mp4",
                    output_options.dash_manifest_layout,
                    preferred_audio_track_id,
                )?;
            }
            TrackKind::Audio | TrackKind::EncryptedAudio => {
                write_dash_representation(
                    &mut manifest,
                    *track_id,
                    track,
                    "audio",
                    "audio/mp4",
                    output_options.dash_manifest_layout,
                    preferred_audio_track_id,
                )?;
            }
        }
    }

    writeln!(manifest, "  </Period>")?;
    writeln!(manifest, "</MPD>")?;
    Ok(DashManifestOutcome {
        total_duration_micros,
        period_start_micros,
    })
}

fn write_dash_representation<W>(
    writer: &mut W,
    track_id: u32,
    track: &TrackOutput,
    content_type: &str,
    mime_type: &str,
    dash_manifest_layout: DashManifestLayout,
    preferred_audio_track_id: Option<u32>,
) -> Result<(), DivideError>
where
    W: Write,
{
    write!(
        writer,
        "    <AdaptationSet id=\"{}\" contentType=\"{}\" mimeType=\"{}\"",
        track_id, content_type, mime_type
    )?;
    if let Some(language) = track.language.as_deref()
        && !language.eq_ignore_ascii_case("und")
    {
        write!(writer, " lang=\"{}\"", dash_escape_attr(language))?;
    }
    writeln!(writer, ">")?;
    if content_type == "audio" && preferred_audio_track_id == Some(track_id) {
        writeln!(
            writer,
            "      <Role schemeIdUri=\"urn:mpeg:dash:role:2011\" value=\"main\" />"
        )?;
    }
    write!(
        writer,
        "      <Representation id=\"{}\" bandwidth=\"{}\" codecs=\"{}\"",
        track_id,
        track.bandwidth,
        dash_escape_attr(&track.codecs)
    )?;
    if let (Some(width), Some(height)) = (track.width, track.height) {
        write!(writer, " width=\"{}\" height=\"{}\"", width, height)?;
    }
    if let Some(sample_rate) = track.sample_rate {
        write!(writer, " audioSamplingRate=\"{}\"", sample_rate)?;
    }
    writeln!(writer, ">")?;
    match dash_manifest_layout {
        DashManifestLayout::Template => {
            writeln!(
                writer,
                "        <SegmentTemplate timescale=\"1000000\" initialization=\"{}/{}\" media=\"{}/$Number$.mp4\" startNumber=\"{}\">",
                track.relative_dir, INIT_FILE_NAME, track.relative_dir, track.first_segment_index
            )?;
            writeln!(writer, "          <SegmentTimeline>")?;
            for duration in &track.segment_durations {
                writeln!(
                    writer,
                    "            <S d=\"{}\" />",
                    ((*duration * 1_000_000.0).round() as u64)
                )?;
            }
            writeln!(writer, "          </SegmentTimeline>")?;
            writeln!(writer, "        </SegmentTemplate>")?;
        }
        DashManifestLayout::List => {
            writeln!(writer, "        <SegmentList>")?;
            writeln!(
                writer,
                "          <Initialization sourceURL=\"{}/{}\" />",
                track.relative_dir, INIT_FILE_NAME
            )?;
            for index in 0..track.segment_durations.len() {
                writeln!(
                    writer,
                    "          <SegmentURL media=\"{}/{}\" />",
                    track.relative_dir,
                    segment_file_name(track.first_segment_index.saturating_add(index))
                )?;
            }
            writeln!(writer, "        </SegmentList>")?;
        }
    }
    writeln!(writer, "      </Representation>")?;
    writeln!(writer, "    </AdaptationSet>")?;
    Ok(())
}

fn track_relative_dir(kind: TrackKind, track_id: u32, multiple_audio_tracks: bool) -> String {
    match kind {
        TrackKind::Video => VIDEO_DIR.to_string(),
        TrackKind::Audio if multiple_audio_tracks => format!("{AUDIO_DIR}_{track_id}"),
        TrackKind::Audio => AUDIO_DIR.to_string(),
        TrackKind::EncryptedVideo => VIDEO_ENC_DIR.to_string(),
        TrackKind::EncryptedAudio if multiple_audio_tracks => {
            format!("{AUDIO_ENC_DIR}_{track_id}")
        }
        TrackKind::EncryptedAudio => AUDIO_ENC_DIR.to_string(),
    }
}

fn normalized_track_language(track: &DetailedTrackInfo) -> Option<String> {
    track
        .language
        .as_deref()
        .filter(|language| !language.is_empty())
        .filter(|language| {
            language
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        })
        .map(ToOwned::to_owned)
}

fn segment_file_name(index: usize) -> String {
    format!("{index}.mp4")
}

fn master_playlist_codecs(video: &TrackOutput, audio_tracks: &[&TrackOutput]) -> String {
    if audio_tracks.is_empty() {
        return video.codecs.clone();
    }

    let mut codecs = Vec::with_capacity(1 + audio_tracks.len());
    codecs.push(video.codecs.clone());
    for track in audio_tracks {
        if !codecs.iter().any(|codec| codec == &track.codecs) {
            codecs.push(track.codecs.clone());
        }
    }
    codecs.join(",")
}

fn seconds_to_micros(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1_000_000.0).round() as u64
}

fn hls_uri(base_url: Option<&str>, relative_uri: &str) -> String {
    match base_url {
        Some(base_url) => format!("{base_url}{relative_uri}"),
        None => relative_uri.to_string(),
    }
}

fn hls_track_media_uri(base_url: Option<&str>, relative_dir: &str, local_name: &str) -> String {
    match base_url {
        Some(base_url) => format!("{base_url}{relative_dir}/{local_name}"),
        None => local_name.to_string(),
    }
}

fn hls_time_offset_attr(micros: i64) -> String {
    format!("{:.6}", micros as f64 / 1_000_000.0)
}

fn dash_profile_urn(profile: DashManifestProfile) -> &'static str {
    match profile {
        DashManifestProfile::Main => "urn:mpeg:dash:profile:isoff-main:2011",
        DashManifestProfile::Live => "urn:mpeg:dash:profile:isoff-live:2011",
    }
}

fn dash_duration_attr_from_micros(micros: u64) -> String {
    dash_duration_attr(micros as f64 / 1_000_000.0)
}

fn dash_duration_attr(seconds: f64) -> String {
    let mut value = format!("{seconds:.6}");
    while value.contains('.') && value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    format!("PT{value}S")
}

fn dash_escape_attr(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn mp4a_codec_string(object_type_indication: u8, audio_object_type: u8) -> String {
    if object_type_indication == 0 {
        "mp4a".to_string()
    } else if audio_object_type == 0 {
        format!("mp4a.{object_type_indication:x}")
    } else {
        format!("mp4a.{object_type_indication:x}.{audio_object_type}")
    }
}

fn write_validation_report<W>(
    writer: &mut W,
    report: &DivideValidationReport,
) -> Result<(), DivideError>
where
    W: Write,
{
    writeln!(writer, "supported fragmented divide layout")?;
    for track in &report.tracks {
        writeln!(
            writer,
            "track {}: role={} codec={} segments={}",
            track.track_id,
            validation_role_label(track.role),
            validation_codec_label(track),
            track.segment_count
        )?;
    }
    Ok(())
}

fn collect_divide_plan_warnings(
    plans: &[ValidatedTrackPlan],
    output_options: &DivideOutputOptions,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if !plans
        .iter()
        .any(|plan| plan.validation.role == DivideTrackRole::Video)
    {
        warnings.push(
            "divide output is audio-only; no fragmented video track was selected".to_string(),
        );
    }

    let multiple_audio_tracks = plans
        .iter()
        .filter(|plan| plan.validation.role == DivideTrackRole::Audio)
        .count()
        > 1;

    for plan in plans {
        let track_id = plan.validation.track_id;
        if plan.segment_durations.is_empty() {
            warnings.push(format!("track {track_id} has no fragmented media segments"));
        }

        let zero_duration_segments = plan
            .segment_durations
            .iter()
            .filter(|duration| **duration <= 0.0)
            .count();
        if zero_duration_segments > 0 {
            warnings.push(format!(
                "track {track_id} has {zero_duration_segments} zero-duration fragmented segment(s)"
            ));
        }

        let duration_changes = count_segment_duration_changes(&plan.segment_durations);
        if duration_changes > 0 {
            warnings.push(format!(
                "track {track_id} changes segment duration {duration_changes} time(s)"
            ));
            if let Some((min_duration, max_duration)) =
                duration_span(plan.segment_durations.iter().copied())
            {
                warnings.push(format!(
                    "track {track_id} fragmented segment duration spans {} to {}",
                    format_warning_seconds(min_duration),
                    format_warning_seconds(max_duration)
                ));
            }
        }

        if let Some(next_segment_index) = output_options
            .dash_session_next_segment_indices
            .get(&track_id)
            .copied()
            .filter(|next_segment_index| *next_segment_index > 0)
        {
            warnings.push(format!(
                "track {track_id} resumes fragmented segment numbering at {next_segment_index}"
            ));
        }

        if multiple_audio_tracks
            && plan.validation.role == DivideTrackRole::Audio
            && plan
                .layout
                .language
                .as_deref()
                .is_none_or(|language| language.eq_ignore_ascii_case("und"))
        {
            warnings.push(format!(
                "audio track {track_id} has no normalized language code for alternate-playlist signaling"
            ));
        }
    }

    warnings
}

fn collect_fragmented_warning_lines<R>(
    reader: &mut R,
    plans: &[ValidatedTrackPlan],
    output_options: &DivideOutputOptions,
) -> Result<Vec<String>, DivideError>
where
    R: Read + Seek,
{
    let mut warnings = collect_divide_plan_warnings(plans, output_options);
    reader.seek(SeekFrom::Start(0))?;
    let summary = probe_detailed(reader)?;
    reader.seek(SeekFrom::Start(0))?;
    let track_warning_diagnostics = fragmented_track_warning_diagnostics(reader)?;
    warnings.extend(collect_fragmented_probe_warnings(
        &summary,
        &track_warning_diagnostics,
    ));
    dedupe_warning_lines(&mut warnings);
    Ok(warnings)
}

fn collect_fragmented_probe_warnings(
    summary: &DetailedProbeInfo,
    track_warning_diagnostics: &BTreeMap<u32, FragmentedTrackWarningDiagnostics>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for track in &summary.tracks {
        let track_id = track.summary.track_id;
        let mut average_duration_changes = 0usize;
        let mut previous_average = None::<(u32, u32)>;
        let mut average_duration_min = None::<f64>;
        let mut average_duration_max = None::<f64>;
        let mut empty_segments = 0usize;
        let mut zero_duration_segment_sample_count = 0u64;
        let mut decode_gap_count = 0usize;
        let mut largest_decode_gap = 0_u64;
        let mut decode_regression_count = 0usize;
        let mut largest_decode_regression = 0_u64;
        let mut previous_end_decode_time = None::<u64>;

        for segment in summary
            .segments
            .iter()
            .filter(|segment| segment.track_id == track_id)
        {
            if segment.sample_count == 0 {
                empty_segments += 1;
            } else {
                if segment.duration == 0 {
                    zero_duration_segment_sample_count += u64::from(segment.sample_count);
                }
                let current_average = (segment.duration, segment.sample_count);
                if let Some((previous_duration, previous_sample_count)) = previous_average
                    && u128::from(previous_duration) * u128::from(segment.sample_count)
                        != u128::from(segment.duration) * u128::from(previous_sample_count)
                {
                    average_duration_changes += 1;
                }
                let average_duration_seconds = if track.summary.timescale == 0 {
                    0.0
                } else {
                    f64::from(segment.duration)
                        / f64::from(segment.sample_count)
                        / f64::from(track.summary.timescale)
                };
                average_duration_min = Some(
                    average_duration_min.map_or(average_duration_seconds, |value| {
                        value.min(average_duration_seconds)
                    }),
                );
                average_duration_max = Some(
                    average_duration_max.map_or(average_duration_seconds, |value| {
                        value.max(average_duration_seconds)
                    }),
                );
                previous_average = Some(current_average);
            }

            if let Some(previous_end_decode_time) = previous_end_decode_time {
                if segment.base_media_decode_time > previous_end_decode_time {
                    decode_gap_count += 1;
                    largest_decode_gap = largest_decode_gap.max(
                        segment
                            .base_media_decode_time
                            .saturating_sub(previous_end_decode_time),
                    );
                } else if segment.base_media_decode_time < previous_end_decode_time {
                    decode_regression_count += 1;
                    largest_decode_regression = largest_decode_regression.max(
                        previous_end_decode_time.saturating_sub(segment.base_media_decode_time),
                    );
                }
            }
            previous_end_decode_time = Some(
                segment
                    .base_media_decode_time
                    .saturating_add(u64::from(segment.duration)),
            );
        }

        if zero_duration_segment_sample_count != 0 {
            warnings.push(format!(
                "track {track_id} carries {zero_duration_segment_sample_count} sample(s) inside zero-duration fragmented segment(s)"
            ));
        }
        if let Some(track_diagnostics) = track_warning_diagnostics.get(&track_id) {
            if track_diagnostics.zero_duration_sample_count != 0 {
                warnings.push(format!(
                    "track {track_id} carries {} zero-duration fragmented sample(s)",
                    track_diagnostics.zero_duration_sample_count
                ));
            }
            if track_diagnostics.sample_duration_change_count != 0 {
                warnings.push(format!(
                    "track {track_id} changes authored fragmented sample duration {} time(s)",
                    track_diagnostics.sample_duration_change_count
                ));
                if let (Some(min_duration), Some(max_duration)) = (
                    track_diagnostics.min_non_zero_sample_duration,
                    track_diagnostics.max_non_zero_sample_duration,
                ) {
                    warnings.push(format!(
                        "track {track_id} authored fragmented sample duration spans {} to {}",
                        format_warning_track_delta(
                            u64::from(min_duration),
                            track.summary.timescale
                        ),
                        format_warning_track_delta(
                            u64::from(max_duration),
                            track.summary.timescale
                        )
                    ));
                }
            }
        }
        if empty_segments != 0 {
            warnings.push(format!(
                "track {track_id} has {empty_segments} fragmented segment(s) with no samples"
            ));
        }
        if average_duration_changes != 0 {
            warnings.push(format!(
                "track {track_id} changes average fragmented sample duration {average_duration_changes} time(s)"
            ));
            if let (Some(min_duration), Some(max_duration)) =
                (average_duration_min, average_duration_max)
            {
                warnings.push(format!(
                    "track {track_id} fragmented average sample duration spans {} to {}",
                    format_warning_seconds(min_duration),
                    format_warning_seconds(max_duration)
                ));
            }
        }
        if decode_gap_count != 0 {
            warnings.push(format!(
                "track {track_id} has {decode_gap_count} fragmented decode-timeline gap(s)"
            ));
            warnings.push(format!(
                "track {track_id} has a largest fragmented decode-timeline gap of {}",
                format_warning_track_delta(largest_decode_gap, track.summary.timescale)
            ));
        }
        if decode_regression_count != 0 {
            warnings.push(format!(
                "track {track_id} has {decode_regression_count} fragmented decode-timeline regression(s)"
            ));
            warnings.push(format!(
                "track {track_id} has a largest fragmented decode-timeline regression of {}",
                format_warning_track_delta(largest_decode_regression, track.summary.timescale)
            ));
        }
    }
    warnings
}

fn dedupe_warning_lines(warnings: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    warnings.retain(|warning| seen.insert(warning.clone()));
}

fn duration_span<I>(durations: I) -> Option<(f64, f64)>
where
    I: IntoIterator<Item = f64>,
{
    let mut values = durations.into_iter();
    let first = values.next()?;
    let mut min_value = first;
    let mut max_value = first;
    for value in values {
        min_value = min_value.min(value);
        max_value = max_value.max(value);
    }
    (min_value < max_value).then_some((min_value, max_value))
}

fn format_warning_seconds(seconds: f64) -> String {
    format!("{seconds:.6}s")
}

fn format_warning_track_delta(ticks: u64, timescale: u32) -> String {
    if timescale == 0 {
        return format!("{ticks} tick(s)");
    }

    format!(
        "{:.6}s ({} tick(s))",
        ticks as f64 / f64::from(timescale),
        ticks
    )
}

#[cfg(feature = "mux")]
pub(crate) fn collect_fragmented_file_warnings<R>(
    reader: &mut R,
) -> Result<Vec<String>, DivideError>
where
    R: Read + Seek,
{
    let plans = validate_divide_track_plans(reader)?;
    collect_fragmented_warning_lines(reader, &plans, &DivideOutputOptions::default())
}

fn count_segment_duration_changes(segment_durations: &[f64]) -> usize {
    let mut previous = None::<u64>;
    let mut changes = 0usize;
    for duration in segment_durations {
        let current = seconds_to_micros((*duration).max(0.0));
        if let Some(previous) = previous
            && previous != current
        {
            changes += 1;
        }
        previous = Some(current);
    }
    changes
}

fn validation_role_label(role: DivideTrackRole) -> &'static str {
    match role {
        DivideTrackRole::Video => "video",
        DivideTrackRole::Audio => "audio",
    }
}

fn validation_codec_label(track: &DivideValidationTrack) -> String {
    track
        .original_format
        .or(track.sample_entry_type)
        .map(|value| value.to_string())
        .unwrap_or_else(|| match track.codec_family {
            TrackCodecFamily::Unknown => "unknown".to_string(),
            TrackCodecFamily::Avc => "avc".to_string(),
            TrackCodecFamily::Hevc => "hevc".to_string(),
            TrackCodecFamily::Av1 => "av1".to_string(),
            TrackCodecFamily::Vp8 => "vp8".to_string(),
            TrackCodecFamily::Vp9 => "vp9".to_string(),
            TrackCodecFamily::Mp4Audio => "mp4a".to_string(),
            TrackCodecFamily::Opus => "opus".to_string(),
            TrackCodecFamily::Ac3 => "ac-3".to_string(),
            TrackCodecFamily::Pcm => "pcm".to_string(),
            TrackCodecFamily::XmlSubtitle => "stpp".to_string(),
            TrackCodecFamily::TextSubtitle => "sbtt".to_string(),
            TrackCodecFamily::WebVtt => "wvtt".to_string(),
        })
}

fn track_codec_label(track: &DetailedTrackInfo) -> String {
    track
        .original_format
        .or(track.sample_entry_type)
        .map(|value| value.to_string())
        .unwrap_or_else(|| match track.codec_family {
            TrackCodecFamily::Unknown => "unknown".to_string(),
            TrackCodecFamily::Avc => "avc".to_string(),
            TrackCodecFamily::Hevc => "hevc".to_string(),
            TrackCodecFamily::Av1 => "av1".to_string(),
            TrackCodecFamily::Vp8 => "vp8".to_string(),
            TrackCodecFamily::Vp9 => "vp9".to_string(),
            TrackCodecFamily::Mp4Audio => "mp4a".to_string(),
            TrackCodecFamily::Opus => "opus".to_string(),
            TrackCodecFamily::Ac3 => "ac-3".to_string(),
            TrackCodecFamily::Pcm => "pcm".to_string(),
            TrackCodecFamily::XmlSubtitle => "stpp".to_string(),
            TrackCodecFamily::TextSubtitle => "sbtt".to_string(),
            TrackCodecFamily::WebVtt => "wvtt".to_string(),
        })
}

fn supported_scope_message() -> &'static str {
    "divide currently supports fragmented inputs with at most one video track from AVC, HEVC, Dolby Vision on HEVC, AV1, VP8, or VP9 and one or more audio tracks from MP4A-based audio, Opus, AC-3, E-AC-3, AC-4, ALAC, DTS-family entries, FLAC, IAMF, MPEG-H, or PCM; subtitle and text tracks remain unsupported"
}

fn default_dash_min_buffer_time_micros(max_segment_duration_seconds: f64) -> u64 {
    seconds_to_micros(max_segment_duration_seconds).max(DEFAULT_DASH_MIN_BUFFER_TIME_MICROS)
}

fn format_hls_program_date_time(time: SystemTime) -> Result<String, DivideError> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid_input("system clock is earlier than the Unix epoch".to_string()))?;
    let total_seconds = duration.as_secs();
    let milliseconds = duration.subsec_millis();
    let seconds_of_day = total_seconds % 86_400;
    let days_since_epoch = total_seconds / 86_400;
    let (year, month, day) = civil_from_days(days_since_epoch);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milliseconds:03}Z"
    ))
}

fn format_dash_utc_timestamp(time: SystemTime) -> Result<String, DivideError> {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid_input("system clock is earlier than the Unix epoch".to_string()))?;
    let total_seconds = duration.as_secs();
    let seconds_of_day = total_seconds % 86_400;
    let days_since_epoch = total_seconds / 86_400;
    let (year, month, day) = civil_from_days(days_since_epoch);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn civil_from_days(days_since_epoch: u64) -> (i32, u32, u32) {
    let z = i128::from(days_since_epoch) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (
        i32::try_from(year).expect("civil year fits in i32"),
        u32::try_from(month).expect("civil month fits in u32"),
        u32::try_from(day).expect("civil day fits in u32"),
    )
}

fn invalid_input(message: String) -> DivideError {
    DivideError::Io(io::Error::new(io::ErrorKind::InvalidInput, message))
}

fn parse_divide_language_tag(value: &str) -> Result<String, DivideError> {
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        return Err(invalid_input(format!(
            "unsupported divide default language tag: {value}"
        )));
    }
    Ok(value.to_ascii_lowercase())
}

fn effective_hls_master_playlist_name(output_options: &DivideOutputOptions) -> &str {
    output_options
        .hls_master_playlist_name
        .as_deref()
        .unwrap_or(PLAYLIST_FILE_NAME)
}

fn effective_hls_media_playlist_name(output_options: &DivideOutputOptions) -> &str {
    output_options
        .hls_media_playlist_name
        .as_deref()
        .unwrap_or(PLAYLIST_FILE_NAME)
}

fn effective_dash_manifest_name(output_options: &DivideOutputOptions) -> &str {
    output_options
        .dash_manifest_name
        .as_deref()
        .unwrap_or(MANIFEST_FILE_NAME)
}

fn select_default_audio_track_id(
    audio_tracks: &[&TrackOutput],
    preferred_language: Option<&str>,
) -> Option<u32> {
    select_preferred_language_audio_track_id(audio_tracks, preferred_language)
        .or_else(|| audio_tracks.first().map(|track| track.track_id))
}

fn select_preferred_language_audio_track_id(
    audio_tracks: &[&TrackOutput],
    preferred_language: Option<&str>,
) -> Option<u32> {
    let preferred_language = preferred_language?;
    audio_tracks
        .iter()
        .find(|track| {
            track.language.as_deref().is_some_and(|language| {
                !language.eq_ignore_ascii_case("und")
                    && language.eq_ignore_ascii_case(preferred_language)
            })
        })
        .map(|track| track.track_id)
}

fn trak_track_id<R>(reader: &mut R, trak: &BoxInfo) -> Result<u32, DivideError>
where
    R: Read + Seek,
{
    let boxes = extract_boxes_with_payload(reader, Some(trak), &[BoxPath::from([TKHD])])?;
    let track = boxes
        .first()
        .and_then(|entry| entry.payload.as_ref().as_any().downcast_ref::<Tkhd>())
        .ok_or(DivideError::MissingTrackId)?;
    Ok(track.track_id)
}

fn moof_track_id<R>(reader: &mut R, moof: &BoxInfo) -> Result<u32, DivideError>
where
    R: Read + Seek,
{
    let boxes = extract_boxes_with_payload(
        reader,
        Some(moof),
        &[BoxPath::from([FourCc::from_bytes(*b"traf"), TFHD])],
    )?;
    let tfhd = boxes
        .first()
        .and_then(|entry| entry.payload.as_ref().as_any().downcast_ref::<Tfhd>())
        .ok_or(DivideError::MissingTrackId)?;
    Ok(tfhd.track_id)
}

fn read_raw_box_bytes<R>(reader: &mut R, info: &BoxInfo) -> Result<Vec<u8>, DivideError>
where
    R: Read + Seek,
{
    let len = usize::try_from(info.size()).map_err(|_| DivideError::NumericOverflow)?;
    let mut bytes = Vec::with_capacity(len);
    bytes.extend_from_slice(&info.encode());
    info.seek_to_payload(reader)?;
    let mut limited = reader.take(info.payload_size()?);
    limited.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn copy_box_stream<R, W>(reader: &mut R, writer: &mut W, info: &BoxInfo) -> Result<(), DivideError>
where
    R: Read + Seek,
    W: Write,
{
    writer.write_all(&info.encode())?;
    info.seek_to_payload(reader)?;
    let mut limited = reader.take(info.payload_size()?);
    io::copy(&mut limited, writer)?;
    Ok(())
}

fn clean_root_eof<R>(reader: &mut R, start: u64, error: &io::Error) -> Result<bool, io::Error>
where
    R: Seek,
{
    if error.kind() != io::ErrorKind::UnexpectedEof {
        return Ok(false);
    }

    let end = reader.seek(SeekFrom::End(0))?;
    Ok(start == end)
}

/// Errors raised while parsing divide arguments or splitting a fragmented file.
#[derive(Debug)]
pub enum DivideError {
    Io(io::Error),
    Header(HeaderError),
    Extract(ExtractError),
    Probe(ProbeError),
    Writer(WriterError),
    MissingTrackId,
    UnknownTrack(u32),
    UnexpectedMdat,
    NoSupportedTracks,
    NumericOverflow,
    UsageRequested,
}

impl DivideError {
    /// Stable coarse category label for user-facing divide diagnostics.
    pub fn category(&self) -> &'static str {
        match self {
            Self::Io(error) if error.kind() == io::ErrorKind::InvalidInput => "input",
            Self::Io(_) => "io",
            Self::Header(_) | Self::Extract(_) | Self::Probe(_) => "input",
            Self::Writer(_) => "writer",
            Self::MissingTrackId
            | Self::UnknownTrack(_)
            | Self::UnexpectedMdat
            | Self::NoSupportedTracks => "layout",
            Self::NumericOverflow => "internal",
            Self::UsageRequested => "request",
        }
    }

    /// Stable coarse stage label for user-facing divide diagnostics.
    pub fn stage(&self) -> &'static str {
        match self {
            Self::Io(error) if error.kind() == io::ErrorKind::InvalidInput => "request",
            Self::Io(_) => "io",
            Self::Header(_) | Self::Extract(_) | Self::Probe(_) => "inspect",
            Self::Writer(_) => "write",
            Self::MissingTrackId | Self::UnknownTrack(_) | Self::UnexpectedMdat => "segment",
            Self::NoSupportedTracks => "request",
            Self::NumericOverflow => "plan",
            Self::UsageRequested => "request",
        }
    }

    fn diagnostic_context(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::UsageRequested => None,
            _ => Some((self.stage(), self.category())),
        }
    }
}

impl fmt::Display for DivideError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Extract(error) => error.fmt(f),
            Self::Probe(error) => error.fmt(f),
            Self::Writer(error) => error.fmt(f),
            Self::MissingTrackId => f.write_str("track id not found"),
            Self::UnknownTrack(track_id) => write!(f, "unknown track id: {track_id}"),
            Self::UnexpectedMdat => f.write_str("mdat appeared without a preceding moof"),
            Self::NoSupportedTracks => write!(
                f,
                "no supported fragmented tracks found; {}",
                supported_scope_message()
            ),
            Self::NumericOverflow => f.write_str("numeric value does not fit in memory"),
            Self::UsageRequested => f.write_str("usage requested"),
        }
    }
}

impl Error for DivideError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Extract(error) => Some(error),
            Self::Probe(error) => Some(error),
            Self::Writer(error) => Some(error),
            Self::MissingTrackId
            | Self::UnknownTrack(..)
            | Self::UnexpectedMdat
            | Self::NoSupportedTracks
            | Self::NumericOverflow
            | Self::UsageRequested => None,
        }
    }
}

impl From<io::Error> for DivideError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for DivideError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<ExtractError> for DivideError {
    fn from(value: ExtractError) -> Self {
        Self::Extract(value)
    }
}

impl From<ProbeError> for DivideError {
    fn from(value: ProbeError) -> Self {
        Self::Probe(value)
    }
}

impl From<WriterError> for DivideError {
    fn from(value: WriterError) -> Self {
        Self::Writer(value)
    }
}
