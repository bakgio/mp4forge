use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[cfg(feature = "async")]
use tokio::fs as tokio_fs;
#[cfg(feature = "async")]
use tokio::io::{AsyncBufReadExt, BufReader as AsyncBufReader};

use super::super::import::{
    CandidateSample, SegmentedMuxSourceSegment, SegmentedMuxSourceSegmentData,
    SegmentedMuxSourceSpec, TrackCandidate, direct_ingest_handler_name,
    direct_ingest_mux_policy_with_preferred_track_id,
};
use super::super::{MuxError, MuxTrackKind};
use super::detect::DetectedContainerPathKind;

/// One detected XML sidecar family supported by the current direct-ingest importer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mux) enum DetectedNhmlSidecarKind {
    Nhml,
    Nhnt,
}

/// One parsed staged-source spec recovered from an NHML/NHNT-like sidecar.
#[derive(Clone)]
pub(in crate::mux) enum ParsedNhmlSourceSpec {
    File(PathBuf),
    Segmented(SegmentedMuxSourceSpec),
}

/// Parsed direct-ingest tracks plus staged-source specs recovered from one sidecar.
#[derive(Clone)]
pub(in crate::mux) struct ParsedNhmlSource {
    pub(in crate::mux) source_specs: BTreeMap<usize, ParsedNhmlSourceSpec>,
    pub(in crate::mux) tracks: Vec<TrackCandidate>,
}

#[derive(Clone)]
struct ParsedTrackDescriptor {
    track_id: u32,
    kind: MuxTrackKind,
    timescale: u32,
    language: [u8; 3],
    handler_name: String,
    sample_entry_type: String,
    sample_entry_box: Vec<u8>,
    width: u16,
    height: u16,
    source_edit_media_time: Option<u64>,
    sample_roll_distance: Option<i16>,
}

#[derive(Clone)]
struct PendingSource {
    index: usize,
    path: PathBuf,
    segmented: bool,
    total_size: u64,
    segments: Vec<SegmentedMuxSourceSegment>,
}

#[derive(Clone)]
struct XmlTag {
    name: String,
    attrs: BTreeMap<String, String>,
    self_closing: bool,
    closing: bool,
}

#[derive(Default)]
struct NhmlParserState {
    source_specs: BTreeMap<usize, ParsedNhmlSourceSpec>,
    tracks: Vec<TrackCandidate>,
    pending_source: Option<PendingSource>,
    pending_track: Option<TrackCandidate>,
    packet_tracks: BTreeMap<u32, ParsedTrackDescriptor>,
    packet_samples: BTreeMap<u32, Vec<(usize, CandidateSample)>>,
    root_kind: Option<DetectedNhmlSidecarKind>,
    saw_root: bool,
}

/// Detects whether one path/prefix pair looks like one supported NHML/NHNT-like sidecar.
pub(in crate::mux) fn detect_nhml_sidecar_kind(
    path: &Path,
    prefix: &[u8],
) -> Option<DetectedContainerPathKind> {
    let root_name = extract_xml_root_name(prefix)?;
    if root_name.eq_ignore_ascii_case("nhml") || root_name.eq_ignore_ascii_case("nhmlstream") {
        return Some(DetectedContainerPathKind::Nhml);
    }
    if root_name.eq_ignore_ascii_case("nhnt") || root_name.eq_ignore_ascii_case("nhntstream") {
        return Some(DetectedContainerPathKind::Nhnt);
    }
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("nhml") {
        return Some(DetectedContainerPathKind::Nhml);
    }
    if extension.eq_ignore_ascii_case("nhnt") {
        return Some(DetectedContainerPathKind::Nhnt);
    }
    None
}

