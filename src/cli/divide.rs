//! Fragmented-file split command support.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::FourCc;
use crate::boxes::iso14496_12::{Tfhd, Tkhd};
use crate::extract::{ExtractError, extract_boxes_with_payload};
use crate::header::{BoxInfo, HeaderError};
use crate::probe::{
    DetailedProbeInfo, DetailedTrackInfo, ProbeError, TrackCodecFamily, probe_detailed,
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

const VIDEO_DIR: &str = "video";
const AUDIO_DIR: &str = "audio";
const VIDEO_ENC_DIR: &str = "video_enc";
const AUDIO_ENC_DIR: &str = "audio_enc";
const INIT_FILE_NAME: &str = "init.mp4";
const PLAYLIST_FILE_NAME: &str = "playlist.m3u8";

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
    match run_inner(args, stdout) {
        Ok(()) => 0,
        Err(DivideError::UsageRequested) => {
            let _ = write_usage(stderr);
            1
        }
        Err(error) => {
            let _ = writeln!(stderr, "Error: {error}");
            1
        }
    }
}

/// Writes the divide subcommand usage text.
pub fn write_usage<W>(writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(writer, "USAGE: mp4forge divide INPUT.mp4 OUTPUT_DIR")?;
    writeln!(writer, "       mp4forge divide -validate INPUT.mp4")?;
    writeln!(writer)?;
    writeln!(writer, "OPTIONS:")?;
    writeln!(
        writer,
        "  -validate    Validate the fragmented divide layout without writing output files"
    )?;
    writeln!(writer)?;
    writeln!(
        writer,
        "Currently supports fragmented inputs with up to one AVC video track and one MP4A audio track,"
    )?;
    writeln!(
        writer,
        "including encrypted wrappers that preserve those original sample-entry formats."
    )
}

#[derive(Debug)]
struct ParsedDivideArgs<'a> {
    validate_only: bool,
    input_path: &'a Path,
    output_dir: Option<&'a Path>,
}

fn run_inner<W>(args: &[String], stdout: &mut W) -> Result<(), DivideError>
where
    W: Write,
{
    let parsed = parse_args(args)?;
    let mut input = File::open(parsed.input_path)?;
    if parsed.validate_only {
        let report = validate_divide_reader(&mut input)?;
        write_validation_report(stdout, &report)?;
        return Ok(());
    }

    divide_reader(
        &mut input,
        parsed.output_dir.ok_or(DivideError::UsageRequested)?,
    )
}

fn parse_args(args: &[String]) -> Result<ParsedDivideArgs<'_>, DivideError> {
    let mut validate_only = false;
    let mut positional = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-validate" | "--validate" => {
                validate_only = true;
                index += 1;
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
            input_path,
            output_dir: None,
        }),
        (false, [input_path, output_dir]) => Ok(ParsedDivideArgs {
            validate_only,
            input_path,
            output_dir: Some(output_dir),
        }),
        _ => Err(DivideError::UsageRequested),
    }
}

/// Splits a fragmented MP4 reader into per-track outputs under `output_dir`.
///
/// The current `divide` surface supports fragmented inputs with at most one AVC video track and
/// one MP4A audio track, including encrypted `encv` and `enca` wrappers when the original format
/// is still `avc1` or `mp4a`.
pub fn divide_reader<R>(reader: &mut R, output_dir: &Path) -> Result<(), DivideError>
where
    R: Read + Seek,
{
    let plans = validate_divide_track_plans(reader)?;
    let mut tracks = build_track_outputs(&plans, output_dir)?;

    reader.seek(SeekFrom::Start(0))?;
    write_init_segments(reader, &mut tracks)?;
    reader.seek(SeekFrom::Start(0))?;
    write_media_segments(reader, &mut tracks)?;
    write_playlists(output_dir, &tracks)?;
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
    audio_channels: Option<u16>,
    width: Option<u16>,
    height: Option<u16>,
}

