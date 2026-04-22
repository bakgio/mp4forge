//! File-summary helpers built on the extraction and box layers, with byte-slice convenience entry
//! points for in-memory probe flows.

use std::error::Error;
use std::fmt;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

use crate::BoxInfo;
use crate::FourCc;
use crate::bitio::BitReader;
use crate::boxes::iso14496_12::{
    AVCDecoderConfiguration, AudioSampleEntry, Co64, Ctts, Mvhd, Stco, Stsc, Stsz, Stts, Tfdt,
    Tfhd, Tkhd, Trun, VisualSampleEntry,
};
use crate::boxes::iso14496_14::Esds;
use crate::codec::{CodecBox, CodecError, ImmutableBox, unmarshal};
use crate::extract::{ExtractError, ExtractedBox, extract_boxes, extract_boxes_with_payload};
use crate::header::HeaderError;
use crate::walk::BoxPath;

const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const MVHD: FourCc = FourCc::from_bytes(*b"mvhd");
const TRAK: FourCc = FourCc::from_bytes(*b"trak");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const TKHD: FourCc = FourCc::from_bytes(*b"tkhd");
const EDTS: FourCc = FourCc::from_bytes(*b"edts");
const ELST: FourCc = FourCc::from_bytes(*b"elst");
const MDIA: FourCc = FourCc::from_bytes(*b"mdia");
const MDHD: FourCc = FourCc::from_bytes(*b"mdhd");
const MINF: FourCc = FourCc::from_bytes(*b"minf");
const STBL: FourCc = FourCc::from_bytes(*b"stbl");
const STSD: FourCc = FourCc::from_bytes(*b"stsd");
const AVC1: FourCc = FourCc::from_bytes(*b"avc1");
const AVCC: FourCc = FourCc::from_bytes(*b"avcC");
const ENCV: FourCc = FourCc::from_bytes(*b"encv");
const MP4A: FourCc = FourCc::from_bytes(*b"mp4a");
const WAVE: FourCc = FourCc::from_bytes(*b"wave");
const ESDS: FourCc = FourCc::from_bytes(*b"esds");
const ENCA: FourCc = FourCc::from_bytes(*b"enca");
const STCO: FourCc = FourCc::from_bytes(*b"stco");
const CO64: FourCc = FourCc::from_bytes(*b"co64");
const STTS: FourCc = FourCc::from_bytes(*b"stts");
const CTTS: FourCc = FourCc::from_bytes(*b"ctts");
const STSC: FourCc = FourCc::from_bytes(*b"stsc");
const STSZ: FourCc = FourCc::from_bytes(*b"stsz");
const TRAF: FourCc = FourCc::from_bytes(*b"traf");
const TFHD: FourCc = FourCc::from_bytes(*b"tfhd");
const TFDT: FourCc = FourCc::from_bytes(*b"tfdt");
const TRUN: FourCc = FourCc::from_bytes(*b"trun");

/// High-level summary of one MP4 file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeInfo {
    /// Major brand from the root `ftyp` box.
    pub major_brand: FourCc,
    /// Minor version from the root `ftyp` box.
    pub minor_version: u32,
    /// Compatible brands listed by the root `ftyp` box.
    pub compatible_brands: Vec<FourCc>,
    /// Whether the `moov` box appears before the first `mdat`.
    pub fast_start: bool,
    /// Movie timescale from `mvhd`.
    pub timescale: u32,
    /// Movie duration from `mvhd`.
    pub duration: u64,
    /// Per-track summaries extracted from `trak` boxes.
    pub tracks: Vec<TrackInfo>,
    /// Fragment summaries extracted from `moof` boxes.
    pub segments: Vec<SegmentInfo>,
}

impl Default for ProbeInfo {
    fn default() -> Self {
        Self {
            major_brand: FourCc::ANY,
            minor_version: 0,
            compatible_brands: Vec::new(),
            fast_start: false,
            timescale: 0,
            duration: 0,
            tracks: Vec::new(),
            segments: Vec::new(),
        }
    }
}

