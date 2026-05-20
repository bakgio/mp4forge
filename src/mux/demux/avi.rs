use std::fs::File;
use std::io::Cursor;
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

use super::super::MuxError;
use super::super::MuxTrackKind;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    CandidateSample, CompositeTrackCandidate, SegmentedMuxSourceSegment, SegmentedMuxSourceSpec,
    StagedSample, TrackCandidate, build_btrt_from_sample_sizes,
    build_generic_audio_sample_entry_box, direct_ingest_handler_name, direct_ingest_mux_policy,
    read_exact_at_sync, with_force_empty_sync_sample_table,
};
#[cfg(feature = "async")]
use super::aac::scan_adts_segmented_async;
use super::aac::{build_aac_lc_sample_entry_box, scan_adts_segmented_sync};
#[cfg(feature = "async")]
use super::ac3::scan_ac3_segmented_async;
use super::ac3::scan_ac3_segmented_sync;
use super::h263::{build_avi_h263_sample_entry_box, parse_h263_picture_bytes};
#[cfg(feature = "async")]
use super::h264::stage_annex_b_h264_segmented_async;
use super::h264::{
    build_h264_sample_entry_from_avc_config_with_options, retune_carried_h264_sample_entry_box,
    stage_annex_b_h264_segmented_sync,
};
use super::jpeg::{build_avi_jpeg_sample_entry_box, parse_jpeg_bytes};
#[cfg(feature = "async")]
use super::mp3::scan_mp3_segmented_async;
use super::mp3::scan_mp3_segmented_sync;
#[cfg(feature = "async")]
use super::mp4v::scan_mp4v_segmented_async;
use super::mp4v::{build_direct_mp4v_sample_entry_box, scan_mp4v_segmented_sync};
use super::pcm::build_pcm_sample_entry_box;
use super::png::{build_avi_png_sample_entry_box, parse_png_bytes};
use crate::FourCc;
use crate::boxes::iso14496_12::{AVCDecoderConfiguration, SampleEntry, VisualSampleEntry};
use crate::codec::unmarshal;

const RIFF: &[u8; 4] = b"RIFF";
const LIST: FourCc = FourCc::from_bytes(*b"LIST");
const AVI_FORM: FourCc = FourCc::from_bytes(*b"AVI ");
const HDRL: FourCc = FourCc::from_bytes(*b"hdrl");
const STRL: FourCc = FourCc::from_bytes(*b"strl");
const STRH: FourCc = FourCc::from_bytes(*b"strh");
const STRF: FourCc = FourCc::from_bytes(*b"strf");
const MOVI: FourCc = FourCc::from_bytes(*b"movi");
const RECL: FourCc = FourCc::from_bytes(*b"rec ");
const AUDS: FourCc = FourCc::from_bytes(*b"auds");
const VIDS: FourCc = FourCc::from_bytes(*b"vids");
const WAVE_FORMAT_PCM: u16 = 0x0001;
const WAVE_FORMAT_ADPCM: u16 = 0x0002;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
const IBM_FORMAT_CVSD: u16 = 0x0005;
const WAVE_FORMAT_ALAW: u16 = 0x0006;
const WAVE_FORMAT_MULAW: u16 = 0x0007;
const WAVE_FORMAT_OKI_ADPCM: u16 = 0x0010;
const WAVE_FORMAT_DVI_ADPCM: u16 = 0x0011;
const WAVE_FORMAT_DIGISTD: u16 = 0x0015;
const WAVE_FORMAT_YAMAHA_ADPCM: u16 = 0x0020;
const WAVE_FORMAT_DSP_TRUESPEECH: u16 = 0x0022;
const WAVE_FORMAT_GSM610: u16 = 0x0031;
const WAVE_FORMAT_MP3: u16 = 0x0055;
const WAVE_FORMAT_AAC_ADTS: u16 = 0x706D;
const IBM_FORMAT_MULAW: u16 = 0x0101;
const IBM_FORMAT_ALAW: u16 = 0x0102;
const IBM_FORMAT_ADPCM: u16 = 0x0103;
const WAVE_FORMAT_AAC: u16 = 0x00FF;
const WAVE_FORMAT_AC3: u16 = 0x2000;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const SAMPLE_ENTRY_IPCM: FourCc = FourCc::from_bytes(*b"ipcm");
const SAMPLE_ENTRY_FPCM: FourCc = FourCc::from_bytes(*b"fpcm");
const SAMPLE_ENTRY_ALAW: FourCc = FourCc::from_bytes(*b"alaw");
const SAMPLE_ENTRY_MLAW: FourCc = FourCc::from_bytes(*b"MLAW");
const SAMPLE_ENTRY_MS_ADPCM: FourCc = FourCc::from_bytes([0x6D, 0x73, 0x00, 0x02]);
const SAMPLE_ENTRY_IMA_ADPCM: FourCc = FourCc::from_bytes([0x6D, 0x73, 0x00, 0x11]);
const SAMPLE_ENTRY_IBM_CVSD: FourCc = FourCc::from_bytes(*b"CSVD");
const SAMPLE_ENTRY_OKI_ADPCM: FourCc = FourCc::from_bytes(*b"OPCM");
const SAMPLE_ENTRY_DIGISTD: FourCc = FourCc::from_bytes(*b"DSTD");
const SAMPLE_ENTRY_YAMAHA_ADPCM: FourCc = FourCc::from_bytes(*b"YPCM");
const SAMPLE_ENTRY_DSP_TRUESPEECH: FourCc = FourCc::from_bytes(*b"TSPE");
const SAMPLE_ENTRY_GSM610: FourCc = FourCc::from_bytes(*b"G610");
const SAMPLE_ENTRY_IBM_ADPCM: FourCc = FourCc::from_bytes(*b"IPCM");
const SAMPLE_ENTRY_DIV3: FourCc = FourCc::from_bytes(*b"DIV3");
const SAMPLE_ENTRY_DIV4: FourCc = FourCc::from_bytes(*b"DIV4");
const SAMPLE_ENTRY_UNCV: FourCc = FourCc::from_bytes(*b"uncv");
const AVC1: FourCc = FourCc::from_bytes(*b"AVC1");
const CMPD: FourCc = FourCc::from_bytes(*b"cmpd");
const UNCC: FourCc = FourCc::from_bytes(*b"uncC");
const AVI_RAW_VIDEO_COMPRESSOR_NAME: &[u8] = b"RawVideo";
const AVI_MS_MPEG4_V3_COMPRESSOR_NAME: &[u8] = b"MS-MPEG4 V3";
const AVI_GENERIC_UNSUPPORTED_COMPRESSOR_NAME: &[u8] = b"Codec Not Supported";
const SUPPORTED_AVI_AUDIO_TAGS: &str = "PCM, extensible PCM, IEEE float PCM, extensible IEEE float PCM, A-law, extensible A-law, IBM A-law, mu-law, extensible mu-law, IBM mu-law, Microsoft ADPCM, IMA ADPCM, IBM CVSD, OKI ADPCM, DIGISTD, Yamaha ADPCM, DSP TrueSpeech, GSM 610, IBM ADPCM, AAC ADTS, AAC, MP3, and AC-3";
const KSDATAFORMAT_SUBTYPE_PCM: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: [u8; 16] = [
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];
const KSDATAFORMAT_SUBTYPE_ALAW: [u8; 16] = [
    0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];
const KSDATAFORMAT_SUBTYPE_MULAW: [u8; 16] = [
    0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

pub(in crate::mux) struct ScannedAviSource {
    pub(in crate::mux) tracks: Vec<TrackCandidate>,
    pub(in crate::mux) composite_tracks: Vec<CompositeTrackCandidate>,
}

pub(in crate::mux) fn scan_avi_source_sync(
    path: &Path,
    spec: &str,
    source_index: usize,
) -> Result<ScannedAviSource, MuxError> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    parse_avi_source_sync(path, &mut file, file_size, spec, source_index)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_avi_source_async(
    path: &Path,
    spec: &str,
    source_index: usize,
) -> Result<ScannedAviSource, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let file_size = file.metadata().await?.len();
    parse_avi_source_async(path, &mut file, file_size, spec, source_index).await
}

#[derive(Clone)]
struct AviTrackDescriptor {
    stream_index: u32,
    stream_type: FourCc,
    timing_scale: u32,
    timing_rate: u32,
    audio_format: Option<AviAudioFormat>,
    video_format: Option<AviVideoFormat>,
}

#[derive(Clone, Copy)]
struct AviAudioFormat {
    format_tag: u16,
    channel_count: u16,
    sample_rate: u32,
    block_align: u16,
    bits_per_sample: u16,
}

#[derive(Clone)]
struct AviVideoFormat {
    width: u16,
    height: u16,
    codec: AviVideoCodec,
    decoder_specific_info: Vec<u8>,
}

#[derive(Clone, Copy)]
enum AviVideoCodec {
    Mp4v,
    H264AnnexB,
    H264Avc1,
    H263,
    Jpeg,
    Png,
    MsMpeg4V3(FourCc),
    RawBgr,
    GenericPassthrough(FourCc),
}

#[derive(Clone, Copy)]
struct AviChunkSpan {
    data_offset: u64,
    data_size: u32,
}

#[derive(Clone, Copy)]
enum AviAdpcmKind {
    Microsoft,
    ImaDvi,
}

fn parse_avi_source_sync(
    path: &Path,
    file: &mut File,
    file_size: u64,
    spec: &str,
    source_index: usize,
) -> Result<ScannedAviSource, MuxError> {
    validate_avi_header_sync(file, file_size, spec)?;

    let mut track_descriptors = Vec::new();
    let mut movi_range = None::<(u64, u64)>;
    let mut offset = 12_u64;
    while offset < file_size {
        let (chunk_type, chunk_size, chunk_payload_offset, chunk_end) =
            read_riff_chunk_header_sync(file, file_size, offset, spec)?;
        if chunk_type == LIST {
            if chunk_size < 4 {
                return Err(invalid_avi(
                    spec,
                    "AVI `LIST` chunk was truncated before its list type",
                ));
            }
            let list_type = read_fourcc_sync(
                file,
                chunk_payload_offset,
                spec,
                "AVI list type is truncated",
            )?;
            if list_type == HDRL {
                parse_hdrl_list_sync(
                    file,
                    chunk_payload_offset + 4,
                    chunk_end,
                    spec,
                    &mut track_descriptors,
                )?;
            } else if list_type == MOVI {
                movi_range = Some((chunk_payload_offset + 4, chunk_end));
            }
        }
        offset = next_riff_offset(chunk_end);
    }

    let (movi_start, movi_end) = movi_range
        .ok_or_else(|| invalid_avi(spec, "AVI input did not contain one `LIST` `movi` payload"))?;
    if track_descriptors.is_empty() {
        return Err(invalid_avi(
            spec,
            "AVI input did not contain any stream descriptors under `LIST` `hdrl`",
        ));
    }

    let mut track_chunks = vec![Vec::new(); track_descriptors.len()];
    parse_movi_chunks_sync(
        file,
        movi_start,
        movi_end,
        spec,
        track_descriptors.len(),
        &mut track_chunks,
    )?;
    finalize_avi_tracks_sync(
        file,
        path,
        spec,
        source_index,
        track_descriptors,
        track_chunks,
    )
}

#[cfg(feature = "async")]
async fn parse_avi_source_async(
    path: &Path,
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
    source_index: usize,
) -> Result<ScannedAviSource, MuxError> {
    validate_avi_header_async(file, file_size, spec).await?;

    let mut track_descriptors = Vec::new();
    let mut movi_range = None::<(u64, u64)>;
    let mut offset = 12_u64;
    while offset < file_size {
        let (chunk_type, chunk_size, chunk_payload_offset, chunk_end) =
            read_riff_chunk_header_async(file, file_size, offset, spec).await?;
        if chunk_type == LIST {
            if chunk_size < 4 {
                return Err(invalid_avi(
                    spec,
                    "AVI `LIST` chunk was truncated before its list type",
                ));
            }
            let list_type = read_fourcc_async(
                file,
                chunk_payload_offset,
                spec,
                "AVI list type is truncated",
            )
            .await?;
            if list_type == HDRL {
                parse_hdrl_list_async(
                    file,
                    chunk_payload_offset + 4,
                    chunk_end,
                    spec,
                    &mut track_descriptors,
                )
                .await?;
            } else if list_type == MOVI {
                movi_range = Some((chunk_payload_offset + 4, chunk_end));
            }
        }
        offset = next_riff_offset(chunk_end);
    }

    let (movi_start, movi_end) = movi_range
        .ok_or_else(|| invalid_avi(spec, "AVI input did not contain one `LIST` `movi` payload"))?;
    if track_descriptors.is_empty() {
        return Err(invalid_avi(
            spec,
            "AVI input did not contain any stream descriptors under `LIST` `hdrl`",
        ));
    }

    let mut track_chunks = vec![Vec::new(); track_descriptors.len()];
    parse_movi_chunks_async(
        file,
        movi_start,
        movi_end,
        spec,
        track_descriptors.len(),
        &mut track_chunks,
    )
    .await?;
    finalize_avi_tracks_async(
        file,
        path,
        spec,
        source_index,
        track_descriptors,
        track_chunks,
    )
    .await
}

