//! Private mux-ingest detectors and codec-focused direct parsers.
//!
//! The public mux surface stays path-first under [`crate::mux`], while this internal tree owns
//! the codec and container-specific detection and parsing work needed to turn those paths into
//! truthful staged tracks.

mod aac;
mod ac3;
mod ac4;
mod alac;
mod amr;
mod annexb_common;
mod av1;
mod avi;
mod caf_common;
mod container_common;
pub(super) mod detect;
mod dts;
mod eac3;
mod flac;
mod h263;
mod h264;
mod h265;
mod iamf;
mod ivf_common;
mod jpeg;
mod latm;
mod mhas;
mod mp3;
mod mp4v;
mod ogg_common;
mod opus;
mod pcm;
mod png;
mod ps;
mod qcp;
mod speex;
mod theora;
mod truehd;
mod ts;
mod vobsub;
mod vorbis;
mod vp10;
mod vp8;
mod vp9;
mod vvc;

#[cfg(feature = "async")]
pub(super) use aac::scan_adts_file_async;
pub(super) use aac::scan_adts_file_sync;
#[cfg(feature = "async")]
pub(super) use ac3::scan_ac3_file_async;
pub(super) use ac3::scan_ac3_file_sync;
#[cfg(feature = "async")]
pub(super) use ac4::scan_ac4_file_async;
pub(super) use ac4::scan_ac4_file_sync;
#[cfg(feature = "async")]
pub(super) use alac::scan_caf_alac_file_async;
pub(super) use alac::scan_caf_alac_file_sync;
#[cfg(feature = "async")]
pub(super) use amr::{scan_amr_file_async, scan_amr_wb_file_async};
pub(super) use amr::{scan_amr_file_sync, scan_amr_wb_file_sync};
#[cfg(feature = "async")]
pub(super) use av1::scan_av1_file_async;
pub(super) use av1::scan_av1_file_sync;
#[cfg(feature = "async")]
pub(super) use avi::scan_avi_source_async;
pub(super) use avi::scan_avi_source_sync;
#[cfg(feature = "async")]
pub(super) use caf_common::detect_caf_track_kind_async;
pub(super) use caf_common::detect_caf_track_kind_sync;
pub(super) use detect::{
    DetectedContainerPathKind, DetectedPathTrackKind, detect_id3_wrapped_audio_from_prefix,
    detect_path_track_kind_from_prefix, id3v2_size_from_prefix,
};
#[cfg(feature = "async")]
pub(super) use dts::scan_dts_file_async;
pub(super) use dts::scan_dts_file_sync;
#[cfg(feature = "async")]
pub(super) use eac3::scan_eac3_file_async;
pub(super) use eac3::scan_eac3_file_sync;
#[cfg(feature = "async")]
pub(super) use flac::scan_flac_file_async;
#[cfg(feature = "async")]
pub(super) use flac::scan_ogg_flac_file_async;
pub(super) use flac::{scan_flac_file_sync, scan_ogg_flac_file_sync};
#[cfg(feature = "async")]
pub(super) use h263::scan_h263_file_async;
pub(super) use h263::scan_h263_file_sync;
#[cfg(feature = "async")]
pub(super) use h264::stage_annex_b_h264_async;
pub(super) use h264::stage_annex_b_h264_sync;
#[cfg(feature = "async")]
pub(super) use h265::stage_annex_b_h265_async;
pub(super) use h265::stage_annex_b_h265_sync;
#[cfg(feature = "async")]
pub(super) use iamf::scan_iamf_file_async;
pub(super) use iamf::scan_iamf_file_sync;
#[cfg(feature = "async")]
pub(super) use jpeg::scan_jpeg_file_async;
pub(super) use jpeg::scan_jpeg_file_sync;
#[cfg(feature = "async")]
pub(super) use latm::scan_latm_file_async;
pub(super) use latm::scan_latm_file_sync;
#[cfg(feature = "async")]
pub(super) use mhas::scan_mhas_file_async;
pub(super) use mhas::scan_mhas_file_sync;
#[cfg(feature = "async")]
pub(super) use mp3::scan_mp3_file_async;
pub(super) use mp3::scan_mp3_file_sync;
#[cfg(feature = "async")]
pub(super) use mp4v::scan_mp4v_file_async;
pub(super) use mp4v::scan_mp4v_file_sync;
#[cfg(feature = "async")]
pub(super) use ogg_common::detect_ogg_track_kind_async;
pub(super) use ogg_common::detect_ogg_track_kind_sync;
#[cfg(feature = "async")]
pub(super) use opus::scan_ogg_opus_file_async;
pub(super) use opus::scan_ogg_opus_file_sync;
#[cfg(feature = "async")]
pub(super) use pcm::scan_pcm_file_async;
pub(super) use pcm::scan_pcm_file_sync;
#[cfg(feature = "async")]
pub(super) use png::scan_png_file_async;
pub(super) use png::scan_png_file_sync;
#[cfg(feature = "async")]
pub(super) use ps::scan_program_stream_async;
pub(super) use ps::scan_program_stream_sync;
#[cfg(feature = "async")]
pub(super) use qcp::scan_qcp_file_async;
pub(super) use qcp::scan_qcp_file_sync;
#[cfg(feature = "async")]
pub(super) use speex::scan_ogg_speex_file_async;
pub(super) use speex::scan_ogg_speex_file_sync;
#[cfg(feature = "async")]
pub(super) use theora::scan_ogg_theora_file_async;
pub(super) use theora::scan_ogg_theora_file_sync;
#[cfg(feature = "async")]
pub(super) use truehd::scan_truehd_file_async;
pub(super) use truehd::scan_truehd_file_sync;
#[cfg(feature = "async")]
pub(super) use ts::scan_transport_stream_async;
pub(super) use ts::scan_transport_stream_sync;
#[cfg(feature = "async")]
pub(super) use vobsub::scan_vobsub_source_async;
pub(super) use vobsub::scan_vobsub_source_sync;
#[cfg(feature = "async")]
pub(super) use vorbis::scan_ogg_vorbis_file_async;
pub(super) use vorbis::scan_ogg_vorbis_file_sync;
#[cfg(feature = "async")]
pub(super) use vp8::scan_vp8_file_async;
pub(super) use vp8::scan_vp8_file_sync;
#[cfg(feature = "async")]
pub(super) use vp9::scan_vp9_file_async;
pub(super) use vp9::scan_vp9_file_sync;
#[cfg(feature = "async")]
pub(super) use vp10::scan_vp10_file_async;
pub(super) use vp10::scan_vp10_file_sync;
#[cfg(feature = "async")]
pub(super) use vvc::stage_annex_b_vvc_async;
pub(super) use vvc::stage_annex_b_vvc_sync;