/// Summary of one logical media track.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    /// Track identifier from `tkhd`.
    pub track_id: u32,
    /// Media timescale from `mdhd`.
    pub timescale: u32,
    /// Media duration from `mdhd`.
    pub duration: u64,
    /// High-level codec classification derived from the sample description.
    pub codec: TrackCodec,
    /// Whether the track uses encrypted sample entries.
    pub encrypted: bool,
    /// Edit-list entries when `elst` is present.
    pub edit_list: Vec<EditListEntry>,
    /// Expanded per-sample timing and size data.
    pub samples: Vec<SampleInfo>,
    /// Expanded chunk offsets and sample counts.
    pub chunks: Vec<ChunkInfo>,
    /// AVC configuration summary when the track is AVC-based.
    pub avc: Option<AvcDecoderConfigInfo>,
    /// AAC configuration summary when the track is MP4A-based.
    pub mp4a: Option<Mp4aInfo>,
}

/// Coarse codec classification used by the probe surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackCodec {
    /// No recognized sample entry was found.
    #[default]
    Unknown,
    /// AVC/H.264 video carried by `avc1` or `encv`.
    Avc1,
    /// MPEG-4 audio carried by `mp4a` or `enca`.
    Mp4a,
}

/// One edit-list entry from `elst`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditListEntry {
    /// Media time selected by the edit entry.
    pub media_time: i64,
    /// Presentation duration covered by the edit entry.
    pub segment_duration: u64,
}

/// Expanded sample timing and size information.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleInfo {
    /// Sample size in bytes.
    pub size: u32,
    /// Decode-time delta in the track timescale.
    pub time_delta: u32,
    /// Composition-time offset in the track timescale.
    pub composition_time_offset: i64,
}

/// Expanded chunk placement information.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChunkInfo {
    /// File offset of the chunk payload.
    pub data_offset: u64,
    /// Number of samples stored in the chunk.
    pub samples_per_chunk: u32,
}

/// Summary of AVC decoder configuration attached to a video track.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AvcDecoderConfigInfo {
    /// AVC decoder configuration version.
    pub configuration_version: u8,
    /// AVC profile indication.
    pub profile: u8,
    /// AVC profile-compatibility byte.
    pub profile_compatibility: u8,
    /// AVC level indication.
    pub level: u8,
    /// Length-prefix width used for NAL units.
    pub length_size: u16,
    /// Display width from the visual sample entry.
    pub width: u16,
    /// Display height from the visual sample entry.
    pub height: u16,
}

/// AAC profile details derived from an `esds` descriptor stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AacProfileInfo {
    /// MPEG object type indication from the decoder-config descriptor.
    pub object_type_indication: u8,
    /// AAC audio object type derived from the decoder-specific info payload.
    pub audio_object_type: u8,
}

/// Summary of MP4A decoder configuration attached to an audio track.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mp4aInfo {
    /// MPEG object type indication from the decoder-config descriptor.
    pub object_type_indication: u8,
    /// AAC audio object type derived from the decoder-specific info payload.
    pub audio_object_type: u8,
    /// Channel count from the audio sample entry.
    pub channel_count: u16,
}

/// Summary of one fragmented-media segment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SegmentInfo {
    /// Track identifier from `tfhd`.
    pub track_id: u32,
    /// File offset of the owning `moof` box.
    pub moof_offset: u64,
    /// Base media decode time from `tfdt`.
    pub base_media_decode_time: u64,
    /// Default sample duration from `tfhd`.
    pub default_sample_duration: u32,
    /// Number of samples described by `trun`.
    pub sample_count: u32,
    /// Segment duration in the track timescale.
    pub duration: u32,
    /// Minimum composition-time offset observed across the run.
    pub composition_time_offset: i32,
    /// Total sample payload size in bytes.
    pub size: u32,
}