fn finalize_avi_tracks_sync(
    file: &mut File,
    path: &Path,
    spec: &str,
    source_index: usize,
    track_descriptors: Vec<AviTrackDescriptor>,
    track_chunks: Vec<Vec<AviChunkSpan>>,
) -> Result<ScannedAviSource, MuxError> {
    let mut tracks = Vec::new();
    let mut composite_tracks = Vec::new();
    for (descriptor, chunks) in track_descriptors.into_iter().zip(track_chunks) {
        if chunks.is_empty() {
            continue;
        }
        match descriptor.stream_type {
            AUDS => {
                let audio_format = descriptor.audio_format.ok_or_else(|| {
                    invalid_avi(
                        spec,
                        &format!(
                            "AVI audio stream {} did not carry a parseable WAVEFORMAT payload",
                            descriptor.stream_index
                        ),
                    )
                })?;
                match audio_format.format_tag {
                    WAVE_FORMAT_PCM => tracks.push(finalize_avi_pcm_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        false,
                    )?),
                    WAVE_FORMAT_IEEE_FLOAT => tracks.push(finalize_avi_pcm_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        true,
                    )?),
                    WAVE_FORMAT_ALAW | IBM_FORMAT_ALAW => {
                        tracks.push(finalize_avi_companded_track(
                            spec,
                            source_index,
                            descriptor.stream_index,
                            audio_format,
                            chunks,
                            SAMPLE_ENTRY_ALAW,
                            "alaw",
                        )?)
                    }
                    WAVE_FORMAT_MULAW | IBM_FORMAT_MULAW => {
                        tracks.push(finalize_avi_companded_track(
                            spec,
                            source_index,
                            descriptor.stream_index,
                            audio_format,
                            chunks,
                            SAMPLE_ENTRY_MLAW,
                            "ulaw",
                        )?)
                    }
                    WAVE_FORMAT_ADPCM => tracks.push(finalize_avi_adpcm_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        AviAdpcmKind::Microsoft,
                    )?),
                    WAVE_FORMAT_DVI_ADPCM => tracks.push(finalize_avi_adpcm_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        AviAdpcmKind::ImaDvi,
                    )?),
                    IBM_FORMAT_CVSD => tracks.push(finalize_avi_generic_coded_audio_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        SAMPLE_ENTRY_IBM_CVSD,
                        "ibm-cvsd",
                    )?),
                    WAVE_FORMAT_OKI_ADPCM => tracks.push(finalize_avi_generic_coded_audio_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        SAMPLE_ENTRY_OKI_ADPCM,
                        "oki-adpcm",
                    )?),
                    WAVE_FORMAT_DIGISTD => tracks.push(finalize_avi_generic_coded_audio_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        SAMPLE_ENTRY_DIGISTD,
                        "digistd",
                    )?),
                    WAVE_FORMAT_YAMAHA_ADPCM => {
                        tracks.push(finalize_avi_generic_coded_audio_track(
                            spec,
                            source_index,
                            descriptor.stream_index,
                            audio_format,
                            chunks,
                            SAMPLE_ENTRY_YAMAHA_ADPCM,
                            "yamaha-adpcm",
                        )?)
                    }
                    WAVE_FORMAT_DSP_TRUESPEECH => {
                        tracks.push(finalize_avi_generic_coded_audio_track(
                            spec,
                            source_index,
                            descriptor.stream_index,
                            audio_format,
                            chunks,
                            SAMPLE_ENTRY_DSP_TRUESPEECH,
                            "truespeech",
                        )?)
                    }
                    WAVE_FORMAT_GSM610 => tracks.push(finalize_avi_generic_coded_audio_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        SAMPLE_ENTRY_GSM610,
                        "gsm610",
                    )?),
                    IBM_FORMAT_ADPCM => tracks.push(finalize_avi_generic_coded_audio_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        SAMPLE_ENTRY_IBM_ADPCM,
                        "ibm-adpcm",
                    )?),
                    WAVE_FORMAT_AAC_ADTS => composite_tracks.push(finalize_avi_adts_track_sync(
                        file, path, spec, descriptor, chunks,
                    )?),
                    WAVE_FORMAT_AAC => tracks.push(finalize_avi_raw_aac_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                    )?),
                    WAVE_FORMAT_MP3 => composite_tracks.push(finalize_avi_mp3_track_sync(
                        file, path, spec, descriptor, chunks,
                    )?),
                    WAVE_FORMAT_AC3 => composite_tracks.push(finalize_avi_ac3_track_sync(
                        file, path, spec, descriptor, chunks,
                    )?),
                    _ => {
                        return Err(invalid_avi(
                            spec,
                            &unsupported_avi_audio_format_tag_message(
                                descriptor.stream_index,
                                audio_format,
                            ),
                        ));
                    }
                }
            }
            VIDS => {
                let video_format = descriptor.video_format.clone().ok_or_else(|| {
                    invalid_avi(
                        spec,
                        &format!(
                            "AVI video stream {} did not carry a parseable BITMAPINFO payload",
                            descriptor.stream_index
                        ),
                    )
                })?;
                match video_format.codec {
                    AviVideoCodec::Mp4v => tracks.push(finalize_avi_mp4v_track_sync(
                        file,
                        path,
                        spec,
                        source_index,
                        descriptor,
                        video_format,
                        chunks,
                    )?),
                    AviVideoCodec::H264AnnexB => {
                        composite_tracks.push(finalize_avi_h264_track_sync(
                            file,
                            path,
                            spec,
                            descriptor,
                            video_format,
                            chunks,
                        )?)
                    }
                    AviVideoCodec::H264Avc1 => tracks.push(finalize_avi_h264_avc1_track_sync(
                        file,
                        spec,
                        source_index,
                        descriptor,
                        video_format,
                        chunks,
                    )?),
                    AviVideoCodec::H263 => tracks.push(finalize_avi_h263_track_sync(
                        file,
                        spec,
                        source_index,
                        descriptor,
                        video_format,
                        chunks,
                    )?),
                    AviVideoCodec::Jpeg => tracks.push(finalize_avi_jpeg_track_sync(
                        file,
                        spec,
                        source_index,
                        descriptor,
                        video_format,
                        chunks,
                    )?),
                    AviVideoCodec::Png => tracks.push(finalize_avi_png_track_sync(
                        file,
                        spec,
                        source_index,
                        descriptor,
                        video_format,
                        chunks,
                    )?),
                    AviVideoCodec::MsMpeg4V3(sample_entry_type) => {
                        tracks.push(finalize_avi_generic_visual_track(
                            source_index,
                            descriptor,
                            video_format,
                            chunks,
                            sample_entry_type,
                            AVI_MS_MPEG4_V3_COMPRESSOR_NAME.to_vec(),
                        )?)
                    }
                    AviVideoCodec::RawBgr => tracks.push(finalize_avi_uncv_bgr_track(
                        source_index,
                        descriptor,
                        video_format,
                        chunks,
                    )?),
                    AviVideoCodec::GenericPassthrough(sample_entry_type) => {
                        tracks.push(finalize_avi_generic_visual_track(
                            source_index,
                            descriptor,
                            video_format,
                            chunks,
                            sample_entry_type,
                            AVI_GENERIC_UNSUPPORTED_COMPRESSOR_NAME.to_vec(),
                        )?)
                    }
                }
            }
            other => {
                return Err(invalid_avi(
                    spec,
                    &format!(
                        "AVI stream {} uses unsupported stream type `{other}` on the native direct-ingest path",
                        descriptor.stream_index
                    ),
                ));
            }
        };
    }
    if tracks.is_empty() && composite_tracks.is_empty() {
        return Err(invalid_avi(
            spec,
            "AVI input did not contain any supported stream chunks under `LIST` `movi`",
        ));
    }
    Ok(ScannedAviSource {
        tracks,
        composite_tracks,
    })
}

#[cfg(feature = "async")]
async fn finalize_avi_tracks_async(
    file: &mut TokioFile,
    path: &Path,
    spec: &str,
    source_index: usize,
    track_descriptors: Vec<AviTrackDescriptor>,
    track_chunks: Vec<Vec<AviChunkSpan>>,
) -> Result<ScannedAviSource, MuxError> {
    let mut tracks = Vec::new();
    let mut composite_tracks = Vec::new();
    for (descriptor, chunks) in track_descriptors.into_iter().zip(track_chunks) {
        if chunks.is_empty() {
            continue;
        }
        match descriptor.stream_type {
            AUDS => {
                let audio_format = descriptor.audio_format.ok_or_else(|| {
                    invalid_avi(
                        spec,
                        &format!(
                            "AVI audio stream {} did not carry a parseable WAVEFORMAT payload",
                            descriptor.stream_index
                        ),
                    )
                })?;
                match audio_format.format_tag {
                    WAVE_FORMAT_PCM => tracks.push(finalize_avi_pcm_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        false,
                    )?),
                    WAVE_FORMAT_IEEE_FLOAT => tracks.push(finalize_avi_pcm_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        true,
                    )?),
                    WAVE_FORMAT_ALAW | IBM_FORMAT_ALAW => {
                        tracks.push(finalize_avi_companded_track(
                            spec,
                            source_index,
                            descriptor.stream_index,
                            audio_format,
                            chunks,
                            SAMPLE_ENTRY_ALAW,
                            "alaw",
                        )?)
                    }
                    WAVE_FORMAT_MULAW | IBM_FORMAT_MULAW => {
                        tracks.push(finalize_avi_companded_track(
                            spec,
                            source_index,
                            descriptor.stream_index,
                            audio_format,
                            chunks,
                            SAMPLE_ENTRY_MLAW,
                            "ulaw",
                        )?)
                    }
                    WAVE_FORMAT_ADPCM => tracks.push(finalize_avi_adpcm_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        AviAdpcmKind::Microsoft,
                    )?),
                    WAVE_FORMAT_DVI_ADPCM => tracks.push(finalize_avi_adpcm_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        AviAdpcmKind::ImaDvi,
                    )?),
                    IBM_FORMAT_CVSD => tracks.push(finalize_avi_generic_coded_audio_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        SAMPLE_ENTRY_IBM_CVSD,
                        "ibm-cvsd",
                    )?),
                    WAVE_FORMAT_OKI_ADPCM => tracks.push(finalize_avi_generic_coded_audio_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        SAMPLE_ENTRY_OKI_ADPCM,
                        "oki-adpcm",
                    )?),
                    WAVE_FORMAT_DIGISTD => tracks.push(finalize_avi_generic_coded_audio_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        SAMPLE_ENTRY_DIGISTD,
                        "digistd",
                    )?),
                    WAVE_FORMAT_YAMAHA_ADPCM => {
                        tracks.push(finalize_avi_generic_coded_audio_track(
                            spec,
                            source_index,
                            descriptor.stream_index,
                            audio_format,
                            chunks,
                            SAMPLE_ENTRY_YAMAHA_ADPCM,
                            "yamaha-adpcm",
                        )?)
                    }
                    WAVE_FORMAT_DSP_TRUESPEECH => {
                        tracks.push(finalize_avi_generic_coded_audio_track(
                            spec,
                            source_index,
                            descriptor.stream_index,
                            audio_format,
                            chunks,
                            SAMPLE_ENTRY_DSP_TRUESPEECH,
                            "truespeech",
                        )?)
                    }
                    WAVE_FORMAT_GSM610 => tracks.push(finalize_avi_generic_coded_audio_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        SAMPLE_ENTRY_GSM610,
                        "gsm610",
                    )?),
                    IBM_FORMAT_ADPCM => tracks.push(finalize_avi_generic_coded_audio_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                        SAMPLE_ENTRY_IBM_ADPCM,
                        "ibm-adpcm",
                    )?),
                    WAVE_FORMAT_AAC_ADTS => composite_tracks.push(
                        finalize_avi_adts_track_async(file, path, spec, descriptor, chunks).await?,
                    ),
                    WAVE_FORMAT_AAC => tracks.push(finalize_avi_raw_aac_track(
                        spec,
                        source_index,
                        descriptor.stream_index,
                        audio_format,
                        chunks,
                    )?),
                    WAVE_FORMAT_MP3 => composite_tracks.push(
                        finalize_avi_mp3_track_async(file, path, spec, descriptor, chunks).await?,
                    ),
                    WAVE_FORMAT_AC3 => composite_tracks.push(
                        finalize_avi_ac3_track_async(file, path, spec, descriptor, chunks).await?,
                    ),
                    _ => {
                        return Err(invalid_avi(
                            spec,
                            &unsupported_avi_audio_format_tag_message(
                                descriptor.stream_index,
                                audio_format,
                            ),
                        ));
                    }
                }
            }
            VIDS => {
                let video_format = descriptor.video_format.clone().ok_or_else(|| {
                    invalid_avi(
                        spec,
                        &format!(
                            "AVI video stream {} did not carry a parseable BITMAPINFO payload",
                            descriptor.stream_index
                        ),
                    )
                })?;
                match video_format.codec {
                    AviVideoCodec::Mp4v => tracks.push(
                        finalize_avi_mp4v_track_async(
                            file,
                            path,
                            spec,
                            source_index,
                            descriptor,
                            video_format,
                            chunks,
                        )
                        .await?,
                    ),
                    AviVideoCodec::H264AnnexB => composite_tracks.push(
                        finalize_avi_h264_track_async(
                            file,
                            path,
                            spec,
                            descriptor,
                            video_format,
                            chunks,
                        )
                        .await?,
                    ),
                    AviVideoCodec::H264Avc1 => tracks.push(
                        finalize_avi_h264_avc1_track_async(
                            file,
                            spec,
                            source_index,
                            descriptor,
                            video_format,
                            chunks,
                        )
                        .await?,
                    ),
                    AviVideoCodec::H263 => tracks.push(
                        finalize_avi_h263_track_async(
                            file,
                            spec,
                            source_index,
                            descriptor,
                            video_format,
                            chunks,
                        )
                        .await?,
                    ),
                    AviVideoCodec::Jpeg => tracks.push(
                        finalize_avi_jpeg_track_async(
                            file,
                            spec,
                            source_index,
                            descriptor,
                            video_format,
                            chunks,
                        )
                        .await?,
                    ),
                    AviVideoCodec::Png => tracks.push(
                        finalize_avi_png_track_async(
                            file,
                            spec,
                            source_index,
                            descriptor,
                            video_format,
                            chunks,
                        )
                        .await?,
                    ),
                    AviVideoCodec::MsMpeg4V3(sample_entry_type) => {
                        tracks.push(finalize_avi_generic_visual_track(
                            source_index,
                            descriptor,
                            video_format,
                            chunks,
                            sample_entry_type,
                            AVI_MS_MPEG4_V3_COMPRESSOR_NAME.to_vec(),
                        )?)
                    }
                    AviVideoCodec::RawBgr => tracks.push(finalize_avi_uncv_bgr_track(
                        source_index,
                        descriptor,
                        video_format,
                        chunks,
                    )?),
                    AviVideoCodec::GenericPassthrough(sample_entry_type) => {
                        tracks.push(finalize_avi_generic_visual_track(
                            source_index,
                            descriptor,
                            video_format,
                            chunks,
                            sample_entry_type,
                            AVI_GENERIC_UNSUPPORTED_COMPRESSOR_NAME.to_vec(),
                        )?)
                    }
                }
            }
            other => {
                return Err(invalid_avi(
                    spec,
                    &format!(
                        "AVI stream {} uses unsupported stream type `{other}` on the native direct-ingest path",
                        descriptor.stream_index
                    ),
                ));
            }
        };
    }
    if tracks.is_empty() && composite_tracks.is_empty() {
        return Err(invalid_avi(
            spec,
            "AVI input did not contain any supported stream chunks under `LIST` `movi`",
        ));
    }
    Ok(ScannedAviSource {
        tracks,
        composite_tracks,
    })
}