struct TrackOutput {
    kind: TrackKind,
    codecs: String,
    audio_channels: Option<u16>,
    width: Option<u16>,
    height: Option<u16>,
    segment_durations: Vec<f64>,
    bandwidth: u64,
    output_dir: PathBuf,
    init_writer: Writer<File>,
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
    Ok(DivideValidationReport {
        tracks: plans.into_iter().map(|plan| plan.validation).collect(),
    })
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
) -> Result<BTreeMap<u32, TrackOutput>, DivideError> {
    let mut tracks = BTreeMap::new();

    for plan in plans {
        let track_dir = output_dir.join(relative_dir(plan.layout.kind));
        fs::create_dir_all(&track_dir)?;
        let init_writer = Writer::new(File::create(track_dir.join(INIT_FILE_NAME))?);

        tracks.insert(
            plan.validation.track_id,
            TrackOutput {
                kind: plan.layout.kind,
                codecs: plan.layout.codecs.clone(),
                audio_channels: plan.layout.audio_channels,
                width: plan.layout.width,
                height: plan.layout.height,
                segment_durations: plan.segment_durations.clone(),
                bandwidth: 0,
                output_dir: track_dir,
                init_writer,
                next_segment_index: 0,
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
    let mut selected_audio_track_id = None;

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
            DivideTrackRole::Audio => {
                if let Some(existing_track_id) =
                    selected_audio_track_id.replace(track.summary.track_id)
                {
                    return Err(invalid_input(format!(
                        "{}; found multiple fragmented audio tracks ({existing_track_id} and {}).",
                        supported_scope_message(),
                        track.summary.track_id
                    )));
                }
            }
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

fn track_layout(track: &DetailedTrackInfo) -> Result<TrackLayout, DivideError> {
    match track.codec_family {
        TrackCodecFamily::Avc => {
            let avc = track.summary.avc.as_ref().ok_or_else(|| {
                invalid_input(format!(
                    "track {} is missing the AVC decoder configuration needed for divide playlist signaling.",
                    track.summary.track_id
                ))
            })?;
            Ok(TrackLayout {
                role: DivideTrackRole::Video,
                kind: if track.summary.encrypted {
                    TrackKind::EncryptedVideo
                } else {
                    TrackKind::Video
                },
                codecs: format!(
                    "avc1.{:02x}{:02x}{:02x}",
                    avc.profile, avc.profile_compatibility, avc.level
                ),
                audio_channels: None,
                width: track.display_width.or(Some(avc.width)),
                height: track.display_height.or(Some(avc.height)),
            })
        }
        TrackCodecFamily::Mp4Audio => {
            let mp4a = track.summary.mp4a.as_ref().ok_or_else(|| {
                invalid_input(format!(
                    "track {} is missing the MP4A decoder configuration needed for divide playlist signaling.",
                    track.summary.track_id
                ))
            })?;
            Ok(TrackLayout {
                role: DivideTrackRole::Audio,
                kind: if track.summary.encrypted {
                    TrackKind::EncryptedAudio
                } else {
                    TrackKind::Audio
                },
                codecs: mp4a_codec_string(mp4a.object_type_indication, mp4a.audio_object_type),
                audio_channels: track
                    .channel_count
                    .or(Some(mp4a.channel_count))
                    .filter(|value| *value != 0),
                width: None,
                height: None,
            })
        }
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

fn write_playlists(
    output_dir: &Path,
    tracks: &BTreeMap<u32, TrackOutput>,
) -> Result<(), DivideError> {
    let audio = tracks
        .values()
        .find(|track| matches!(track.kind, TrackKind::Audio | TrackKind::EncryptedAudio));
    let video = tracks
        .values()
        .find(|track| matches!(track.kind, TrackKind::Video | TrackKind::EncryptedVideo));

    if let Some(video) = video {
        let mut master = File::create(output_dir.join(PLAYLIST_FILE_NAME))?;
        writeln!(master, "#EXTM3U")?;
        if let Some(audio) = audio {
            write!(
                master,
                "#EXT-X-MEDIA:TYPE=AUDIO,URI=\"{}/{}\",GROUP-ID=\"audio\",NAME=\"audio\",AUTOSELECT=YES",
                relative_dir(audio.kind),
                PLAYLIST_FILE_NAME
            )?;
            if let Some(channels) = audio.audio_channels {
                write!(master, ",CHANNELS=\"{channels}\"")?;
            }
            writeln!(master)?;
        }

        write!(
            master,
            "#EXT-X-STREAM-INF:BANDWIDTH={},CODECS=\"{}\"",
            video.bandwidth,
            master_playlist_codecs(video, audio)
        )?;
        if let (Some(width), Some(height)) = (video.width, video.height) {
            write!(master, ",RESOLUTION={}x{}", width, height)?;
        }
        if audio.is_some() {
            write!(master, ",AUDIO=\"audio\"")?;
        }
        writeln!(master)?;
        writeln!(
            master,
            "{}/{}",
            relative_dir(video.kind),
            PLAYLIST_FILE_NAME
        )?;
    }

    for track in tracks.values() {
        let mut media = File::create(track.output_dir.join(PLAYLIST_FILE_NAME))?;
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
        writeln!(media, "#EXT-X-PLAYLIST-TYPE:VOD")?;
        writeln!(media, "#EXT-X-MAP:URI=\"{}\"", INIT_FILE_NAME)?;
        for (index, duration) in track.segment_durations.iter().enumerate() {
            writeln!(media, "#EXTINF:{duration:.6},")?;
            writeln!(media, "{}", segment_file_name(index))?;
        }
        writeln!(media, "#EXT-X-ENDLIST")?;
    }

    Ok(())
}

fn relative_dir(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => VIDEO_DIR,
        TrackKind::Audio => AUDIO_DIR,
        TrackKind::EncryptedVideo => VIDEO_ENC_DIR,
        TrackKind::EncryptedAudio => AUDIO_ENC_DIR,
    }
}

fn segment_file_name(index: usize) -> String {
    format!("{index}.mp4")
}

fn master_playlist_codecs(video: &TrackOutput, audio: Option<&TrackOutput>) -> String {
    match audio {
        Some(audio) => format!("{},{}", video.codecs, audio.codecs),
        None => video.codecs.clone(),
    }
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
    "divide currently supports fragmented inputs with at most one AVC video track and one MP4A audio track"
}

fn invalid_input(message: String) -> DivideError {
    DivideError::Io(io::Error::new(io::ErrorKind::InvalidInput, message))
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