/// Probes a file and returns high-level movie, track, and fragment summaries.
pub fn probe<R>(reader: &mut R) -> Result<ProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    let infos = extract_boxes(
        reader,
        None,
        &[
            BoxPath::from([FTYP]),
            BoxPath::from([MOOV]),
            BoxPath::from([MOOV, MVHD]),
            BoxPath::from([MOOV, TRAK]),
            BoxPath::from([MOOF]),
            BoxPath::from([MDAT]),
        ],
    )?;

    let mut summary = ProbeInfo::default();
    let mut mdat_appeared = false;

    for info in infos {
        match info.box_type() {
            FTYP => {
                let ftyp = read_payload_as::<_, crate::boxes::iso14496_12::Ftyp>(reader, &info)?;
                summary.major_brand = ftyp.major_brand;
                summary.minor_version = ftyp.minor_version;
                summary.compatible_brands = ftyp.compatible_brands;
            }
            MOOV => {
                summary.fast_start = !mdat_appeared;
            }
            MVHD => {
                let mvhd = read_payload_as::<_, Mvhd>(reader, &info)?;
                summary.timescale = mvhd.timescale;
                summary.duration = mvhd.duration();
            }
            TRAK => {
                summary.tracks.push(probe_trak(reader, &info)?);
            }
            MOOF => {
                summary.segments.push(probe_moof(reader, &info)?);
            }
            MDAT => {
                mdat_appeared = true;
            }
            _ => {}
        }
    }

    Ok(summary)
}

/// Probes an in-memory MP4 byte slice and returns high-level movie, track, and fragment
/// summaries.
///
/// This is equivalent to calling [`probe`] with `Cursor<&[u8]>`.
pub fn probe_bytes(input: &[u8]) -> Result<ProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe(&mut reader)
}

/// Legacy fragmented-file probe entry point that currently aliases [`probe`].
pub fn probe_fra<R>(reader: &mut R) -> Result<ProbeInfo, ProbeError>
where
    R: Read + Seek,
{
    probe(reader)
}

/// Legacy fragmented-file probe entry point for in-memory MP4 bytes.
///
/// This currently aliases [`probe_bytes`] for callers that already use the `probe_fra` naming.
pub fn probe_fra_bytes(input: &[u8]) -> Result<ProbeInfo, ProbeError> {
    let mut reader = Cursor::new(input);
    probe_fra(&mut reader)
}

/// Detects the AAC object profile exposed by an `esds` descriptor stream.
pub fn detect_aac_profile(esds: &Esds) -> Result<Option<AacProfileInfo>, ProbeError> {
    let Some(decoder_config) = esds.decoder_config_descriptor() else {
        return Ok(None);
    };
    if decoder_config.object_type_indication != 0x40 {
        return Ok(Some(AacProfileInfo {
            object_type_indication: decoder_config.object_type_indication,
            audio_object_type: 0,
        }));
    }

    let specific_info = esds
        .decoder_specific_info()
        .ok_or(ProbeError::MissingDescriptor(
            "decoder specific info descriptor",
        ))?;

    let mut reader = BitReader::new(Cursor::new(specific_info));
    let mut remaining_bits = specific_info.len() * 8;

    let (audio_object_type, read_bits) = get_audio_object_type(&mut reader)?;
    remaining_bits = remaining_bits.saturating_sub(read_bits);

    let sampling_frequency_index = read_bits_u8(&mut reader, 4)?;
    remaining_bits = remaining_bits.saturating_sub(4);
    if sampling_frequency_index == 0x0f {
        let _ = read_bits_u32(&mut reader, 24)?;
        remaining_bits = remaining_bits.saturating_sub(24);
    }

    if audio_object_type == 2 && remaining_bits >= 20 {
        let _ = read_bits_u8(&mut reader, 4)?;
        remaining_bits = remaining_bits.saturating_sub(4);
        let sync_extension_type = read_bits_u16(&mut reader, 11)?;
        remaining_bits = remaining_bits.saturating_sub(11);
        if sync_extension_type == 0x02b7 {
            let (ext_audio_object_type, _) = get_audio_object_type(&mut reader)?;
            if ext_audio_object_type == 5 || ext_audio_object_type == 22 {
                let sbr = read_bits_u8(&mut reader, 1)?;
                remaining_bits = remaining_bits.saturating_sub(1);
                if sbr != 0 {
                    if ext_audio_object_type == 5 {
                        let ext_sampling_frequency_index = read_bits_u8(&mut reader, 4)?;
                        remaining_bits = remaining_bits.saturating_sub(4);
                        if ext_sampling_frequency_index == 0x0f {
                            let _ = read_bits_u32(&mut reader, 24)?;
                            remaining_bits = remaining_bits.saturating_sub(24);
                        }
                        if remaining_bits >= 12 {
                            let sync_extension_type = read_bits_u16(&mut reader, 11)?;
                            if sync_extension_type == 0x0548 {
                                let ps = read_bits_u8(&mut reader, 1)?;
                                if ps != 0 {
                                    return Ok(Some(AacProfileInfo {
                                        object_type_indication: 0x40,
                                        audio_object_type: 29,
                                    }));
                                }
                            }
                        }
                    }

                    return Ok(Some(AacProfileInfo {
                        object_type_indication: 0x40,
                        audio_object_type: 5,
                    }));
                }
            }
        }
    }

    Ok(Some(AacProfileInfo {
        object_type_indication: 0x40,
        audio_object_type,
    }))
}