fn finalize_avi_pcm_track(
    spec: &str,
    source_index: usize,
    stream_index: u32,
    audio_format: AviAudioFormat,
    chunks: Vec<AviChunkSpan>,
    floating_point: bool,
) -> Result<TrackCandidate, MuxError> {
    if audio_format.block_align == 0 {
        return Err(invalid_avi(
            spec,
            &format!("AVI PCM stream {stream_index} declared a zero block align"),
        ));
    }
    let sample_entry_type = if floating_point {
        SAMPLE_ENTRY_FPCM
    } else {
        SAMPLE_ENTRY_IPCM
    };
    let sample_entry_box = build_pcm_sample_entry_box(
        sample_entry_type,
        audio_format.sample_rate,
        audio_format.channel_count,
        audio_format.bits_per_sample,
        true,
    )?;
    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        if !chunk
            .data_size
            .is_multiple_of(u32::from(audio_format.block_align))
        {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI PCM stream {stream_index} chunk size {} is not a whole number of PCM frames",
                    chunk.data_size
                ),
            ));
        }
        let duration = chunk.data_size / u32::from(audio_format.block_align);
        if duration == 0 {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI PCM stream {stream_index} chunk did not contain a complete audio frame"
                ),
            ));
        }
        samples.push(CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    Ok(TrackCandidate {
        track_id: stream_index + 1,
        kind: MuxTrackKind::Audio,
        timescale: audio_format.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("pcm"),
        mux_policy: direct_ingest_mux_policy("pcm", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

fn finalize_avi_companded_track(
    spec: &str,
    source_index: usize,
    stream_index: u32,
    audio_format: AviAudioFormat,
    chunks: Vec<AviChunkSpan>,
    sample_entry_type: FourCc,
    codec_label: &str,
) -> Result<TrackCandidate, MuxError> {
    if audio_format.block_align == 0 {
        return Err(invalid_avi(
            spec,
            &format!("AVI audio stream {stream_index} declared a zero block align"),
        ));
    }
    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        if !chunk
            .data_size
            .is_multiple_of(u32::from(audio_format.block_align))
        {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI audio stream {stream_index} chunk size {} is not a whole number of audio sample frames",
                    chunk.data_size
                ),
            ));
        }
        let duration = chunk.data_size / u32::from(audio_format.block_align);
        if duration == 0 {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI audio stream {stream_index} chunk did not contain a complete audio frame"
                ),
            ));
        }
        samples.push(CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    let sample_entry_sample_size = if sample_entry_type == SAMPLE_ENTRY_MLAW {
        16
    } else {
        audio_format.bits_per_sample
    };
    let mut child_boxes = Vec::new();
    if sample_entry_type == SAMPLE_ENTRY_MLAW {
        child_boxes.push(super::super::mp4::encode_typed_box(
            &build_btrt_from_sample_sizes(
                samples.iter().map(|sample| (sample.data_size, sample.duration)),
                audio_format.sample_rate,
            )?,
            &[],
        )?);
    }
    let sample_entry_box = build_generic_audio_sample_entry_box(
        sample_entry_type,
        audio_format.sample_rate,
        audio_format.channel_count,
        sample_entry_sample_size,
        &child_boxes,
    )?;
    Ok(TrackCandidate {
        track_id: stream_index + 1,
        kind: MuxTrackKind::Audio,
        timescale: audio_format.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name(codec_label),
        mux_policy: direct_ingest_mux_policy(codec_label, MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

fn finalize_avi_generic_coded_audio_track(
    spec: &str,
    source_index: usize,
    stream_index: u32,
    audio_format: AviAudioFormat,
    chunks: Vec<AviChunkSpan>,
    sample_entry_type: FourCc,
    codec_label: &str,
) -> Result<TrackCandidate, MuxError> {
    let channels = u32::from(audio_format.channel_count);
    if channels == 0 {
        return Err(invalid_avi(
            spec,
            &format!("AVI audio stream {stream_index} declared zero channels"),
        ));
    }
    let bits_per_sample = u32::from(audio_format.bits_per_sample);
    if bits_per_sample == 0 {
        return Err(invalid_avi(
            spec,
            &format!("AVI audio stream {stream_index} declared zero bits per sample"),
        ));
    }
    let coded_sample_width = bits_per_sample
        .checked_mul(channels)
        .ok_or_else(|| invalid_avi(spec, "AVI coded-audio sample width overflow"))?;
    let sample_entry_box = build_generic_audio_sample_entry_box(
        sample_entry_type,
        audio_format.sample_rate,
        audio_format.channel_count,
        audio_format.bits_per_sample,
        &[],
    )?;
    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let coded_payload_bits = chunk
            .data_size
            .checked_mul(8)
            .ok_or(MuxError::LayoutOverflow("AVI coded-audio payload bits"))?;
        // Mirror the local reference AVI demux timing model for framed coded audio:
        // duration comes from payload size, coded sample width, and channel count.
        let duration = coded_payload_bits / coded_sample_width;
        if duration == 0 {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI audio stream {stream_index} chunk did not contain one complete coded audio sample frame"
                ),
            ));
        }
        samples.push(CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    Ok(TrackCandidate {
        track_id: stream_index + 1,
        kind: MuxTrackKind::Audio,
        timescale: audio_format.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name(codec_label),
        mux_policy: direct_ingest_mux_policy(codec_label, MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

fn finalize_avi_adpcm_track(
    spec: &str,
    source_index: usize,
    stream_index: u32,
    audio_format: AviAudioFormat,
    chunks: Vec<AviChunkSpan>,
    adpcm_kind: AviAdpcmKind,
) -> Result<TrackCandidate, MuxError> {
    if audio_format.block_align == 0 {
        return Err(invalid_avi(
            spec,
            &format!("AVI audio stream {stream_index} declared a zero block align"),
        ));
    }
    let (sample_entry_type, codec_label, samples_per_block) =
        avi_adpcm_parameters(spec, stream_index, audio_format, adpcm_kind)?;
    let sample_entry_box = build_generic_audio_sample_entry_box(
        sample_entry_type,
        audio_format.sample_rate,
        audio_format.channel_count,
        audio_format.bits_per_sample,
        &[],
    )?;
    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        if !chunk
            .data_size
            .is_multiple_of(u32::from(audio_format.block_align))
        {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI audio stream {stream_index} chunk size {} is not a whole number of ADPCM blocks",
                    chunk.data_size
                ),
            ));
        }
        let block_count = chunk.data_size / u32::from(audio_format.block_align);
        if block_count == 0 {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI audio stream {stream_index} chunk did not contain one complete ADPCM block"
                ),
            ));
        }
        let duration = block_count.checked_mul(samples_per_block).ok_or_else(|| {
            invalid_avi(
                spec,
                &format!("AVI audio stream {stream_index} ADPCM duration overflowed"),
            )
        })?;
        samples.push(CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    Ok(TrackCandidate {
        track_id: stream_index + 1,
        kind: MuxTrackKind::Audio,
        timescale: audio_format.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name(codec_label),
        mux_policy: direct_ingest_mux_policy(codec_label, MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

fn avi_adpcm_parameters(
    spec: &str,
    stream_index: u32,
    audio_format: AviAudioFormat,
    adpcm_kind: AviAdpcmKind,
) -> Result<(FourCc, &'static str, u32), MuxError> {
    let channels = u32::from(audio_format.channel_count);
    if channels == 0 {
        return Err(invalid_avi(
            spec,
            &format!("AVI audio stream {stream_index} declared zero channels"),
        ));
    }
    let bits_per_sample = u32::from(audio_format.bits_per_sample);
    if bits_per_sample == 0 {
        return Err(invalid_avi(
            spec,
            &format!("AVI audio stream {stream_index} declared zero bits per sample"),
        ));
    }
    let block_align = u32::from(audio_format.block_align);
    let (sample_entry_type, codec_label, header_bytes_per_channel, leading_samples) =
        match adpcm_kind {
            AviAdpcmKind::Microsoft => (SAMPLE_ENTRY_MS_ADPCM, "adpcm", 7_u32, 2_u32),
            AviAdpcmKind::ImaDvi => (SAMPLE_ENTRY_IMA_ADPCM, "ima-adpcm", 4_u32, 1_u32),
        };
    let header_bytes = header_bytes_per_channel
        .checked_mul(channels)
        .ok_or_else(|| invalid_avi(spec, "AVI ADPCM header-size overflow"))?;
    if block_align <= header_bytes {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI audio stream {stream_index} declared block_align {} too small for its ADPCM header",
                audio_format.block_align
            ),
        ));
    }
    let coded_payload_bits = block_align
        .checked_sub(header_bytes)
        .and_then(|value| value.checked_mul(8))
        .ok_or_else(|| invalid_avi(spec, "AVI ADPCM coded-payload size overflow"))?;
    let coded_sample_width = bits_per_sample
        .checked_mul(channels)
        .ok_or_else(|| invalid_avi(spec, "AVI ADPCM coded-sample width overflow"))?;
    if coded_sample_width == 0 || coded_payload_bits % coded_sample_width != 0 {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI audio stream {stream_index} declared one unsupported ADPCM block geometry (block_align={}, channels={}, bits_per_sample={})",
                audio_format.block_align, audio_format.channel_count, audio_format.bits_per_sample
            ),
        ));
    }
    let samples_per_block = coded_payload_bits / coded_sample_width + leading_samples;
    if samples_per_block == 0 {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI audio stream {stream_index} did not expose one complete ADPCM sample block"
            ),
        ));
    }
    Ok((sample_entry_type, codec_label, samples_per_block))
}

fn finalize_avi_mp3_track_sync(
    file: &mut File,
    path: &Path,
    spec: &str,
    descriptor: AviTrackDescriptor,
    chunks: Vec<AviChunkSpan>,
) -> Result<CompositeTrackCandidate, MuxError> {
    let source_spec = build_avi_segmented_source_spec(path, &chunks)?;
    let parsed =
        scan_mp3_segmented_sync(file, &source_spec.segments, source_spec.total_size, spec)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: descriptor.stream_index + 1,
            kind: MuxTrackKind::Audio,
            timescale: parsed.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("mp3"),
            mux_policy: direct_ingest_mux_policy("mp3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box: parsed.sample_entry_box,
            source_edit_media_time: None,
            samples: candidate_samples_from_staged(parsed.samples),
        },
        source_spec,
    })
}

#[cfg(feature = "async")]
async fn finalize_avi_mp3_track_async(
    file: &mut TokioFile,
    path: &Path,
    spec: &str,
    descriptor: AviTrackDescriptor,
    chunks: Vec<AviChunkSpan>,
) -> Result<CompositeTrackCandidate, MuxError> {
    let source_spec = build_avi_segmented_source_spec(path, &chunks)?;
    let parsed =
        scan_mp3_segmented_async(file, &source_spec.segments, source_spec.total_size, spec).await?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: descriptor.stream_index + 1,
            kind: MuxTrackKind::Audio,
            timescale: parsed.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("mp3"),
            mux_policy: direct_ingest_mux_policy("mp3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box: parsed.sample_entry_box,
            source_edit_media_time: None,
            samples: candidate_samples_from_staged(parsed.samples),
        },
        source_spec,
    })
}

