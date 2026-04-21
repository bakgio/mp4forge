//! Fragmented-file split command support.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::FourCc;
use crate::boxes::iso14496_12::{Tfhd, Tkhd};
use crate::extract::{ExtractError, extract_boxes_with_payload};
use crate::header::{BoxInfo, HeaderError};
use crate::probe::{ProbeError, ProbeInfo, TrackCodec, probe};
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
const MASTER_PLAYLIST_CODECS: &str = "avc1.64001f,mp4a.40.2";

/// Runs the divide subcommand with `args`, writing files under `OUTPUT_DIR`.
pub fn run<E>(args: &[String], stderr: &mut E) -> i32
where
    E: Write,
{
    match run_inner(args) {
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
    writeln!(writer, "USAGE: mp4forge divide INPUT.mp4 OUTPUT_DIR")
}

fn run_inner(args: &[String]) -> Result<(), DivideError> {
    if args.len() != 2 {
        return Err(DivideError::UsageRequested);
    }

    let input_path = Path::new(&args[0]);
    let output_dir = Path::new(&args[1]);
    let mut input = File::open(input_path)?;
    divide_reader(&mut input, output_dir)
}

/// Splits a fragmented MP4 reader into per-track outputs under `output_dir`.
pub fn divide_reader<R>(reader: &mut R, output_dir: &Path) -> Result<(), DivideError>
where
    R: Read + Seek,
{
    let summary = probe(reader)?;
    let mut tracks = build_track_outputs(&summary, output_dir)?;
    if tracks.is_empty() {
        return Err(DivideError::NoSupportedTracks);
    }

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

struct TrackOutput {
    kind: TrackKind,
    width: Option<u16>,
    height: Option<u16>,
    segment_durations: Vec<f64>,
    bandwidth: u64,
    output_dir: PathBuf,
    init_writer: Writer<File>,
    next_segment_index: usize,
}

fn build_track_outputs(
    summary: &ProbeInfo,
    output_dir: &Path,
) -> Result<BTreeMap<u32, TrackOutput>, DivideError> {
    let mut tracks = BTreeMap::new();
    for track in &summary.tracks {
        let Some((kind, dir_name, width, height)) = track_layout(track) else {
            continue;
        };

        let track_dir = output_dir.join(dir_name);
        fs::create_dir_all(&track_dir)?;
        let init_writer = Writer::new(File::create(track_dir.join(INIT_FILE_NAME))?);

        let segment_durations = summary
            .segments
            .iter()
            .filter(|segment| segment.track_id == track.track_id)
            .map(|segment| {
                if track.timescale == 0 {
                    0.0
                } else {
                    segment.duration as f64 / f64::from(track.timescale)
                }
            })
            .collect::<Vec<_>>();

        tracks.insert(
            track.track_id,
            TrackOutput {
                kind,
                width,
                height,
                segment_durations,
                bandwidth: 0,
                output_dir: track_dir,
                init_writer,
                next_segment_index: 0,
            },
        );
    }

    Ok(tracks)
}

fn track_layout(
    track: &crate::probe::TrackInfo,
) -> Option<(TrackKind, &'static str, Option<u16>, Option<u16>)> {
    match (track.codec, track.encrypted) {
        (TrackCodec::Avc1, false) => Some((
            TrackKind::Video,
            VIDEO_DIR,
            track.avc.as_ref().map(|avc| avc.width),
            track.avc.as_ref().map(|avc| avc.height),
        )),
        (TrackCodec::Avc1, true) => Some((
            TrackKind::EncryptedVideo,
            VIDEO_ENC_DIR,
            track.avc.as_ref().map(|avc| avc.width),
            track.avc.as_ref().map(|avc| avc.height),
        )),
        (TrackCodec::Mp4a, false) => Some((TrackKind::Audio, AUDIO_DIR, None, None)),
        (TrackCodec::Mp4a, true) => Some((TrackKind::EncryptedAudio, AUDIO_ENC_DIR, None, None)),
        (TrackCodec::Unknown, _) => None,
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
            writeln!(
                master,
                "#EXT-X-MEDIA:TYPE=AUDIO,URI=\"{}/{}\",GROUP-ID=\"audio\",NAME=\"audio\",AUTOSELECT=YES,CHANNELS=\"2\"",
                relative_dir(audio.kind),
                PLAYLIST_FILE_NAME
            )?;
        }

        write!(
            master,
            "#EXT-X-STREAM-INF:BANDWIDTH={},CODECS=\"{}\"",
            video.bandwidth, MASTER_PLAYLIST_CODECS
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
            Self::NoSupportedTracks => f.write_str("no supported tracks found"),
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