/// Finds sample indices whose AVC payload contains an IDR NAL unit.
pub fn find_idr_frames<R>(reader: &mut R, track: &TrackInfo) -> Result<Vec<usize>, ProbeError>
where
    R: Read + Seek,
{
    let Some(avc) = track.avc.as_ref() else {
        return Ok(Vec::new());
    };
    let length_size = u32::from(avc.length_size);

    let mut sample_index = 0usize;
    let mut indices = Vec::new();
    for chunk in &track.chunks {
        let end = sample_index.saturating_add(chunk.samples_per_chunk as usize);
        let mut data_offset = chunk.data_offset;
        while sample_index < end && sample_index < track.samples.len() {
            let sample = &track.samples[sample_index];
            if sample.size != 0 {
                let mut nal_offset = 0_u32;
                while nal_offset.saturating_add(length_size).saturating_add(1) <= sample.size {
                    reader.seek(SeekFrom::Start(data_offset + u64::from(nal_offset)))?;
                    let mut data = vec![0_u8; length_size as usize + 1];
                    reader.read_exact(&mut data)?;

                    let mut nal_length = 0_u32;
                    for byte in &data[..length_size as usize] {
                        nal_length = (nal_length << 8) | u32::from(*byte);
                    }
                    if data[length_size as usize] & 0x1f == 5 {
                        indices.push(sample_index);
                        break;
                    }

                    nal_offset = nal_offset
                        .saturating_add(length_size)
                        .saturating_add(nal_length);
                }
            }

            data_offset = data_offset.saturating_add(u64::from(sample.size));
            sample_index += 1;
        }
    }

    Ok(indices)
}

/// Returns the average bitrate implied by `samples` in the supplied timescale.
pub fn average_sample_bitrate(samples: &[SampleInfo], timescale: u32) -> u64 {
    let total_size = samples
        .iter()
        .map(|sample| u64::from(sample.size))
        .sum::<u64>();
    let total_duration = samples
        .iter()
        .map(|sample| u64::from(sample.time_delta))
        .sum::<u64>();
    if total_duration == 0 {
        return 0;
    }

    8 * total_size * u64::from(timescale) / total_duration
}

/// Returns the maximum rolling-window bitrate implied by `samples`.
pub fn max_sample_bitrate(samples: &[SampleInfo], timescale: u32, window_time_delta: u64) -> u64 {
    if window_time_delta == 0 || samples.is_empty() {
        return 0;
    }

    let mut max_bitrate = 0_u64;
    let mut size = 0_u64;
    let mut duration = 0_u64;
    let mut begin = 0usize;
    let mut end = 0usize;

    while end < samples.len() {
        while end < samples.len() {
            size += u64::from(samples[end].size);
            duration += u64::from(samples[end].time_delta);
            end += 1;
            if duration >= window_time_delta {
                break;
            }
        }

        if let Some(bitrate) = size
            .checked_mul(8)
            .and_then(|bits| bits.checked_mul(u64::from(timescale)))
            .and_then(|scaled_bits| scaled_bits.checked_div(duration))
        {
            max_bitrate = max_bitrate.max(bitrate);
        }

        while duration >= window_time_delta && begin < end {
            size -= u64::from(samples[begin].size);
            duration -= u64::from(samples[begin].time_delta);
            begin += 1;
        }
    }

    max_bitrate
}