fn finalize_avi_ac3_track_sync(
    file: &mut File,
    path: &Path,
    spec: &str,
    descriptor: AviTrackDescriptor,
    chunks: Vec<AviChunkSpan>,
) -> Result<CompositeTrackCandidate, MuxError> {
    let source_spec = build_avi_segmented_source_spec(path, &chunks)?;
    let parsed =
        scan_ac3_segmented_sync(file, &source_spec.segments, source_spec.total_size, spec)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: descriptor.stream_index + 1,
            kind: MuxTrackKind::Audio,
            timescale: parsed.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("ac3"),
            mux_policy: direct_ingest_mux_policy("ac3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box: parsed.sample_entry_box,
            source_edit_media_time: None,
            samples: candidate_samples_from_staged(parsed.samples),
        },
        source_spec,
    })
}

#[cfg(feature = "async")]
async fn finalize_avi_ac3_track_async(
    file: &mut TokioFile,
    path: &Path,
    spec: &str,
    descriptor: AviTrackDescriptor,
    chunks: Vec<AviChunkSpan>,
) -> Result<CompositeTrackCandidate, MuxError> {
    let source_spec = build_avi_segmented_source_spec(path, &chunks)?;
    let parsed =
        scan_ac3_segmented_async(file, &source_spec.segments, source_spec.total_size, spec).await?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: descriptor.stream_index + 1,
            kind: MuxTrackKind::Audio,
            timescale: parsed.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("ac3"),
            mux_policy: direct_ingest_mux_policy("ac3", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box: parsed.sample_entry_box,
            source_edit_media_time: None,
            samples: candidate_samples_from_staged(parsed.samples),
        },
        source_spec,
    })
}

fn finalize_avi_adts_track_sync(
    file: &mut File,
    path: &Path,
    spec: &str,
    descriptor: AviTrackDescriptor,
    chunks: Vec<AviChunkSpan>,
) -> Result<CompositeTrackCandidate, MuxError> {
    let source_spec = build_avi_segmented_source_spec(path, &chunks)?;
    let parsed =
        scan_adts_segmented_sync(file, &source_spec.segments, source_spec.total_size, spec)?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: descriptor.stream_index + 1,
            kind: MuxTrackKind::Audio,
            timescale: parsed.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("aac"),
            mux_policy: direct_ingest_mux_policy("aac", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box: parsed.sample_entry_box,
            source_edit_media_time: None,
            samples: candidate_samples_from_staged(parsed.samples),
        },
        source_spec,
    })
}

#[cfg(feature = "async")]
async fn finalize_avi_adts_track_async(
    file: &mut TokioFile,
    path: &Path,
    spec: &str,
    descriptor: AviTrackDescriptor,
    chunks: Vec<AviChunkSpan>,
) -> Result<CompositeTrackCandidate, MuxError> {
    let source_spec = build_avi_segmented_source_spec(path, &chunks)?;
    let parsed =
        scan_adts_segmented_async(file, &source_spec.segments, source_spec.total_size, spec)
            .await?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: descriptor.stream_index + 1,
            kind: MuxTrackKind::Audio,
            timescale: parsed.sample_rate,
            language: *b"und",
            handler_name: direct_ingest_handler_name("aac"),
            mux_policy: direct_ingest_mux_policy("aac", MuxTrackKind::Audio),
            width: 0,
            height: 0,
            sample_entry_box: parsed.sample_entry_box,
            source_edit_media_time: None,
            samples: candidate_samples_from_staged(parsed.samples),
        },
        source_spec,
    })
}

fn finalize_avi_raw_aac_track(
    spec: &str,
    source_index: usize,
    stream_index: u32,
    audio_format: AviAudioFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    let sample_entry_box =
        build_aac_lc_sample_entry_box(audio_format.sample_rate, audio_format.channel_count)?;
    let samples = chunks
        .into_iter()
        .map(|chunk| CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration: 1_024,
            composition_time_offset: 0,
            is_sync_sample: true,
        })
        .collect::<Vec<_>>();
    if samples.is_empty() {
        return Err(invalid_avi(
            spec,
            &format!("AVI AAC stream {stream_index} did not contain any audio chunks"),
        ));
    }
    Ok(TrackCandidate {
        track_id: stream_index + 1,
        kind: MuxTrackKind::Audio,
        timescale: audio_format.sample_rate,
        language: *b"und",
        handler_name: direct_ingest_handler_name("aac"),
        mux_policy: direct_ingest_mux_policy("aac", MuxTrackKind::Audio),
        width: 0,
        height: 0,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

fn finalize_avi_mp4v_track_sync(
    file: &mut File,
    path: &Path,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let input_spec = build_avi_segmented_source_spec(path, &chunks)?;
    let (decoder_specific_info, parsed_samples) = match scan_mp4v_segmented_sync(
        file,
        &input_spec.segments,
        input_spec.total_size,
        spec,
    ) {
        Ok(parsed) => {
            if parsed.samples.len() != chunks.len() {
                return Err(invalid_avi(
                    spec,
                    &format!(
                        "AVI MPEG-4 Part 2 stream {} did not map one chunk to one access unit on the native direct-ingest path",
                        descriptor.stream_index
                    ),
                ));
            }
            if parsed.width != video_format.width || parsed.height != video_format.height {
                return Err(invalid_avi(
                    spec,
                    &format!(
                        "AVI MPEG-4 Part 2 stream {} carried container dimensions that disagreed with the decoder configuration",
                        descriptor.stream_index
                    ),
                ));
            }
            (parsed.decoder_specific_info, Some(parsed.samples))
        }
        Err(MuxError::UnsupportedTrackImport { message, .. })
            if message == "MPEG-4 Part 2 decoder config did not precede the first VOP sample"
                && !video_format.decoder_specific_info.is_empty() =>
        {
            (video_format.decoder_specific_info.clone(), None)
        }
        Err(error) => return Err(error),
    };
    let native_samples = if let Some(parsed_samples) = parsed_samples {
        build_native_avi_mp4v_candidate_samples(
            spec,
            descriptor.stream_index,
            source_index,
            &chunks,
            parsed_samples,
        )?
    } else {
        None
    };
    let (samples, sample_sizes) = match native_samples {
        Some(native) => native,
        None => build_fallback_avi_mp4v_candidate_samples_sync(
            file,
            spec,
            descriptor.stream_index,
            source_index,
            timing.sample_duration,
            &decoder_specific_info,
            &chunks,
        )?,
    };
    Ok(TrackCandidate {
        track_id: descriptor.stream_index + 1,
        kind: MuxTrackKind::Video,
        timescale: timing.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("mp4v"),
        mux_policy: direct_ingest_mux_policy("mp4v", MuxTrackKind::Video),
        width: video_format.width,
        height: video_format.height,
        sample_entry_box: build_direct_mp4v_sample_entry_box(
            video_format.width,
            video_format.height,
            &decoder_specific_info,
            timing.timescale,
            sample_sizes,
        )?,
        source_edit_media_time: None,
        samples,
    })
}

#[cfg(feature = "async")]
async fn finalize_avi_mp4v_track_async(
    file: &mut TokioFile,
    path: &Path,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let input_spec = build_avi_segmented_source_spec(path, &chunks)?;
    let (decoder_specific_info, parsed_samples) = match scan_mp4v_segmented_async(
        file,
        &input_spec.segments,
        input_spec.total_size,
        spec,
    )
    .await
    {
        Ok(parsed) => {
            if parsed.samples.len() != chunks.len() {
                return Err(invalid_avi(
                    spec,
                    &format!(
                        "AVI MPEG-4 Part 2 stream {} did not map one chunk to one access unit on the native direct-ingest path",
                        descriptor.stream_index
                    ),
                ));
            }
            if parsed.width != video_format.width || parsed.height != video_format.height {
                return Err(invalid_avi(
                    spec,
                    &format!(
                        "AVI MPEG-4 Part 2 stream {} carried container dimensions that disagreed with the decoder configuration",
                        descriptor.stream_index
                    ),
                ));
            }
            (parsed.decoder_specific_info, Some(parsed.samples))
        }
        Err(MuxError::UnsupportedTrackImport { message, .. })
            if message == "MPEG-4 Part 2 decoder config did not precede the first VOP sample"
                && !video_format.decoder_specific_info.is_empty() =>
        {
            (video_format.decoder_specific_info.clone(), None)
        }
        Err(error) => return Err(error),
    };
    let native_samples = if let Some(parsed_samples) = parsed_samples {
        build_native_avi_mp4v_candidate_samples(
            spec,
            descriptor.stream_index,
            source_index,
            &chunks,
            parsed_samples,
        )?
    } else {
        None
    };
    let (samples, sample_sizes) = match native_samples {
        Some(native) => native,
        None => {
            build_fallback_avi_mp4v_candidate_samples_async(
                file,
                spec,
                descriptor.stream_index,
                source_index,
                timing.sample_duration,
                &decoder_specific_info,
                &chunks,
            )
            .await?
        }
    };
    Ok(TrackCandidate {
        track_id: descriptor.stream_index + 1,
        kind: MuxTrackKind::Video,
        timescale: timing.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("mp4v"),
        mux_policy: direct_ingest_mux_policy("mp4v", MuxTrackKind::Video),
        width: video_format.width,
        height: video_format.height,
        sample_entry_box: build_direct_mp4v_sample_entry_box(
            video_format.width,
            video_format.height,
            &decoder_specific_info,
            timing.timescale,
            sample_sizes,
        )?,
        source_edit_media_time: None,
        samples,
    })
}

fn build_native_avi_mp4v_candidate_samples(
    spec: &str,
    stream_index: u32,
    source_index: usize,
    chunks: &[AviChunkSpan],
    parsed_samples: Vec<StagedSample>,
) -> Result<Option<AviCandidateSamples>, MuxError> {
    let mut logical_chunk_offset = 0_u64;
    let mut samples = Vec::with_capacity(parsed_samples.len());
    let mut sample_sizes = Vec::with_capacity(parsed_samples.len());
    for (chunk, sample) in chunks.iter().zip(parsed_samples.into_iter()) {
        let sample_start = sample
            .data_offset
            .checked_sub(logical_chunk_offset)
            .ok_or_else(|| {
                invalid_avi(
                    spec,
                    &format!(
                        "AVI MPEG-4 Part 2 stream {} produced one sample before its chunk start",
                        stream_index
                    ),
                )
            })?;
        let sample_end = sample_start
            .checked_add(u64::from(sample.data_size))
            .ok_or(MuxError::LayoutOverflow("AVI MPEG-4 Part 2 sample end"))?;
        if sample_end > u64::from(chunk.data_size) {
            return Ok(None);
        }
        let data_offset = chunk
            .data_offset
            .checked_add(sample_start)
            .ok_or(MuxError::LayoutOverflow("AVI MPEG-4 Part 2 sample offset"))?;
        sample_sizes.push((sample.data_size, sample.duration));
        samples.push(CandidateSample {
            source_index,
            data_offset,
            data_size: sample.data_size,
            duration: sample.duration,
            composition_time_offset: sample.composition_time_offset,
            is_sync_sample: sample.is_sync_sample,
        });
        logical_chunk_offset = logical_chunk_offset
            .checked_add(u64::from(chunk.data_size))
            .ok_or(MuxError::LayoutOverflow("AVI MPEG-4 Part 2 logical offset"))?;
    }
    Ok(Some((samples, sample_sizes)))
}

type AviCandidateSamples = (Vec<CandidateSample>, Vec<(u32, u32)>);

fn trimmed_avi_mp4v_chunk_payload(
    spec: &str,
    stream_index: u32,
    chunk: &AviChunkSpan,
    frame: &[u8],
    decoder_specific_info: &[u8],
) -> Result<(u64, u32), MuxError> {
    if decoder_specific_info.is_empty() || !frame.starts_with(decoder_specific_info) {
        return Ok((chunk.data_offset, chunk.data_size));
    }

    let trimmed_prefix = u64::try_from(decoder_specific_info.len())
        .map_err(|_| MuxError::LayoutOverflow("AVI MPEG-4 Part 2 decoder config size"))?;
    let trimmed_prefix_u32 = u32::try_from(trimmed_prefix)
        .map_err(|_| MuxError::LayoutOverflow("AVI MPEG-4 Part 2 decoder config size"))?;
    let trimmed_size = chunk
        .data_size
        .checked_sub(trimmed_prefix_u32)
        .ok_or_else(|| {
            invalid_avi(
                spec,
                &format!(
                    "AVI MPEG-4 Part 2 stream {} carried one chunk with only decoder configuration and no VOP payload",
                    stream_index
                ),
            )
        })?;
    if trimmed_size == 0 {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI MPEG-4 Part 2 stream {} carried one zero-length VOP payload after stripping duplicated decoder configuration",
                stream_index
            ),
        ));
    }

    let trimmed_offset = chunk
        .data_offset
        .checked_add(trimmed_prefix)
        .ok_or(MuxError::LayoutOverflow("AVI MPEG-4 Part 2 sample offset"))?;
    Ok((trimmed_offset, trimmed_size))
}