pub(in crate::mux) fn parse_nhml_source_sync(
    path: &Path,
    kind: DetectedNhmlSidecarKind,
) -> Result<ParsedNhmlSource, MuxError> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut first_line = true;
    let mut state = NhmlParserState::default();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let rendered = if first_line {
            first_line = false;
            line.strip_prefix('\u{FEFF}').unwrap_or(&line)
        } else {
            &line
        };
        state.push_line(path, kind, rendered)?;
    }
    state.finish(path, kind)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn parse_nhml_source_async(
    path: &Path,
    kind: DetectedNhmlSidecarKind,
) -> Result<ParsedNhmlSource, MuxError> {
    let file = tokio_fs::File::open(path).await?;
    let mut reader = AsyncBufReader::new(file);
    let mut line = String::new();
    let mut first_line = true;
    let mut state = NhmlParserState::default();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let rendered = if first_line {
            first_line = false;
            line.strip_prefix('\u{FEFF}').unwrap_or(&line)
        } else {
            &line
        };
        state.push_line(path, kind, rendered)?;
    }
    state.finish(path, kind)
}

impl NhmlParserState {
    fn push_line(
        &mut self,
        path: &Path,
        expected_kind: DetectedNhmlSidecarKind,
        raw_line: &str,
    ) -> Result<(), MuxError> {
        let Some(tag) =
            parse_xml_tag(raw_line).map_err(|message| invalid_sidecar(path, &message))?
        else {
            return Ok(());
        };
        let name = tag.name.as_str();
        if tag.closing {
            if name.eq_ignore_ascii_case("source") {
                let Some(source) = self.pending_source.take() else {
                    return Err(invalid_sidecar(
                        path,
                        "encountered `</source>` without `<source>`",
                    ));
                };
                insert_source_spec(path, &mut self.source_specs, source)?;
                return Ok(());
            }
            if name.eq_ignore_ascii_case("track") {
                let Some(track) = self.pending_track.take() else {
                    return Err(invalid_sidecar(
                        path,
                        "encountered `</track>` without `<track>`",
                    ));
                };
                self.tracks.push(track);
                return Ok(());
            }
            return Ok(());
        }

        if name.eq_ignore_ascii_case("nhml") || name.eq_ignore_ascii_case("nhmlstream") {
            self.root_kind = Some(DetectedNhmlSidecarKind::Nhml);
            self.saw_root = true;
            return Ok(());
        }
        if name.eq_ignore_ascii_case("nhnt") || name.eq_ignore_ascii_case("nhntstream") {
            self.root_kind = Some(DetectedNhmlSidecarKind::Nhnt);
            self.saw_root = true;
            return Ok(());
        }
        if !self.saw_root {
            return Err(invalid_sidecar(path, "missing NHML/NHNT root element"));
        }

        if name.eq_ignore_ascii_case("source") {
            let source = parse_source_tag(path, &tag.attrs)?;
            if tag.self_closing {
                insert_source_spec(path, &mut self.source_specs, source)?;
            } else if self.pending_source.replace(source).is_some() {
                return Err(invalid_sidecar(
                    path,
                    "encountered nested `<source>` elements",
                ));
            }
            return Ok(());
        }

        if name.eq_ignore_ascii_case("segment") {
            let Some(source) = self.pending_source.as_mut() else {
                return Err(invalid_sidecar(
                    path,
                    "encountered `<segment>` outside `<source>`",
                ));
            };
            source.segments.push(parse_segment_tag(path, &tag.attrs)?);
            return Ok(());
        }

        match expected_kind {
            DetectedNhmlSidecarKind::Nhml => {
                if name.eq_ignore_ascii_case("track") {
                    let descriptor = parse_track_descriptor(path, &tag.attrs)?;
                    let track = track_from_descriptor(descriptor, Vec::new());
                    if tag.self_closing {
                        self.tracks.push(track);
                    } else if self.pending_track.replace(track).is_some() {
                        return Err(invalid_sidecar(
                            path,
                            "encountered nested `<track>` elements",
                        ));
                    }
                    return Ok(());
                }
                if name.eq_ignore_ascii_case("sample") {
                    let Some(track) = self.pending_track.as_mut() else {
                        return Err(invalid_sidecar(
                            path,
                            "encountered `<sample>` outside `<track>`",
                        ));
                    };
                    track.samples.push(parse_sample_tag(path, &tag.attrs)?);
                    return Ok(());
                }
            }
            DetectedNhmlSidecarKind::Nhnt => {
                if name.eq_ignore_ascii_case("track") {
                    let descriptor = parse_track_descriptor(path, &tag.attrs)?;
                    self.packet_tracks.insert(descriptor.track_id, descriptor);
                    return Ok(());
                }
                if name.eq_ignore_ascii_case("packet") || name.eq_ignore_ascii_case("nhntsample") {
                    let (track_id, packet_index, sample) = parse_packet_tag(path, &tag.attrs)?;
                    self.packet_samples
                        .entry(track_id)
                        .or_default()
                        .push((packet_index, sample));
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn finish(
        mut self,
        path: &Path,
        expected_kind: DetectedNhmlSidecarKind,
    ) -> Result<ParsedNhmlSource, MuxError> {
        let Some(actual_kind) = self.root_kind else {
            return Err(invalid_sidecar(path, "missing NHML/NHNT root element"));
        };
        if actual_kind != expected_kind {
            return Err(invalid_sidecar(
                path,
                "sidecar root does not match the detected sidecar kind",
            ));
        }
        if self.pending_source.is_some() {
            return Err(invalid_sidecar(path, "unterminated `<source>` element"));
        }
        if self.pending_track.is_some() {
            return Err(invalid_sidecar(path, "unterminated `<track>` element"));
        }

        if expected_kind == DetectedNhmlSidecarKind::Nhnt {
            for (track_id, descriptor) in self.packet_tracks {
                let Some(mut samples) = self.packet_samples.remove(&track_id) else {
                    return Err(invalid_sidecar(
                        path,
                        &format!("NHNT track {track_id} does not carry any packet entries"),
                    ));
                };
                samples.sort_by_key(|(packet_index, _)| *packet_index);
                let samples = samples.into_iter().map(|(_, sample)| sample).collect();
                self.tracks.push(track_from_descriptor(descriptor, samples));
            }
            if !self.packet_samples.is_empty() {
                let missing_track_id = *self.packet_samples.keys().next().unwrap();
                return Err(invalid_sidecar(
                    path,
                    &format!(
                        "NHNT packet track {missing_track_id} is missing the required `<track ... />` metadata entry"
                    ),
                ));
            }
        }

        Ok(ParsedNhmlSource {
            source_specs: self.source_specs,
            tracks: self.tracks,
        })
    }
}

fn parse_source_tag(
    path: &Path,
    attrs: &BTreeMap<String, String>,
) -> Result<PendingSource, MuxError> {
    let index = required_attr_usize(path, attrs, "index")?;
    let path_attr = required_attr_string(path, attrs, "path")?;
    let segmented = required_attr_bool(path, attrs, "segmented")?;
    let total_size = required_attr_u64(path, attrs, "totalSize")?;
    Ok(PendingSource {
        index,
        path: resolve_sidecar_path(path, &path_attr),
        segmented,
        total_size,
        segments: Vec::new(),
    })
}

fn parse_segment_tag(
    path: &Path,
    attrs: &BTreeMap<String, String>,
) -> Result<SegmentedMuxSourceSegment, MuxError> {
    let kind = required_attr_string(path, attrs, "kind")?;
    let logical_offset = required_attr_u64(path, attrs, "logicalOffset")?;
    let logical_size = required_attr_u64(path, attrs, "logicalSize")?;
    let data = match kind.as_str() {
        "prefix" => {
            let data_hex = required_attr_string(path, attrs, "dataHex")?;
            let bytes = decode_hex(path, "dataHex", &data_hex)?;
            let prefix: [u8; 4] = bytes.try_into().map_err(|_| {
                invalid_sidecar(path, "prefix segment `dataHex` must decode to four bytes")
            })?;
            if logical_size != 4 {
                return Err(invalid_sidecar(
                    path,
                    "prefix segment `logicalSize` must stay equal to four bytes",
                ));
            }
            SegmentedMuxSourceSegmentData::Prefix(prefix)
        }
        "bytes" => {
            let data_hex = required_attr_string(path, attrs, "dataHex")?;
            let bytes = decode_hex(path, "dataHex", &data_hex)?;
            if logical_size != bytes.len() as u64 {
                return Err(invalid_sidecar(
                    path,
                    "inline segment `logicalSize` does not match the decoded `dataHex` length",
                ));
            }
            SegmentedMuxSourceSegmentData::Bytes(bytes)
        }
        "file_range" => {
            let source_offset = required_attr_u64(path, attrs, "sourceOffset")?;
            let size = u32::try_from(logical_size)
                .map_err(|_| invalid_sidecar(path, "file-range segment size exceeds u32"))?;
            if let Some(source_path) = attrs.get("sourcePath") {
                SegmentedMuxSourceSegmentData::ExternalFileRange {
                    path: resolve_sidecar_path(path, source_path),
                    source_offset,
                    size,
                }
            } else {
                SegmentedMuxSourceSegmentData::FileRange {
                    source_offset,
                    size,
                }
            }
        }
        other => {
            return Err(invalid_sidecar(
                path,
                &format!("unsupported NHML/NHNT source segment kind `{other}`"),
            ));
        }
    };
    Ok(SegmentedMuxSourceSegment {
        logical_offset,
        data,
    })
}

fn insert_source_spec(
    path: &Path,
    source_specs: &mut BTreeMap<usize, ParsedNhmlSourceSpec>,
    source: PendingSource,
) -> Result<(), MuxError> {
    let spec = if source.segmented {
        if source.segments.is_empty() {
            return Err(invalid_sidecar(
                path,
                "segmented sidecar sources must carry one or more `<segment ... />` entries",
            ));
        }
        ParsedNhmlSourceSpec::Segmented(SegmentedMuxSourceSpec {
            path: source.path,
            segments: source.segments,
            total_size: source.total_size,
        })
    } else {
        ParsedNhmlSourceSpec::File(source.path)
    };
    if source_specs.insert(source.index, spec).is_some() {
        return Err(invalid_sidecar(
            path,
            &format!("duplicate staged source index {}", source.index),
        ));
    }
    Ok(())
}

fn parse_track_descriptor(
    path: &Path,
    attrs: &BTreeMap<String, String>,
) -> Result<ParsedTrackDescriptor, MuxError> {
    let track_id = required_attr_u32(path, attrs, "trackID")?;
    let kind = parse_track_kind(path, &required_attr_string(path, attrs, "kind")?)?;
    let timescale = required_attr_u32(path, attrs, "timescale")?;
    let language = parse_language(path, &required_attr_string(path, attrs, "language")?)?;
    let handler_name = attrs
        .get("handlerName")
        .cloned()
        .unwrap_or_else(|| default_handler_name_for_kind(kind));
    let sample_entry_box_hex = required_attr_string(path, attrs, "sampleEntryBoxHex")?;
    let sample_entry_box = decode_hex(path, "sampleEntryBoxHex", &sample_entry_box_hex)?;
    if sample_entry_box.len() < 8 {
        return Err(invalid_sidecar(
            path,
            "sample entry box hex must decode to one full MP4 box header and payload",
        ));
    }
    let declared_box_size = u32::from_be_bytes([
        sample_entry_box[0],
        sample_entry_box[1],
        sample_entry_box[2],
        sample_entry_box[3],
    ]);
    if declared_box_size != sample_entry_box.len() as u32 {
        return Err(invalid_sidecar(
            path,
            "sample entry box hex does not decode to one self-sized MP4 box payload",
        ));
    }
    let sample_entry_type = attrs
        .get("sampleEntryType")
        .cloned()
        .unwrap_or_else(|| String::from_utf8_lossy(&sample_entry_box[4..8]).into_owned());
    let width = optional_attr_u16(path, attrs, "width")?.unwrap_or(0);
    let height = optional_attr_u16(path, attrs, "height")?.unwrap_or(0);
    let source_edit_media_time = optional_attr_u64(path, attrs, "sourceEditMediaTime")?;
    let sample_roll_distance = optional_attr_i16(path, attrs, "sampleRollDistance")?;
    Ok(ParsedTrackDescriptor {
        track_id,
        kind,
        timescale,
        language,
        handler_name,
        sample_entry_type,
        sample_entry_box,
        width,
        height,
        source_edit_media_time,
        sample_roll_distance,
    })
}

fn track_from_descriptor(
    descriptor: ParsedTrackDescriptor,
    samples: Vec<CandidateSample>,
) -> TrackCandidate {
    let codec_label =
        codec_label_from_sample_entry_type(&descriptor.sample_entry_type, descriptor.kind);
    let mut mux_policy = direct_ingest_mux_policy_with_preferred_track_id(
        &codec_label,
        descriptor.kind,
        descriptor.track_id,
    );
    if let Some(sample_roll_distance) = descriptor.sample_roll_distance {
        mux_policy = mux_policy.with_sample_roll_distance(sample_roll_distance);
    }
    TrackCandidate {
        track_id: descriptor.track_id,
        kind: descriptor.kind,
        timescale: descriptor.timescale,
        language: descriptor.language,
        handler_name: descriptor.handler_name,
        mux_policy,
        width: descriptor.width,
        height: descriptor.height,
        sample_entry_box: descriptor.sample_entry_box,
        source_edit_media_time: descriptor.source_edit_media_time,
        samples,
    }
}

fn parse_sample_tag(
    path: &Path,
    attrs: &BTreeMap<String, String>,
) -> Result<CandidateSample, MuxError> {
    Ok(CandidateSample {
        source_index: required_attr_usize(path, attrs, "sourceIndex")?,
        data_offset: required_attr_u64(path, attrs, "dataOffset")?,
        data_size: required_attr_u32(path, attrs, "dataSize")?,
        duration: required_attr_u32(path, attrs, "duration")?,
        composition_time_offset: required_attr_i32(path, attrs, "compositionTimeOffset")?,
        is_sync_sample: required_attr_bool(path, attrs, "sync")?,
    })
}

fn parse_packet_tag(
    path: &Path,
    attrs: &BTreeMap<String, String>,
) -> Result<(u32, usize, CandidateSample), MuxError> {
    let track_id = required_attr_u32(path, attrs, "trackID")?;
    let packet_index = required_attr_usize(path, attrs, "packetIndex")?;
    Ok((
        track_id,
        packet_index,
        CandidateSample {
            source_index: required_attr_usize(path, attrs, "sourceIndex")?,
            data_offset: required_attr_u64(path, attrs, "dataOffset")?,
            data_size: required_attr_u32(path, attrs, "dataSize")?,
            duration: required_attr_u32(path, attrs, "duration")?,
            composition_time_offset: required_attr_i32(path, attrs, "compositionTimeOffset")?,
            is_sync_sample: required_attr_bool(path, attrs, "sync")?,
        },
    ))
}

fn parse_xml_tag(line: &str) -> Result<Option<XmlTag>, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("<?xml") {
        return Ok(None);
    }
    if !trimmed.starts_with('<') || !trimmed.ends_with('>') {
        return Err(format!("unsupported NHML/NHNT line `{trimmed}`"));
    }
    if let Some(content) = trimmed
        .strip_prefix("</")
        .and_then(|value| value.strip_suffix('>'))
    {
        return Ok(Some(XmlTag {
            name: content.trim().to_string(),
            attrs: BTreeMap::new(),
            self_closing: false,
            closing: true,
        }));
    }
    let mut inner = trimmed
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .ok_or_else(|| format!("unsupported NHML/NHNT line `{trimmed}`"))?
        .trim();
    let self_closing = inner.ends_with('/');
    if self_closing {
        inner = inner[..inner.len() - 1].trim_end();
    }
    let name_end = inner.find(char::is_whitespace).unwrap_or(inner.len());
    let name = inner[..name_end].to_string();
    let mut attrs = BTreeMap::new();
    let mut cursor = inner[name_end..].trim_start();
    while !cursor.is_empty() {
        let Some(eq_pos) = cursor.find('=') else {
            return Err(format!("malformed NHML/NHNT attribute list in `{trimmed}`"));
        };
        let key = cursor[..eq_pos].trim();
        if key.is_empty() {
            return Err(format!("malformed NHML/NHNT attribute list in `{trimmed}`"));
        }
        let rest = cursor[eq_pos + 1..].trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            return Err(format!(
                "NHML/NHNT attribute `{key}` in `{trimmed}` must use double quotes"
            ));
        };
        let Some(value_end) = rest.find('"') else {
            return Err(format!(
                "unterminated NHML/NHNT attribute `{key}` in `{trimmed}`"
            ));
        };
        attrs.insert(key.to_string(), xml_unescape_attr(&rest[..value_end])?);
        cursor = rest[value_end + 1..].trim_start();
    }
    Ok(Some(XmlTag {
        name,
        attrs,
        self_closing,
        closing: false,
    }))
}

fn extract_xml_root_name(prefix: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(prefix).ok()?;
    let text = text.trim_start_matches('\u{FEFF}').trim_start();
    let text = if text.starts_with("<?xml") {
        let end = text.find("?>")?;
        text[end + 2..].trim_start()
    } else {
        text
    };
    let body = text.strip_prefix('<')?;
    let name_end = body
        .find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
        .unwrap_or(body.len());
    if name_end == 0 {
        None
    } else {
        Some(body[..name_end].to_string())
    }
}

fn xml_unescape_attr(value: &str) -> Result<String, String> {
    let mut rendered = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            rendered.push(ch);
            continue;
        }
        let mut entity = String::new();
        for next in chars.by_ref() {
            if next == ';' {
                break;
            }
            entity.push(next);
        }
        match entity.as_str() {
            "amp" => rendered.push('&'),
            "lt" => rendered.push('<'),
            "gt" => rendered.push('>'),
            "quot" => rendered.push('"'),
            "#39" => rendered.push('\''),
            _ => return Err(format!("unsupported XML entity `&{entity};`")),
        }
    }
    Ok(rendered)
}