/// Returns the average bitrate implied by `segments` for `track_id`.
pub fn average_segment_bitrate(segments: &[SegmentInfo], track_id: u32, timescale: u32) -> u64 {
    let mut total_size = 0_u64;
    let mut total_duration = 0_u64;
    for segment in segments {
        if segment.track_id == track_id {
            total_size += u64::from(segment.size);
            total_duration += u64::from(segment.duration);
        }
    }
    if total_duration == 0 {
        return 0;
    }

    8 * total_size * u64::from(timescale) / total_duration
}

/// Returns the maximum per-segment bitrate implied by `segments` for `track_id`.
pub fn max_segment_bitrate(segments: &[SegmentInfo], track_id: u32, timescale: u32) -> u64 {
    let mut max_bitrate = 0_u64;
    for segment in segments {
        if segment.track_id == track_id && segment.duration != 0 {
            let bitrate =
                8 * u64::from(segment.size) * u64::from(timescale) / u64::from(segment.duration);
            max_bitrate = max_bitrate.max(bitrate);
        }
    }
    max_bitrate
}

fn probe_trak<R>(reader: &mut R, parent: &BoxInfo) -> Result<TrackInfo, ProbeError>
where
    R: Read + Seek,
{
    let boxes = extract_boxes_with_payload(
        reader,
        Some(parent),
        &[
            BoxPath::from([TKHD]),
            BoxPath::from([EDTS, ELST]),
            BoxPath::from([MDIA, MDHD]),
            BoxPath::from([MDIA, MINF, STBL, STSD, AVC1]),
            BoxPath::from([MDIA, MINF, STBL, STSD, AVC1, AVCC]),
            BoxPath::from([MDIA, MINF, STBL, STSD, ENCV]),
            BoxPath::from([MDIA, MINF, STBL, STSD, ENCV, AVCC]),
            BoxPath::from([MDIA, MINF, STBL, STSD, MP4A]),
            BoxPath::from([MDIA, MINF, STBL, STSD, MP4A, ESDS]),
            BoxPath::from([MDIA, MINF, STBL, STSD, MP4A, WAVE, ESDS]),
            BoxPath::from([MDIA, MINF, STBL, STSD, ENCA]),
            BoxPath::from([MDIA, MINF, STBL, STSD, ENCA, ESDS]),
            BoxPath::from([MDIA, MINF, STBL, STCO]),
            BoxPath::from([MDIA, MINF, STBL, CO64]),
            BoxPath::from([MDIA, MINF, STBL, STTS]),
            BoxPath::from([MDIA, MINF, STBL, CTTS]),
            BoxPath::from([MDIA, MINF, STBL, STSC]),
            BoxPath::from([MDIA, MINF, STBL, STSZ]),
        ],
    )?;

    let mut track = TrackInfo::default();
    let mut tkhd = None;
    let mut mdhd = None;
    let mut visual_sample_entry = None;
    let mut avcc = None;
    let mut audio_sample_entry = None;
    let mut esds = None;
    let mut stco = None;
    let mut co64 = None;
    let mut stts = None;
    let mut ctts = None;
    let mut stsc = None;
    let mut stsz = None;

    for extracted in boxes {
        match extracted.info.box_type() {
            TKHD => {
                let payload = downcast_clone::<Tkhd>(&extracted)?;
                track.track_id = payload.track_id;
                tkhd = Some(payload);
            }
            ELST => {
                let elst = downcast_clone::<crate::boxes::iso14496_12::Elst>(&extracted)?;
                track.edit_list = elst
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(index, _)| EditListEntry {
                        media_time: elst.media_time(index),
                        segment_duration: elst.segment_duration(index),
                    })
                    .collect();
            }
            MDHD => {
                let payload = downcast_clone::<crate::boxes::iso14496_12::Mdhd>(&extracted)?;
                track.timescale = payload.timescale;
                track.duration = payload.duration();
                mdhd = Some(payload);
            }
            AVC1 => {
                track.codec = TrackCodec::Avc1;
                visual_sample_entry = Some(downcast_clone::<VisualSampleEntry>(&extracted)?);
            }
            AVCC => {
                avcc = Some(downcast_clone::<AVCDecoderConfiguration>(&extracted)?);
            }
            ENCV => {
                track.codec = TrackCodec::Avc1;
                track.encrypted = true;
                visual_sample_entry = Some(downcast_clone::<VisualSampleEntry>(&extracted)?);
            }
            MP4A => {
                track.codec = TrackCodec::Mp4a;
                audio_sample_entry = Some(downcast_clone::<AudioSampleEntry>(&extracted)?);
            }
            ENCA => {
                track.codec = TrackCodec::Mp4a;
                track.encrypted = true;
                audio_sample_entry = Some(downcast_clone::<AudioSampleEntry>(&extracted)?);
            }
            ESDS => {
                esds = Some(downcast_clone::<Esds>(&extracted)?);
            }
            STCO => {
                stco = Some(downcast_clone::<Stco>(&extracted)?);
            }
            CO64 => {
                co64 = Some(downcast_clone::<Co64>(&extracted)?);
            }
            STTS => {
                stts = Some(downcast_clone::<Stts>(&extracted)?);
            }
            CTTS => {
                ctts = Some(downcast_clone::<Ctts>(&extracted)?);
            }
            STSC => {
                stsc = Some(downcast_clone::<Stsc>(&extracted)?);
            }
            STSZ => {
                stsz = Some(downcast_clone::<Stsz>(&extracted)?);
            }
            _ => {}
        }
    }

    if tkhd.is_none() {
        return Err(ProbeError::MissingRequiredBox("tkhd"));
    }
    if mdhd.is_none() {
        return Err(ProbeError::MissingRequiredBox("mdhd"));
    }

    if let (Some(entry), Some(avcc)) = (visual_sample_entry.as_ref(), avcc.as_ref()) {
        track.avc = Some(AvcDecoderConfigInfo {
            configuration_version: avcc.configuration_version,
            profile: avcc.profile,
            profile_compatibility: avcc.profile_compatibility,
            level: avcc.level,
            length_size: u16::from(avcc.length_size_minus_one) + 1,
            width: entry.width,
            height: entry.height,
        });
    }

    if let (Some(entry), Some(esds)) = (audio_sample_entry.as_ref(), esds.as_ref())
        && let Some(profile) = detect_aac_profile(esds)?
    {
        track.mp4a = Some(Mp4aInfo {
            object_type_indication: profile.object_type_indication,
            audio_object_type: profile.audio_object_type,
            channel_count: entry.channel_count,
        });
    }

    let mut chunks = Vec::new();
    if let Some(stco) = stco.as_ref() {
        chunks.extend(stco.chunk_offset.iter().map(|offset| ChunkInfo {
            data_offset: *offset,
            samples_per_chunk: 0,
        }));
    } else if let Some(co64) = co64.as_ref() {
        chunks.extend(co64.chunk_offset.iter().map(|offset| ChunkInfo {
            data_offset: *offset,
            samples_per_chunk: 0,
        }));
    } else {
        return Err(ProbeError::MissingRequiredBox("stco/co64"));
    }

    let stts = stts.ok_or(ProbeError::MissingRequiredBox("stts"))?;
    let mut samples = Vec::new();
    for entry in &stts.entries {
        for _ in 0..entry.sample_count {
            samples.push(SampleInfo {
                time_delta: entry.sample_delta,
                ..SampleInfo::default()
            });
        }
    }

    let stsc = stsc.ok_or(ProbeError::MissingRequiredBox("stsc"))?;
    for (index, entry) in stsc.entries.iter().enumerate() {
        let mut end = chunks.len() as u32;
        if index + 1 != stsc.entries.len() {
            end = end.min(stsc.entries[index + 1].first_chunk.saturating_sub(1));
        }
        for chunk_index in entry.first_chunk.saturating_sub(1)..end {
            if let Some(chunk) = chunks.get_mut(chunk_index as usize) {
                chunk.samples_per_chunk = entry.samples_per_chunk;
            }
        }
    }

    if let Some(ctts) = ctts.as_ref() {
        let mut sample_index = 0usize;
        for (entry_index, entry) in ctts.entries.iter().enumerate() {
            for _ in 0..entry.sample_count {
                if sample_index >= samples.len() {
                    break;
                }
                samples[sample_index].composition_time_offset = ctts.sample_offset(entry_index);
                sample_index += 1;
            }
        }
    }

    if let Some(stsz) = stsz.as_ref() {
        if stsz.sample_size != 0 {
            for sample in &mut samples {
                sample.size = stsz.sample_size;
            }
        } else {
            for (sample, entry_size) in samples.iter_mut().zip(stsz.entry_size.iter()) {
                sample.size =
                    (*entry_size)
                        .try_into()
                        .map_err(|_| ProbeError::NumericOverflow {
                            field_name: "stsz entry size",
                        })?;
            }
        }
    }

    track.chunks = chunks;
    track.samples = samples;
    Ok(track)
}