fn build_fallback_avi_mp4v_candidate_samples_sync(
    file: &mut File,
    spec: &str,
    stream_index: u32,
    source_index: usize,
    sample_duration: u32,
    decoder_specific_info: &[u8],
    chunks: &[AviChunkSpan],
) -> Result<AviCandidateSamples, MuxError> {
    let mut samples = Vec::with_capacity(chunks.len());
    let mut sample_sizes = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        if chunk.data_size == 0 {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI video stream {} carried one zero-length chunk",
                    stream_index
                ),
            ));
        }
        let mut frame = vec![
            0_u8;
            usize::try_from(chunk.data_size)
                .map_err(|_| MuxError::LayoutOverflow("AVI video chunk size"))?
        ];
        read_exact_at_sync(
            file,
            chunk.data_offset,
            &mut frame,
            spec,
            "AVI video chunk is truncated",
        )?;
        let is_sync_sample = avi_mp4v_chunk_is_sync_sample(spec, stream_index, &frame)?;
        let (data_offset, data_size) = trimmed_avi_mp4v_chunk_payload(
            spec,
            stream_index,
            chunk,
            &frame,
            decoder_specific_info,
        )?;
        sample_sizes.push((data_size, sample_duration));
        samples.push(CandidateSample {
            source_index,
            data_offset,
            data_size,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample,
        });
    }
    Ok((samples, sample_sizes))
}

#[cfg(feature = "async")]
async fn build_fallback_avi_mp4v_candidate_samples_async(
    file: &mut TokioFile,
    spec: &str,
    stream_index: u32,
    source_index: usize,
    sample_duration: u32,
    decoder_specific_info: &[u8],
    chunks: &[AviChunkSpan],
) -> Result<AviCandidateSamples, MuxError> {
    let mut samples = Vec::with_capacity(chunks.len());
    let mut sample_sizes = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        if chunk.data_size == 0 {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI video stream {} carried one zero-length chunk",
                    stream_index
                ),
            ));
        }
        let mut frame = vec![
            0_u8;
            usize::try_from(chunk.data_size)
                .map_err(|_| MuxError::LayoutOverflow("AVI video chunk size"))?
        ];
        read_exact_at_async(
            file,
            chunk.data_offset,
            &mut frame,
            spec,
            "AVI video chunk is truncated",
        )
        .await?;
        let is_sync_sample = avi_mp4v_chunk_is_sync_sample(spec, stream_index, &frame)?;
        let (data_offset, data_size) = trimmed_avi_mp4v_chunk_payload(
            spec,
            stream_index,
            chunk,
            &frame,
            decoder_specific_info,
        )?;
        sample_sizes.push((data_size, sample_duration));
        samples.push(CandidateSample {
            source_index,
            data_offset,
            data_size,
            duration: sample_duration,
            composition_time_offset: 0,
            is_sync_sample,
        });
    }
    Ok((samples, sample_sizes))
}

fn finalize_avi_h264_track_sync(
    file: &mut File,
    path: &Path,
    spec: &str,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<CompositeTrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let input_spec = build_avi_segmented_source_spec(path, &chunks)?;
    let mut staged = stage_annex_b_h264_segmented_sync(
        path,
        file,
        &input_spec.segments,
        input_spec.total_size,
        spec,
    )?;
    if staged.samples.len() != chunks.len() {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI H.264 stream {} did not map one chunk to one access unit on the native direct-ingest path",
                descriptor.stream_index
            ),
        ));
    }
    if staged.track_width != video_format.width || staged.track_height != video_format.height {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI H.264 stream {} carried container dimensions that disagreed with the SPS dimensions",
                descriptor.stream_index
            ),
        ));
    }
    for sample in &mut staged.samples {
        sample.duration = timing.sample_duration;
        sample.composition_time_offset = 0;
    }
    let sample_entry_box = retune_carried_h264_sample_entry_box(
        &staged.sample_entry_box,
        timing.timescale,
        Some(super::h264::authored_h264_media_duration(
            staged
                .samples
                .iter()
                .map(|sample| (sample.duration, sample.composition_time_offset)),
        )?),
        staged
            .samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        true,
        false,
    )?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: descriptor.stream_index + 1,
            kind: MuxTrackKind::Video,
            timescale: timing.timescale,
            language: *b"und",
            handler_name: direct_ingest_handler_name("h264"),
            mux_policy: direct_ingest_mux_policy("h264", MuxTrackKind::Video),
            width: staged.track_width,
            height: staged.track_height,
            sample_entry_box,
            source_edit_media_time: None,
            samples: candidate_samples_from_staged(staged.samples),
        },
        source_spec: staged.segmented_source,
    })
}

#[cfg(feature = "async")]
async fn finalize_avi_h264_track_async(
    file: &mut TokioFile,
    path: &Path,
    spec: &str,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<CompositeTrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let input_spec = build_avi_segmented_source_spec(path, &chunks)?;
    let mut staged = stage_annex_b_h264_segmented_async(
        path,
        file,
        &input_spec.segments,
        input_spec.total_size,
        spec,
    )
    .await?;
    if staged.samples.len() != chunks.len() {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI H.264 stream {} did not map one chunk to one access unit on the native direct-ingest path",
                descriptor.stream_index
            ),
        ));
    }
    if staged.track_width != video_format.width || staged.track_height != video_format.height {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI H.264 stream {} carried container dimensions that disagreed with the SPS dimensions",
                descriptor.stream_index
            ),
        ));
    }
    for sample in &mut staged.samples {
        sample.duration = timing.sample_duration;
        sample.composition_time_offset = 0;
    }
    let sample_entry_box = retune_carried_h264_sample_entry_box(
        &staged.sample_entry_box,
        timing.timescale,
        Some(super::h264::authored_h264_media_duration(
            staged
                .samples
                .iter()
                .map(|sample| (sample.duration, sample.composition_time_offset)),
        )?),
        staged
            .samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
        true,
        false,
    )?;
    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: descriptor.stream_index + 1,
            kind: MuxTrackKind::Video,
            timescale: timing.timescale,
            language: *b"und",
            handler_name: direct_ingest_handler_name("h264"),
            mux_policy: direct_ingest_mux_policy("h264", MuxTrackKind::Video),
            width: staged.track_width,
            height: staged.track_height,
            sample_entry_box,
            source_edit_media_time: None,
            samples: candidate_samples_from_staged(staged.samples),
        },
        source_spec: staged.segmented_source,
    })
}

fn finalize_avi_h264_avc1_track_sync(
    file: &mut File,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let avcc = parse_avi_avc1_decoder_configuration(
        spec,
        descriptor.stream_index,
        &video_format.decoder_specific_info,
    )?;
    let length_size = usize::from(avcc.length_size_minus_one) + 1;
    let (sample_entry_box, coded_width, coded_height) =
        build_h264_sample_entry_from_avc_config_with_options(&avcc, spec, false)?;
    if coded_width != video_format.width || coded_height != video_format.height {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI H.264 `avc1` stream {} carried container dimensions that disagreed with the decoder configuration",
                descriptor.stream_index
            ),
        ));
    }

    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        if chunk.data_size == 0 {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI video stream {} carried one zero-length chunk",
                    descriptor.stream_index
                ),
            ));
        }
        let mut frame = vec![
            0_u8;
            usize::try_from(chunk.data_size)
                .map_err(|_| MuxError::LayoutOverflow("AVI video chunk size"))?
        ];
        read_exact_at_sync(
            file,
            chunk.data_offset,
            &mut frame,
            spec,
            "AVI video chunk is truncated",
        )?;
        let is_sync_sample =
            avi_avc1_chunk_is_sync_sample(spec, descriptor.stream_index, &frame, length_size)?;
        samples.push(CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration: timing.sample_duration,
            composition_time_offset: 0,
            is_sync_sample,
        });
    }
    let sample_entry_box = append_btrt_to_visual_sample_entry(
        sample_entry_box,
        timing.timescale,
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
    )?;
    Ok(TrackCandidate {
        track_id: descriptor.stream_index + 1,
        kind: MuxTrackKind::Video,
        timescale: timing.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("h264"),
        mux_policy: avi_avc1_mux_policy(),
        width: video_format.width,
        height: video_format.height,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

#[cfg(feature = "async")]
async fn finalize_avi_h264_avc1_track_async(
    file: &mut TokioFile,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let avcc = parse_avi_avc1_decoder_configuration(
        spec,
        descriptor.stream_index,
        &video_format.decoder_specific_info,
    )?;
    let length_size = usize::from(avcc.length_size_minus_one) + 1;
    let (sample_entry_box, coded_width, coded_height) =
        build_h264_sample_entry_from_avc_config_with_options(&avcc, spec, false)?;
    if coded_width != video_format.width || coded_height != video_format.height {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI H.264 `avc1` stream {} carried container dimensions that disagreed with the decoder configuration",
                descriptor.stream_index
            ),
        ));
    }

    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        if chunk.data_size == 0 {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI video stream {} carried one zero-length chunk",
                    descriptor.stream_index
                ),
            ));
        }
        let mut frame = vec![
            0_u8;
            usize::try_from(chunk.data_size)
                .map_err(|_| MuxError::LayoutOverflow("AVI video chunk size"))?
        ];
        read_exact_at_async(
            file,
            chunk.data_offset,
            &mut frame,
            spec,
            "AVI video chunk is truncated",
        )
        .await?;
        let is_sync_sample =
            avi_avc1_chunk_is_sync_sample(spec, descriptor.stream_index, &frame, length_size)?;
        samples.push(CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration: timing.sample_duration,
            composition_time_offset: 0,
            is_sync_sample,
        });
    }
    let sample_entry_box = append_btrt_to_visual_sample_entry(
        sample_entry_box,
        timing.timescale,
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
    )?;
    Ok(TrackCandidate {
        track_id: descriptor.stream_index + 1,
        kind: MuxTrackKind::Video,
        timescale: timing.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("h264"),
        mux_policy: avi_avc1_mux_policy(),
        width: video_format.width,
        height: video_format.height,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

fn finalize_avi_h263_track_sync(
    file: &mut File,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let frame = read_avi_chunk_bytes_sync(file, chunk, spec, "AVI H.263 chunk is truncated")?;
        let (width, height, is_sync_sample) = parse_h263_picture_bytes(spec, &frame)?;
        if width != video_format.width || height != video_format.height {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI H.263 stream {} carried container dimensions that disagreed with the picture header",
                    descriptor.stream_index
                ),
            ));
        }
        samples.push(CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration: timing.sample_duration,
            composition_time_offset: 0,
            is_sync_sample,
        });
    }
    let sample_entry_box = build_avi_h263_sample_entry_box(
        video_format.width,
        video_format.height,
        build_btrt_from_sample_sizes(
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
            timing.timescale,
        )?,
    )?;
    Ok(TrackCandidate {
        track_id: descriptor.stream_index + 1,
        kind: MuxTrackKind::Video,
        timescale: timing.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("h263"),
        mux_policy: with_force_empty_sync_sample_table(direct_ingest_mux_policy(
            "h263",
            MuxTrackKind::Video,
        )),
        width: video_format.width,
        height: video_format.height,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

#[cfg(feature = "async")]
async fn finalize_avi_h263_track_async(
    file: &mut TokioFile,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let frame =
            read_avi_chunk_bytes_async(file, chunk, spec, "AVI H.263 chunk is truncated").await?;
        let (width, height, is_sync_sample) = parse_h263_picture_bytes(spec, &frame)?;
        if width != video_format.width || height != video_format.height {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI H.263 stream {} carried container dimensions that disagreed with the picture header",
                    descriptor.stream_index
                ),
            ));
        }
        samples.push(CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration: timing.sample_duration,
            composition_time_offset: 0,
            is_sync_sample,
        });
    }
    let sample_entry_box = build_avi_h263_sample_entry_box(
        video_format.width,
        video_format.height,
        build_btrt_from_sample_sizes(
            samples
                .iter()
                .map(|sample| (sample.data_size, sample.duration)),
            timing.timescale,
        )?,
    )?;
    Ok(TrackCandidate {
        track_id: descriptor.stream_index + 1,
        kind: MuxTrackKind::Video,
        timescale: timing.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("h263"),
        mux_policy: with_force_empty_sync_sample_table(direct_ingest_mux_policy(
            "h263",
            MuxTrackKind::Video,
        )),
        width: video_format.width,
        height: video_format.height,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

fn finalize_avi_jpeg_track_sync(
    file: &mut File,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    finalize_avi_still_image_track_sync(
        file,
        spec,
        source_index,
        descriptor,
        video_format,
        chunks,
        "jpeg",
    )
}

#[cfg(feature = "async")]
async fn finalize_avi_jpeg_track_async(
    file: &mut TokioFile,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    finalize_avi_still_image_track_async(
        file,
        spec,
        source_index,
        descriptor,
        video_format,
        chunks,
        "jpeg",
    )
    .await
}

fn finalize_avi_png_track_sync(
    file: &mut File,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    finalize_avi_still_image_track_sync(
        file,
        spec,
        source_index,
        descriptor,
        video_format,
        chunks,
        "png",
    )
}

#[cfg(feature = "async")]
async fn finalize_avi_png_track_async(
    file: &mut TokioFile,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    finalize_avi_still_image_track_async(
        file,
        spec,
        source_index,
        descriptor,
        video_format,
        chunks,
        "png",
    )
    .await
}

fn finalize_avi_still_image_track_sync(
    file: &mut File,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
    codec_label: &'static str,
) -> Result<TrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let mut sample_entry_box = None::<Vec<u8>>;
    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let frame =
            read_avi_chunk_bytes_sync(file, chunk, spec, "AVI still-image chunk is truncated")?;
        let (parsed_width, parsed_height, parsed_sample_entry_box) = match codec_label {
            "jpeg" => {
                let parsed = parse_jpeg_bytes(spec, &frame)?;
                (
                    parsed.width,
                    parsed.height,
                    build_avi_jpeg_sample_entry_box(parsed.width, parsed.height)?,
                )
            }
            "png" => {
                let parsed = parse_png_bytes(spec, &frame)?;
                (
                    parsed.width,
                    parsed.height,
                    build_avi_png_sample_entry_box(parsed.width, parsed.height)?,
                )
            }
            _ => unreachable!("AVI still-image helper only supports JPEG and PNG"),
        };
        if parsed_width != video_format.width || parsed_height != video_format.height {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI video stream {} carried container dimensions that disagreed with the still-image payload",
                    descriptor.stream_index
                ),
            ));
        }
        if sample_entry_box.is_none() {
            sample_entry_box = Some(parsed_sample_entry_box);
        }
        samples.push(CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration: timing.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    Ok(TrackCandidate {
        track_id: descriptor.stream_index + 1,
        kind: MuxTrackKind::Video,
        timescale: timing.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name(codec_label),
        mux_policy: with_force_empty_sync_sample_table(direct_ingest_mux_policy(
            codec_label,
            MuxTrackKind::Video,
        )),
        width: video_format.width,
        height: video_format.height,
        sample_entry_box: sample_entry_box.ok_or_else(|| {
            invalid_avi(
                spec,
                &format!(
                    "AVI video stream {} did not contain any still-image chunks",
                    descriptor.stream_index
                ),
            )
        })?,
        source_edit_media_time: None,
        samples,
    })
}