fn required_attr_string(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<String, MuxError> {
    attrs
        .get(key)
        .cloned()
        .ok_or_else(|| invalid_sidecar(path, &format!("missing required attribute `{key}`")))
}

fn required_attr_bool(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<bool, MuxError> {
    match required_attr_string(path, attrs, key)?.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid_sidecar(
            path,
            &format!("attribute `{key}` must stay `true` or `false`"),
        )),
    }
}

fn required_attr_u32(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<u32, MuxError> {
    required_attr_string(path, attrs, key)?
        .parse::<u32>()
        .map_err(|_| {
            invalid_sidecar(
                path,
                &format!("attribute `{key}` must be one unsigned 32-bit integer"),
            )
        })
}

fn required_attr_u64(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<u64, MuxError> {
    required_attr_string(path, attrs, key)?
        .parse::<u64>()
        .map_err(|_| {
            invalid_sidecar(
                path,
                &format!("attribute `{key}` must be one unsigned 64-bit integer"),
            )
        })
}

fn required_attr_i32(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<i32, MuxError> {
    required_attr_string(path, attrs, key)?
        .parse::<i32>()
        .map_err(|_| {
            invalid_sidecar(
                path,
                &format!("attribute `{key}` must be one signed 32-bit integer"),
            )
        })
}

fn required_attr_usize(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<usize, MuxError> {
    required_attr_string(path, attrs, key)?
        .parse::<usize>()
        .map_err(|_| {
            invalid_sidecar(
                path,
                &format!("attribute `{key}` must be one platform-sized unsigned integer"),
            )
        })
}

fn optional_attr_u16(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<u16>, MuxError> {
    let Some(value) = attrs.get(key) else {
        return Ok(None);
    };
    value.parse::<u16>().map(Some).map_err(|_| {
        invalid_sidecar(
            path,
            &format!("attribute `{key}` must be one unsigned 16-bit integer"),
        )
    })
}

fn optional_attr_u64(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<u64>, MuxError> {
    let Some(value) = attrs.get(key) else {
        return Ok(None);
    };
    value.parse::<u64>().map(Some).map_err(|_| {
        invalid_sidecar(
            path,
            &format!("attribute `{key}` must be one unsigned 64-bit integer"),
        )
    })
}

fn optional_attr_i16(
    path: &Path,
    attrs: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<i16>, MuxError> {
    let Some(value) = attrs.get(key) else {
        return Ok(None);
    };
    value.parse::<i16>().map(Some).map_err(|_| {
        invalid_sidecar(
            path,
            &format!("attribute `{key}` must be one signed 16-bit integer"),
        )
    })
}

fn parse_track_kind(path: &Path, value: &str) -> Result<MuxTrackKind, MuxError> {
    match value {
        "audio" => Ok(MuxTrackKind::Audio),
        "video" => Ok(MuxTrackKind::Video),
        "text" => Ok(MuxTrackKind::Text),
        "subtitle" => Ok(MuxTrackKind::Subtitle),
        _ => Err(invalid_sidecar(
            path,
            &format!("unsupported sidecar track kind `{value}`"),
        )),
    }
}

fn parse_language(path: &Path, value: &str) -> Result<[u8; 3], MuxError> {
    let bytes = value.as_bytes();
    if bytes.len() != 3 || !bytes.iter().all(|byte| byte.is_ascii_lowercase()) {
        return Err(invalid_sidecar(
            path,
            "sidecar language codes must stay one three-letter lowercase ISO-639-2 code",
        ));
    }
    Ok([bytes[0], bytes[1], bytes[2]])
}

fn default_handler_name_for_kind(kind: MuxTrackKind) -> String {
    match kind {
        MuxTrackKind::Audio => direct_ingest_handler_name("audio"),
        MuxTrackKind::Video => direct_ingest_handler_name("h264"),
        MuxTrackKind::Text => "TextHandler".to_string(),
        MuxTrackKind::Subtitle => "SubtitleHandler".to_string(),
    }
}

fn codec_label_from_sample_entry_type(sample_entry_type: &str, kind: MuxTrackKind) -> String {
    match sample_entry_type {
        "Opus" => "opus".to_string(),
        "fLaC" => "flac".to_string(),
        "vp08" => "vp8".to_string(),
        "vp09" => "vp9".to_string(),
        "av01" => "av1".to_string(),
        "avc1" | "avc3" | "AVC1" => "h264".to_string(),
        "hvc1" | "hev1" => "h265".to_string(),
        "mhm1" | "mha1" => "mhas".to_string(),
        "fpcm" | "ipcm" => "pcm".to_string(),
        "alaw" => "alaw".to_string(),
        "ulaw" => "mulaw".to_string(),
        _ => match kind {
            MuxTrackKind::Audio => "audio".to_string(),
            MuxTrackKind::Video => "video".to_string(),
            MuxTrackKind::Text => "text".to_string(),
            MuxTrackKind::Subtitle => "subtitle".to_string(),
        },
    }
}

fn decode_hex(path: &Path, key: &str, value: &str) -> Result<Vec<u8>, MuxError> {
    if !value.len().is_multiple_of(2) {
        return Err(invalid_sidecar(
            path,
            &format!("attribute `{key}` must carry one even-length hexadecimal string"),
        ));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let as_bytes = value.as_bytes();
    let mut index = 0usize;
    while index < as_bytes.len() {
        let hi = decode_hex_nibble(path, key, as_bytes[index])?;
        let lo = decode_hex_nibble(path, key, as_bytes[index + 1])?;
        bytes.push((hi << 4) | lo);
        index += 2;
    }
    Ok(bytes)
}

fn decode_hex_nibble(path: &Path, key: &str, value: u8) -> Result<u8, MuxError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(invalid_sidecar(
            path,
            &format!("attribute `{key}` must carry one hexadecimal string"),
        )),
    }
}

fn resolve_sidecar_path(sidecar_path: &Path, value: &str) -> PathBuf {
    let candidate = PathBuf::from(value);
    if candidate.is_absolute() {
        candidate
    } else if let Some(parent) = sidecar_path.parent() {
        parent.join(candidate)
    } else {
        candidate
    }
}

fn invalid_sidecar(path: &Path, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: path.display().to_string(),
        message: format!("invalid NHML/NHNT sidecar: {message}"),
    }
}