fn probe_moof<R>(reader: &mut R, parent: &BoxInfo) -> Result<SegmentInfo, ProbeError>
where
    R: Read + Seek,
{
    let boxes = extract_boxes_with_payload(
        reader,
        Some(parent),
        &[
            BoxPath::from([TRAF, TFHD]),
            BoxPath::from([TRAF, TFDT]),
            BoxPath::from([TRAF, TRUN]),
        ],
    )?;

    let mut tfhd = None;
    let mut tfdt = None;
    let mut trun = None;
    for extracted in boxes {
        match extracted.info.box_type() {
            TFHD => tfhd = Some(downcast_clone::<Tfhd>(&extracted)?),
            TFDT => tfdt = Some(downcast_clone::<Tfdt>(&extracted)?),
            TRUN => trun = Some(downcast_clone::<Trun>(&extracted)?),
            _ => {}
        }
    }

    let tfhd = tfhd.ok_or(ProbeError::MissingRequiredBox("tfhd"))?;
    let mut segment = SegmentInfo {
        track_id: tfhd.track_id,
        moof_offset: parent.offset(),
        default_sample_duration: tfhd.default_sample_duration,
        ..SegmentInfo::default()
    };

    if let Some(tfdt) = tfdt.as_ref() {
        segment.base_media_decode_time = tfdt.base_media_decode_time();
    }

    if let Some(trun) = trun.as_ref() {
        segment.sample_count = trun.sample_count;

        if trun.flags() & crate::boxes::iso14496_12::TRUN_SAMPLE_DURATION_PRESENT != 0 {
            segment.duration = trun
                .entries
                .iter()
                .map(|entry| entry.sample_duration)
                .sum::<u32>();
        } else {
            segment.duration = tfhd
                .default_sample_duration
                .saturating_mul(segment.sample_count);
        }

        if trun.flags() & crate::boxes::iso14496_12::TRUN_SAMPLE_SIZE_PRESENT != 0 {
            segment.size = trun
                .entries
                .iter()
                .map(|entry| entry.sample_size)
                .sum::<u32>();
        } else {
            segment.size = tfhd
                .default_sample_size
                .saturating_mul(segment.sample_count);
        }

        let mut duration = 0_u32;
        let mut min_offset = None;
        for (index, entry) in trun.entries.iter().enumerate() {
            let offset = i64::from(duration) + trun.sample_composition_time_offset(index);
            min_offset = Some(min_offset.map_or(offset, |current: i64| current.min(offset)));
            duration = duration.saturating_add(
                if trun.flags() & crate::boxes::iso14496_12::TRUN_SAMPLE_DURATION_PRESENT != 0 {
                    entry.sample_duration
                } else {
                    tfhd.default_sample_duration
                },
            );
        }
        if let Some(offset) = min_offset {
            segment.composition_time_offset =
                offset.try_into().map_err(|_| ProbeError::NumericOverflow {
                    field_name: "segment composition time offset",
                })?;
        }
    }

    Ok(segment)
}