fn finalize_avi_generic_visual_track(
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
    sample_entry_type: FourCc,
    compressor_name: Vec<u8>,
) -> Result<TrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let samples = chunks
        .into_iter()
        .map(|chunk| CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration: timing.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: false,
        })
        .collect::<Vec<_>>();
    let sample_entry_box = build_avi_generic_visual_sample_entry_box(
        sample_entry_type,
        video_format.width,
        video_format.height,
        &compressor_name,
    )?;
    let sample_entry_box = append_btrt_to_visual_sample_entry(
        sample_entry_box,
        timing.timescale,
        samples
            .iter()
            .map(|sample| (sample.data_size, sample.duration)),
    )?;
    Ok(TrackCandidate {
        track_id: descriptor.stream_index + 1,
        kind: MuxTrackKind::Video,
        timescale: timing.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("mp4v"),
        mux_policy: with_force_empty_sync_sample_table(direct_ingest_mux_policy(
            "mp4v",
            MuxTrackKind::Video,
        )),
        width: video_format.width,
        height: video_format.height,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

fn finalize_avi_uncv_bgr_track(
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
) -> Result<TrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let samples = chunks
        .into_iter()
        .map(|chunk| CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration: timing.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        })
        .collect::<Vec<_>>();
    let sample_entry_box = build_avi_uncv_bgr_sample_entry_box(
        video_format.width,
        video_format.height,
        AVI_RAW_VIDEO_COMPRESSOR_NAME,
    )?;
    Ok(TrackCandidate {
        track_id: descriptor.stream_index + 1,
        kind: MuxTrackKind::Video,
        timescale: timing.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name("mp4v"),
        mux_policy: direct_ingest_mux_policy("mp4v", MuxTrackKind::Video),
        width: video_format.width,
        height: video_format.height,
        sample_entry_box,
        source_edit_media_time: None,
        samples,
    })
}

#[cfg(feature = "async")]
async fn finalize_avi_still_image_track_async(
    file: &mut TokioFile,
    spec: &str,
    source_index: usize,
    descriptor: AviTrackDescriptor,
    video_format: AviVideoFormat,
    chunks: Vec<AviChunkSpan>,
    codec_label: &'static str,
) -> Result<TrackCandidate, MuxError> {
    let timing = avi_video_timing(descriptor.timing_scale, descriptor.timing_rate);
    let mut sample_entry_box = None::<Vec<u8>>;
    let mut samples = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let frame =
            read_avi_chunk_bytes_async(file, chunk, spec, "AVI still-image chunk is truncated")
                .await?;
        let (parsed_width, parsed_height, parsed_sample_entry_box) = match codec_label {
            "jpeg" => {
                let parsed = parse_jpeg_bytes(spec, &frame)?;
                (
                    parsed.width,
                    parsed.height,
                    build_avi_jpeg_sample_entry_box(parsed.width, parsed.height)?,
                )
            }
            "png" => {
                let parsed = parse_png_bytes(spec, &frame)?;
                (
                    parsed.width,
                    parsed.height,
                    build_avi_png_sample_entry_box(parsed.width, parsed.height)?,
                )
            }
            _ => unreachable!("AVI still-image helper only supports JPEG and PNG"),
        };
        if parsed_width != video_format.width || parsed_height != video_format.height {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI video stream {} carried container dimensions that disagreed with the still-image payload",
                    descriptor.stream_index
                ),
            ));
        }
        if sample_entry_box.is_none() {
            sample_entry_box = Some(parsed_sample_entry_box);
        }
        samples.push(CandidateSample {
            source_index,
            data_offset: chunk.data_offset,
            data_size: chunk.data_size,
            duration: timing.sample_duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    Ok(TrackCandidate {
        track_id: descriptor.stream_index + 1,
        kind: MuxTrackKind::Video,
        timescale: timing.timescale,
        language: *b"und",
        handler_name: direct_ingest_handler_name(codec_label),
        mux_policy: with_force_empty_sync_sample_table(direct_ingest_mux_policy(
            codec_label,
            MuxTrackKind::Video,
        )),
        width: video_format.width,
        height: video_format.height,
        sample_entry_box: sample_entry_box.ok_or_else(|| {
            invalid_avi(
                spec,
                &format!(
                    "AVI video stream {} did not contain any still-image chunks",
                    descriptor.stream_index
                ),
            )
        })?,
        source_edit_media_time: None,
        samples,
    })
}

fn parse_hdrl_list_sync(
    file: &mut File,
    start: u64,
    end: u64,
    spec: &str,
    tracks: &mut Vec<AviTrackDescriptor>,
) -> Result<(), MuxError> {
    let mut offset = start;
    while offset < end {
        let (chunk_type, chunk_size, chunk_payload_offset, chunk_end) =
            read_riff_chunk_header_sync(file, end, offset, spec)?;
        if chunk_type == LIST {
            if chunk_size < 4 {
                return Err(invalid_avi(
                    spec,
                    "AVI stream list was truncated before its list type",
                ));
            }
            let list_type = read_fourcc_sync(
                file,
                chunk_payload_offset,
                spec,
                "AVI stream list type is truncated",
            )?;
            if list_type == STRL {
                tracks.push(parse_stream_list_sync(
                    file,
                    chunk_payload_offset + 4,
                    chunk_end,
                    spec,
                    u32::try_from(tracks.len())
                        .map_err(|_| MuxError::LayoutOverflow("AVI stream index"))?,
                )?);
            }
        }
        offset = next_riff_offset(chunk_end);
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn parse_hdrl_list_async(
    file: &mut TokioFile,
    start: u64,
    end: u64,
    spec: &str,
    tracks: &mut Vec<AviTrackDescriptor>,
) -> Result<(), MuxError> {
    let mut offset = start;
    while offset < end {
        let (chunk_type, chunk_size, chunk_payload_offset, chunk_end) =
            read_riff_chunk_header_async(file, end, offset, spec).await?;
        if chunk_type == LIST {
            if chunk_size < 4 {
                return Err(invalid_avi(
                    spec,
                    "AVI stream list was truncated before its list type",
                ));
            }
            let list_type = read_fourcc_async(
                file,
                chunk_payload_offset,
                spec,
                "AVI stream list type is truncated",
            )
            .await?;
            if list_type == STRL {
                tracks.push(
                    parse_stream_list_async(
                        file,
                        chunk_payload_offset + 4,
                        chunk_end,
                        spec,
                        u32::try_from(tracks.len())
                            .map_err(|_| MuxError::LayoutOverflow("AVI stream index"))?,
                    )
                    .await?,
                );
            }
        }
        offset = next_riff_offset(chunk_end);
    }
    Ok(())
}

fn parse_stream_list_sync(
    file: &mut File,
    start: u64,
    end: u64,
    spec: &str,
    stream_index: u32,
) -> Result<AviTrackDescriptor, MuxError> {
    let mut strh = None::<Vec<u8>>;
    let mut strf = None::<Vec<u8>>;
    let mut offset = start;
    while offset < end {
        let (chunk_type, chunk_size, chunk_payload_offset, chunk_end) =
            read_riff_chunk_header_sync(file, end, offset, spec)?;
        if matches!(chunk_type, STRH | STRF) {
            let mut bytes = vec![
                0_u8;
                usize::try_from(chunk_size).map_err(|_| {
                    MuxError::LayoutOverflow("AVI stream chunk size")
                })?
            ];
            read_exact_at_sync(
                file,
                chunk_payload_offset,
                &mut bytes,
                spec,
                "AVI stream chunk is truncated",
            )?;
            match chunk_type {
                STRH => strh = Some(bytes),
                STRF => strf = Some(bytes),
                _ => {}
            }
        }
        offset = next_riff_offset(chunk_end);
    }
    parse_stream_descriptor(spec, stream_index, strh, strf)
}

#[cfg(feature = "async")]
async fn parse_stream_list_async(
    file: &mut TokioFile,
    start: u64,
    end: u64,
    spec: &str,
    stream_index: u32,
) -> Result<AviTrackDescriptor, MuxError> {
    let mut strh = None::<Vec<u8>>;
    let mut strf = None::<Vec<u8>>;
    let mut offset = start;
    while offset < end {
        let (chunk_type, chunk_size, chunk_payload_offset, chunk_end) =
            read_riff_chunk_header_async(file, end, offset, spec).await?;
        if matches!(chunk_type, STRH | STRF) {
            let mut bytes = vec![
                0_u8;
                usize::try_from(chunk_size).map_err(|_| {
                    MuxError::LayoutOverflow("AVI stream chunk size")
                })?
            ];
            read_exact_at_async(
                file,
                chunk_payload_offset,
                &mut bytes,
                spec,
                "AVI stream chunk is truncated",
            )
            .await?;
            match chunk_type {
                STRH => strh = Some(bytes),
                STRF => strf = Some(bytes),
                _ => {}
            }
        }
        offset = next_riff_offset(chunk_end);
    }
    parse_stream_descriptor(spec, stream_index, strh, strf)
}

fn parse_stream_descriptor(
    spec: &str,
    stream_index: u32,
    strh: Option<Vec<u8>>,
    strf: Option<Vec<u8>>,
) -> Result<AviTrackDescriptor, MuxError> {
    let strh = strh.ok_or_else(|| invalid_avi(spec, "AVI stream list did not contain `strh`"))?;
    let strf = strf.ok_or_else(|| invalid_avi(spec, "AVI stream list did not contain `strf`"))?;
    if strh.len() < 36 {
        return Err(invalid_avi(
            spec,
            "AVI `strh` payload is shorter than 36 bytes",
        ));
    }
    let stream_type = FourCc::from_bytes(strh[0..4].try_into().unwrap());
    let stream_handler = FourCc::from_bytes(strh[4..8].try_into().unwrap());
    let scale = u32::from_le_bytes(strh[20..24].try_into().unwrap());
    let rate = u32::from_le_bytes(strh[24..28].try_into().unwrap());
    if scale == 0 || rate == 0 {
        return Err(invalid_avi(
            spec,
            &format!("AVI stream {stream_index} declared zero timing scale or rate"),
        ));
    }
    let audio_format = if stream_type == AUDS {
        Some(parse_avi_audio_format(spec, stream_index, &strf)?)
    } else {
        None
    };
    let video_format = if stream_type == VIDS {
        Some(parse_avi_video_format(
            spec,
            stream_index,
            stream_handler,
            &strf,
        )?)
    } else {
        None
    };
    Ok(AviTrackDescriptor {
        stream_index,
        stream_type,
        timing_scale: scale,
        timing_rate: rate,
        audio_format,
        video_format,
    })
}

fn parse_avi_audio_format(
    spec: &str,
    stream_index: u32,
    bytes: &[u8],
) -> Result<AviAudioFormat, MuxError> {
    if bytes.len() < 16 {
        return Err(invalid_avi(
            spec,
            &format!("AVI audio stream {stream_index} carried a truncated WAVEFORMAT payload"),
        ));
    }
    let mut format_tag = u16::from_le_bytes(bytes[0..2].try_into().unwrap());
    let channel_count = u16::from_le_bytes(bytes[2..4].try_into().unwrap());
    let sample_rate = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let block_align = u16::from_le_bytes(bytes[12..14].try_into().unwrap());
    let bits_per_sample = u16::from_le_bytes(bytes[14..16].try_into().unwrap());
    if channel_count == 0 || sample_rate == 0 {
        return Err(invalid_avi(
            spec,
            &format!("AVI audio stream {stream_index} declared zero channels or zero sample rate"),
        ));
    }
    if format_tag == WAVE_FORMAT_EXTENSIBLE {
        if bytes.len() < 40 {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI audio stream {stream_index} carried a truncated WAVE extensible payload"
                ),
            ));
        }
        let cb_size = u16::from_le_bytes(bytes[16..18].try_into().unwrap());
        if cb_size < 22 {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI audio stream {stream_index} carried one unsupported WAVE extensible extra size {cb_size}"
                ),
            ));
        }
        let subtype_guid = &bytes[24..40];
        format_tag = if subtype_guid == KSDATAFORMAT_SUBTYPE_PCM {
            WAVE_FORMAT_PCM
        } else if subtype_guid == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
            WAVE_FORMAT_IEEE_FLOAT
        } else if subtype_guid == KSDATAFORMAT_SUBTYPE_ALAW {
            WAVE_FORMAT_ALAW
        } else if subtype_guid == KSDATAFORMAT_SUBTYPE_MULAW {
            WAVE_FORMAT_MULAW
        } else {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI audio stream {stream_index} carried one unsupported WAVE extensible subtype GUID {}",
                    format_extensible_guid(subtype_guid)
                ),
            ));
        };
    }
    Ok(AviAudioFormat {
        format_tag,
        channel_count,
        sample_rate,
        block_align,
        bits_per_sample,
    })
}

fn format_extensible_guid(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join("")
}

fn parse_avi_video_format(
    spec: &str,
    stream_index: u32,
    stream_handler: FourCc,
    bytes: &[u8],
) -> Result<AviVideoFormat, MuxError> {
    if bytes.len() < 40 {
        return Err(invalid_avi(
            spec,
            &format!("AVI video stream {stream_index} carried a truncated BITMAPINFO payload"),
        ));
    }
    let header_size = usize::try_from(u32::from_le_bytes(bytes[0..4].try_into().unwrap()))
        .map_err(|_| MuxError::LayoutOverflow("AVI BITMAPINFO header size"))?;
    if header_size < 40 || bytes.len() < header_size {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI video stream {stream_index} carried one unsupported BITMAPINFO header size {header_size}"
            ),
        ));
    }
    let width = u16::try_from(i32::from_le_bytes(bytes[4..8].try_into().unwrap()).unsigned_abs())
        .map_err(|_| {
        invalid_avi(
            spec,
            "AVI video width does not fit in an MP4 visual sample entry",
        )
    })?;
    let height = u16::try_from(i32::from_le_bytes(bytes[8..12].try_into().unwrap()).unsigned_abs())
        .map_err(|_| {
            invalid_avi(
                spec,
                "AVI video height does not fit in an MP4 visual sample entry",
            )
        })?;
    let planes = u16::from_le_bytes(bytes[12..14].try_into().unwrap());
    if width == 0 || height == 0 || planes != 1 {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI video stream {stream_index} declared invalid width, height, or plane count"
            ),
        ));
    }
    let original_compression = FourCc::from_bytes(bytes[16..20].try_into().unwrap());
    let compression = normalize_avi_video_tag(original_compression);
    let handler = normalize_avi_video_tag(stream_handler);
    let codec = if avi_tag_maps_to_mp4v(compression) || avi_tag_maps_to_mp4v(handler) {
        AviVideoCodec::Mp4v
    } else if avi_tag_maps_to_h264_annex_b(compression) || avi_tag_maps_to_h264_annex_b(handler) {
        AviVideoCodec::H264AnnexB
    } else if compression == AVC1 || handler == AVC1 {
        AviVideoCodec::H264Avc1
    } else if avi_tag_maps_to_h263(compression) || avi_tag_maps_to_h263(handler) {
        AviVideoCodec::H263
    } else if avi_tag_maps_to_jpeg(compression) || avi_tag_maps_to_jpeg(handler) {
        AviVideoCodec::Jpeg
    } else if avi_tag_maps_to_png(compression) || avi_tag_maps_to_png(handler) {
        AviVideoCodec::Png
    } else if compression == SAMPLE_ENTRY_DIV3 || compression == SAMPLE_ENTRY_DIV4 {
        AviVideoCodec::MsMpeg4V3(SAMPLE_ENTRY_DIV3)
    } else if original_compression.into_bytes() == [0, 0, 0, 0] {
        AviVideoCodec::RawBgr
    } else {
        AviVideoCodec::GenericPassthrough(original_compression)
    };
    Ok(AviVideoFormat {
        width,
        height,
        codec,
        decoder_specific_info: bytes[header_size..].to_vec(),
    })
}

fn parse_avi_avc1_decoder_configuration(
    spec: &str,
    stream_index: u32,
    bytes: &[u8],
) -> Result<AVCDecoderConfiguration, MuxError> {
    if bytes.is_empty() {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI H.264 `avc1` stream {stream_index} did not carry an AVC decoder configuration payload"
            ),
        ));
    }
    let mut avcc = AVCDecoderConfiguration::default();
    unmarshal(
        &mut Cursor::new(bytes),
        u64::try_from(bytes.len()).map_err(|_| MuxError::LayoutOverflow("AVI avcC payload"))?,
        &mut avcc,
        None,
    )
    .map_err(|_| {
        invalid_avi(
            spec,
            &format!(
                "AVI H.264 `avc1` stream {stream_index} carried one invalid AVC decoder configuration payload"
            ),
        )
    })?;
    if avcc.configuration_version != 1
        || avcc.sequence_parameter_sets.is_empty()
        || avcc.picture_parameter_sets.is_empty()
    {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI H.264 `avc1` stream {stream_index} carried one incomplete AVC decoder configuration payload"
            ),
        ));
    }
    Ok(avcc)
}

fn avi_avc1_chunk_is_sync_sample(
    spec: &str,
    stream_index: u32,
    frame: &[u8],
    length_size: usize,
) -> Result<bool, MuxError> {
    if length_size == 0 || length_size > 4 {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI H.264 `avc1` stream {stream_index} declared unsupported NAL length width {length_size}"
            ),
        ));
    }
    let mut offset = 0usize;
    let mut saw_nal = false;
    let mut is_sync_sample = false;
    while offset < frame.len() {
        if frame.len() - offset < length_size {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI H.264 `avc1` stream {stream_index} carried one truncated length-prefixed access unit"
                ),
            ));
        }
        let mut nal_size = 0usize;
        for byte in &frame[offset..offset + length_size] {
            nal_size = (nal_size << 8) | usize::from(*byte);
        }
        offset += length_size;
        if nal_size == 0 || frame.len() - offset < nal_size {
            return Err(invalid_avi(
                spec,
                &format!(
                    "AVI H.264 `avc1` stream {stream_index} carried one invalid length-prefixed NAL unit"
                ),
            ));
        }
        saw_nal = true;
        if frame[offset] & 0x1F == 5 {
            is_sync_sample = true;
        }
        offset += nal_size;
    }
    if !saw_nal {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI H.264 `avc1` stream {stream_index} carried one empty length-prefixed access unit"
            ),
        ));
    }
    Ok(is_sync_sample)
}

fn append_btrt_to_visual_sample_entry<I>(
    mut sample_entry_box: Vec<u8>,
    timescale: u32,
    samples: I,
) -> Result<Vec<u8>, MuxError>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    let btrt_box = super::super::mp4::encode_typed_box(
        &build_btrt_from_sample_sizes(samples, timescale)?,
        &[],
    )?;
    if sample_entry_box.len() < 8 {
        return Err(MuxError::LayoutOverflow("AVI visual sample-entry header"));
    }
    let existing_size = u32::from_be_bytes(sample_entry_box[..4].try_into().unwrap());
    let appended_size = u32::try_from(btrt_box.len())
        .map_err(|_| MuxError::LayoutOverflow("AVI visual btrt child size"))?;
    let updated_size = existing_size
        .checked_add(appended_size)
        .ok_or(MuxError::LayoutOverflow("AVI visual sample-entry size"))?;
    sample_entry_box[..4].copy_from_slice(&updated_size.to_be_bytes());
    sample_entry_box.extend_from_slice(&btrt_box);
    Ok(sample_entry_box)
}

fn build_avi_generic_visual_sample_entry_box(
    sample_entry_type: FourCc,
    width: u16,
    height: u16,
    compressor_name: &[u8],
) -> Result<Vec<u8>, MuxError> {
    build_avi_generic_visual_sample_entry_box_with_children(
        sample_entry_type,
        width,
        height,
        compressor_name,
        &[],
    )
}

fn build_avi_generic_visual_sample_entry_box_with_children(
    sample_entry_type: FourCc,
    width: u16,
    height: u16,
    compressor_name: &[u8],
    child_boxes: &[Vec<u8>],
) -> Result<Vec<u8>, MuxError> {
    let mut compressorname = [0_u8; 32];
    let visible_len = compressor_name.len().min(31);
    compressorname[0] =
        u8::try_from(visible_len).map_err(|_| MuxError::LayoutOverflow("compressor name"))?;
    compressorname[1..1 + visible_len].copy_from_slice(&compressor_name[..visible_len]);
    super::super::mp4::encode_typed_box(
        &VisualSampleEntry {
            sample_entry: SampleEntry {
                box_type: sample_entry_type,
                data_reference_index: 1,
            },
            pre_defined2: [0, 0, 0],
            width,
            height,
            // The retained reference authoring writes literal 72 here, not 16.16 fixed-point.
            horizresolution: 72,
            vertresolution: 72,
            frame_count: 1,
            compressorname,
            depth: 0x0018,
            pre_defined3: -1,
            ..VisualSampleEntry::default()
        },
        &child_boxes.concat(),
    )
}

fn build_avi_uncv_bgr_sample_entry_box(
    width: u16,
    height: u16,
    compressor_name: &[u8],
) -> Result<Vec<u8>, MuxError> {
    let child_boxes = vec![
        build_avi_uncv_bgr_cmpd_box()?,
        build_avi_uncv_bgr_uncc_box()?,
    ];
    build_avi_generic_visual_sample_entry_box_with_children(
        SAMPLE_ENTRY_UNCV,
        width,
        height,
        compressor_name,
        &child_boxes,
    )
}

fn build_avi_uncv_bgr_cmpd_box() -> Result<Vec<u8>, MuxError> {
    let mut payload = Vec::with_capacity(10);
    payload.extend_from_slice(&3_u32.to_be_bytes());
    payload.extend_from_slice(&6_u16.to_be_bytes());
    payload.extend_from_slice(&5_u16.to_be_bytes());
    payload.extend_from_slice(&4_u16.to_be_bytes());
    super::super::mp4::encode_raw_box(CMPD, &payload)
}

fn build_avi_uncv_bgr_uncc_box() -> Result<Vec<u8>, MuxError> {
    let mut payload = Vec::with_capacity(51);
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&3_u32.to_be_bytes());
    for component_index in 0_u16..3 {
        payload.extend_from_slice(&component_index.to_be_bytes());
        payload.push(7);
        payload.push(0);
        payload.push(0);
    }
    payload.push(0);
    payload.push(1);
    payload.push(0);
    payload.push(0x08);
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    super::super::mp4::encode_raw_box(UNCC, &payload)
}

fn avi_avc1_mux_policy() -> super::super::import::ImportedTrackMuxPolicy {
    with_force_empty_sync_sample_table(direct_ingest_mux_policy("h264", MuxTrackKind::Video))
}

#[derive(Clone, Copy)]
struct AviVideoTiming {
    timescale: u32,
    sample_duration: u32,
}

fn avi_video_timing(frame_scale: u32, frame_rate: u32) -> AviVideoTiming {
    let fps_times_1000 = ((f64::from(frame_rate) / f64::from(frame_scale)) * 1000.0 + 0.5) as u32;
    match fps_times_1000 {
        29_970 => AviVideoTiming {
            timescale: 30_000,
            sample_duration: 1_001,
        },
        23_976 => AviVideoTiming {
            timescale: 24_000,
            sample_duration: 1_001,
        },
        59_940 => AviVideoTiming {
            timescale: 60_000,
            sample_duration: 1_001,
        },
        _ => AviVideoTiming {
            timescale: fps_times_1000,
            sample_duration: 1_000,
        },
    }
}

fn normalize_avi_video_tag(tag: FourCc) -> FourCc {
    let mut bytes = tag.into_bytes();
    for byte in &mut bytes {
        byte.make_ascii_uppercase();
    }
    FourCc::from_bytes(bytes)
}

fn avi_tag_maps_to_mp4v(tag: FourCc) -> bool {
    const TAGS: &[[u8; 4]] = &[
        *b"DIVX", *b"DX50", *b"XVID", *b"3IV2", *b"FVFW", *b"NDIG", *b"MP4V", *b"M4CC", *b"PVMM",
        *b"SEDG", *b"RMP4", *b"MP43", *b"FMP4", *b"VP6F",
    ];
    TAGS.contains(&tag.into_bytes())
}