fn read_payload_as<R, B>(reader: &mut R, info: &BoxInfo) -> Result<B, ProbeError>
where
    R: Read + Seek,
    B: CodecBox + Default,
{
    info.seek_to_payload(reader)?;
    let mut decoded = B::default();
    unmarshal(reader, info.payload_size()?, &mut decoded, None)?;
    Ok(decoded)
}

fn downcast_clone<T>(extracted: &ExtractedBox) -> Result<T, ProbeError>
where
    T: Clone + 'static,
{
    extracted
        .payload
        .as_ref()
        .as_any()
        .downcast_ref::<T>()
        .cloned()
        .ok_or(ProbeError::UnexpectedPayloadType {
            box_type: extracted.info.box_type(),
        })
}

fn get_audio_object_type<R>(reader: &mut BitReader<R>) -> Result<(u8, usize), ProbeError>
where
    R: Read,
{
    let audio_object_type = read_bits_u8(reader, 5)?;
    if audio_object_type != 0x1f {
        return Ok((audio_object_type, 5));
    }

    let extended = read_bits_u8(reader, 6)?;
    Ok((extended.saturating_add(32), 11))
}

fn read_bits_u8<R>(reader: &mut BitReader<R>, width: usize) -> Result<u8, ProbeError>
where
    R: Read,
{
    let bits = reader.read_bits(width).map_err(ProbeError::Io)?;
    let mut value = 0_u16;
    for byte in bits {
        value = (value << 8) | u16::from(byte);
    }
    u8::try_from(value).map_err(|_| ProbeError::NumericOverflow {
        field_name: "bitfield read",
    })
}

fn read_bits_u16<R>(reader: &mut BitReader<R>, width: usize) -> Result<u16, ProbeError>
where
    R: Read,
{
    let bits = reader.read_bits(width).map_err(ProbeError::Io)?;
    let mut value = 0_u32;
    for byte in bits {
        value = (value << 8) | u32::from(byte);
    }
    u16::try_from(value).map_err(|_| ProbeError::NumericOverflow {
        field_name: "bitfield read",
    })
}

fn read_bits_u32<R>(reader: &mut BitReader<R>, width: usize) -> Result<u32, ProbeError>
where
    R: Read,
{
    let bits = reader.read_bits(width).map_err(ProbeError::Io)?;
    let mut value = 0_u64;
    for byte in bits {
        value = (value << 8) | u64::from(byte);
    }
    u32::try_from(value).map_err(|_| ProbeError::NumericOverflow {
        field_name: "bitfield read",
    })
}

/// Errors raised while probing files or derived codec summaries.
#[derive(Debug)]
pub enum ProbeError {
    Io(io::Error),
    Header(HeaderError),
    Codec(CodecError),
    Extract(ExtractError),
    MissingRequiredBox(&'static str),
    MissingDescriptor(&'static str),
    UnexpectedPayloadType { box_type: FourCc },
    NumericOverflow { field_name: &'static str },
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => error.fmt(f),
            Self::Codec(error) => error.fmt(f),
            Self::Extract(error) => error.fmt(f),
            Self::MissingRequiredBox(name) => write!(f, "{name} box not found"),
            Self::MissingDescriptor(name) => write!(f, "{name} not found"),
            Self::UnexpectedPayloadType { box_type } => {
                write!(f, "unexpected payload type for {box_type}")
            }
            Self::NumericOverflow { field_name } => {
                write!(f, "numeric value does not fit while reading {field_name}")
            }
        }
    }
}

impl Error for ProbeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::Extract(error) => Some(error),
            Self::MissingRequiredBox(..)
            | Self::MissingDescriptor(..)
            | Self::UnexpectedPayloadType { .. }
            | Self::NumericOverflow { .. } => None,
        }
    }
}

impl From<io::Error> for ProbeError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<HeaderError> for ProbeError {
    fn from(value: HeaderError) -> Self {
        Self::Header(value)
    }
}

impl From<CodecError> for ProbeError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}

impl From<ExtractError> for ProbeError {
    fn from(value: ExtractError) -> Self {
        Self::Extract(value)
    }
}