fn avi_tag_maps_to_h264_annex_b(tag: FourCc) -> bool {
    matches!(
        tag.into_bytes(),
        [b'H', b'2', b'6', b'4'] | [b'X', b'2', b'6', b'4']
    )
}

fn avi_tag_maps_to_h263(tag: FourCc) -> bool {
    matches!(
        tag.into_bytes(),
        [b'H', b'2', b'6', b'3'] | [b'S', b'2', b'6', b'3']
    )
}

fn avi_tag_maps_to_jpeg(tag: FourCc) -> bool {
    matches!(
        tag.into_bytes(),
        [b'M', b'J', b'P', b'G'] | [b'J', b'P', b'E', b'G']
    )
}

fn avi_tag_maps_to_png(tag: FourCc) -> bool {
    matches!(tag.into_bytes(), [b'P', b'N', b'G', b' '])
}

fn read_avi_chunk_bytes_sync(
    file: &mut File,
    chunk: AviChunkSpan,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    if chunk.data_size == 0 {
        return Err(invalid_avi(spec, "AVI chunk payload was empty"));
    }
    let mut bytes = vec![
        0_u8;
        usize::try_from(chunk.data_size)
            .map_err(|_| MuxError::LayoutOverflow("AVI chunk size"))?
    ];
    read_exact_at_sync(file, chunk.data_offset, &mut bytes, spec, truncated_message)?;
    Ok(bytes)
}

#[cfg(feature = "async")]
async fn read_avi_chunk_bytes_async(
    file: &mut TokioFile,
    chunk: AviChunkSpan,
    spec: &str,
    truncated_message: &'static str,
) -> Result<Vec<u8>, MuxError> {
    if chunk.data_size == 0 {
        return Err(invalid_avi(spec, "AVI chunk payload was empty"));
    }
    let mut bytes = vec![
        0_u8;
        usize::try_from(chunk.data_size)
            .map_err(|_| MuxError::LayoutOverflow("AVI chunk size"))?
    ];
    read_exact_at_async(file, chunk.data_offset, &mut bytes, spec, truncated_message).await?;
    Ok(bytes)
}

fn parse_movi_chunks_sync(
    file: &mut File,
    start: u64,
    end: u64,
    spec: &str,
    track_count: usize,
    track_chunks: &mut [Vec<AviChunkSpan>],
) -> Result<(), MuxError> {
    let mut ranges = vec![(start, end)];
    while let Some((range_start, range_end)) = ranges.pop() {
        let mut offset = range_start;
        while offset < range_end {
            let (chunk_type, chunk_size, chunk_payload_offset, chunk_end) =
                read_riff_chunk_header_sync(file, range_end, offset, spec)?;
            if chunk_type == LIST {
                if chunk_size < 4 {
                    return Err(invalid_avi(
                        spec,
                        "AVI `movi` sub-list was truncated before its list type",
                    ));
                }
                let list_type = read_fourcc_sync(
                    file,
                    chunk_payload_offset,
                    spec,
                    "AVI `movi` sub-list type is truncated",
                )?;
                if list_type == RECL {
                    ranges.push((chunk_payload_offset + 4, chunk_end));
                }
            } else if let Some(stream_index) = parse_stream_chunk_index(chunk_type)
                && stream_index < track_count
            {
                track_chunks[stream_index].push(AviChunkSpan {
                    data_offset: chunk_payload_offset,
                    data_size: u32::try_from(chunk_size)
                        .map_err(|_| MuxError::LayoutOverflow("AVI chunk size"))?,
                });
            }
            offset = next_riff_offset(chunk_end);
        }
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn parse_movi_chunks_async(
    file: &mut TokioFile,
    start: u64,
    end: u64,
    spec: &str,
    track_count: usize,
    track_chunks: &mut [Vec<AviChunkSpan>],
) -> Result<(), MuxError> {
    let mut ranges = vec![(start, end)];
    while let Some((range_start, range_end)) = ranges.pop() {
        let mut offset = range_start;
        while offset < range_end {
            let (chunk_type, chunk_size, chunk_payload_offset, chunk_end) =
                read_riff_chunk_header_async(file, range_end, offset, spec).await?;
            if chunk_type == LIST {
                if chunk_size < 4 {
                    return Err(invalid_avi(
                        spec,
                        "AVI `movi` sub-list was truncated before its list type",
                    ));
                }
                let list_type = read_fourcc_async(
                    file,
                    chunk_payload_offset,
                    spec,
                    "AVI `movi` sub-list type is truncated",
                )
                .await?;
                if list_type == RECL {
                    ranges.push((chunk_payload_offset + 4, chunk_end));
                }
            } else if let Some(stream_index) = parse_stream_chunk_index(chunk_type)
                && stream_index < track_count
            {
                track_chunks[stream_index].push(AviChunkSpan {
                    data_offset: chunk_payload_offset,
                    data_size: u32::try_from(chunk_size)
                        .map_err(|_| MuxError::LayoutOverflow("AVI chunk size"))?,
                });
            }
            offset = next_riff_offset(chunk_end);
        }
    }
    Ok(())
}

fn validate_avi_header_sync(file: &mut File, file_size: u64, spec: &str) -> Result<(), MuxError> {
    if file_size < 12 {
        return Err(invalid_avi(
            spec,
            "AVI input is truncated before the 12-byte RIFF header",
        ));
    }
    let mut header = [0_u8; 12];
    read_exact_at_sync(
        file,
        0,
        &mut header,
        spec,
        "AVI input is truncated before the 12-byte RIFF header",
    )?;
    validate_avi_header_bytes(&header, file_size, spec)
}

#[cfg(feature = "async")]
async fn validate_avi_header_async(
    file: &mut TokioFile,
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if file_size < 12 {
        return Err(invalid_avi(
            spec,
            "AVI input is truncated before the 12-byte RIFF header",
        ));
    }
    let mut header = [0_u8; 12];
    read_exact_at_async(
        file,
        0,
        &mut header,
        spec,
        "AVI input is truncated before the 12-byte RIFF header",
    )
    .await?;
    validate_avi_header_bytes(&header, file_size, spec)
}

fn validate_avi_header_bytes(
    header: &[u8; 12],
    file_size: u64,
    spec: &str,
) -> Result<(), MuxError> {
    if &header[..4] != RIFF {
        return Err(invalid_avi(
            spec,
            "AVI input did not start with the `RIFF` signature",
        ));
    }
    if FourCc::from_bytes(header[8..12].try_into().unwrap()) != AVI_FORM {
        return Err(invalid_avi(
            spec,
            "AVI input did not carry the `AVI ` RIFF form type",
        ));
    }
    let declared_size = u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap())) + 8;
    if declared_size > file_size {
        return Err(invalid_avi(
            spec,
            &format!(
                "AVI RIFF size field declared {declared_size} bytes but the file only contains {file_size}"
            ),
        ));
    }
    Ok(())
}

fn read_riff_chunk_header_sync(
    file: &mut File,
    file_end: u64,
    offset: u64,
    spec: &str,
) -> Result<(FourCc, u64, u64, u64), MuxError> {
    if file_end - offset < 8 {
        return Err(invalid_avi(spec, "AVI chunk header is truncated"));
    }
    let mut header = [0_u8; 8];
    read_exact_at_sync(
        file,
        offset,
        &mut header,
        spec,
        "AVI chunk header is truncated",
    )?;
    decode_riff_chunk_header(offset, file_end, header, spec)
}

#[cfg(feature = "async")]
async fn read_riff_chunk_header_async(
    file: &mut TokioFile,
    file_end: u64,
    offset: u64,
    spec: &str,
) -> Result<(FourCc, u64, u64, u64), MuxError> {
    if file_end - offset < 8 {
        return Err(invalid_avi(spec, "AVI chunk header is truncated"));
    }
    let mut header = [0_u8; 8];
    read_exact_at_async(
        file,
        offset,
        &mut header,
        spec,
        "AVI chunk header is truncated",
    )
    .await?;
    decode_riff_chunk_header(offset, file_end, header, spec)
}

fn decode_riff_chunk_header(
    offset: u64,
    file_end: u64,
    header: [u8; 8],
    spec: &str,
) -> Result<(FourCc, u64, u64, u64), MuxError> {
    let chunk_type = FourCc::from_bytes(header[0..4].try_into().unwrap());
    let chunk_size = u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap()));
    let chunk_payload_offset = offset + 8;
    let chunk_end = chunk_payload_offset
        .checked_add(chunk_size)
        .ok_or(MuxError::LayoutOverflow("AVI chunk range"))?;
    if chunk_end > file_end {
        return Err(invalid_avi(
            spec,
            &format!("AVI chunk `{chunk_type}` overruns the input length"),
        ));
    }
    Ok((chunk_type, chunk_size, chunk_payload_offset, chunk_end))
}

fn read_fourcc_sync(
    file: &mut File,
    offset: u64,
    spec: &str,
    truncated_message: &'static str,
) -> Result<FourCc, MuxError> {
    let mut bytes = [0_u8; 4];
    read_exact_at_sync(file, offset, &mut bytes, spec, truncated_message)?;
    Ok(FourCc::from_bytes(bytes))
}

#[cfg(feature = "async")]
async fn read_fourcc_async(
    file: &mut TokioFile,
    offset: u64,
    spec: &str,
    truncated_message: &'static str,
) -> Result<FourCc, MuxError> {
    let mut bytes = [0_u8; 4];
    read_exact_at_async(file, offset, &mut bytes, spec, truncated_message).await?;
    Ok(FourCc::from_bytes(bytes))
}

fn next_riff_offset(chunk_end: u64) -> u64 {
    chunk_end + (chunk_end & 1)
}

fn parse_stream_chunk_index(chunk_type: FourCc) -> Option<usize> {
    let bytes = chunk_type.into_bytes();
    if !bytes[0].is_ascii_digit() || !bytes[1].is_ascii_digit() {
        return None;
    }
    Some(usize::from(bytes[0] - b'0') * 10 + usize::from(bytes[1] - b'0'))
}

fn build_avi_segmented_source_spec(
    path: &Path,
    chunks: &[AviChunkSpan],
) -> Result<SegmentedMuxSourceSpec, MuxError> {
    let mut segments = Vec::with_capacity(chunks.len());
    let mut logical_offset = 0_u64;
    for chunk in chunks {
        segments.push(SegmentedMuxSourceSegment {
            logical_offset,
            data: super::super::import::SegmentedMuxSourceSegmentData::FileRange {
                source_offset: chunk.data_offset,
                size: chunk.data_size,
            },
        });
        logical_offset = logical_offset
            .checked_add(u64::from(chunk.data_size))
            .ok_or(MuxError::LayoutOverflow("AVI segmented logical size"))?;
    }
    Ok(SegmentedMuxSourceSpec {
        path: path.to_path_buf(),
        segments,
        total_size: logical_offset,
    })
}

fn candidate_samples_from_staged(samples: Vec<StagedSample>) -> Vec<CandidateSample> {
    samples
        .into_iter()
        .map(|sample| CandidateSample {
            source_index: 0,
            data_offset: sample.data_offset,
            data_size: sample.data_size,
            duration: sample.duration,
            composition_time_offset: sample.composition_time_offset,
            is_sync_sample: sample.is_sync_sample,
        })
        .collect()
}

fn avi_mp4v_chunk_is_sync_sample(
    spec: &str,
    stream_index: u32,
    bytes: &[u8],
) -> Result<bool, MuxError> {
    for offset in 0..bytes.len().saturating_sub(4) {
        if bytes[offset..].starts_with(&[0x00, 0x00, 0x01, 0xB6]) {
            let Some(header) = bytes.get(offset + 4) else {
                return Err(invalid_avi(
                    spec,
                    &format!(
                        "AVI video stream {stream_index} carried a truncated MPEG-4 Part 2 VOP header"
                    ),
                ));
            };
            return Ok((header >> 6) == 0);
        }
    }
    Err(invalid_avi(
        spec,
        &format!(
            "AVI video stream {stream_index} did not expose one MPEG-4 Part 2 VOP start code in its chunk payload"
        ),
    ))
}

fn invalid_avi(spec: &str, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: message.to_string(),
    }
}

fn unsupported_avi_audio_format_tag_message(
    stream_index: u32,
    audio_format: AviAudioFormat,
) -> String {
    format!(
        "AVI audio stream {stream_index} uses unsupported WAVE format tag 0x{:04X} (channels={}, sample_rate={}, bits_per_sample={}, block_align={}); native direct-ingest currently accepts {SUPPORTED_AVI_AUDIO_TAGS}",
        audio_format.format_tag,
        audio_format.channel_count,
        audio_format.sample_rate,
        audio_format.bits_per_sample,
        audio_format.block_align,
    )
}

#[cfg(test)]
mod tests {
    use super::avi_video_timing;

    #[test]
    fn avi_video_timing_matches_expected_import_style_rates() {
        let exact = avi_video_timing(1, 25);
        assert_eq!(exact.timescale, 25_000);
        assert_eq!(exact.sample_duration, 1_000);

        let ntsc = avi_video_timing(1_001, 30_000);
        assert_eq!(ntsc.timescale, 30_000);
        assert_eq!(ntsc.sample_duration, 1_001);

        let film = avi_video_timing(1_001, 24_000);
        assert_eq!(film.timescale, 24_000);
        assert_eq!(film.sample_duration, 1_001);
    }
}
