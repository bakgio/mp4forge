use crate::FourCc;

use super::super::MuxRawCodec;
use super::dash::looks_like_dash_manifest_path;
use super::iamf::looks_like_iamf_prefix;
use super::vobsub::looks_like_vobsub_prefix;

const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const STYP: FourCc = FourCc::from_bytes(*b"styp");
const FREE: FourCc = FourCc::from_bytes(*b"free");
const SKIP: FourCc = FourCc::from_bytes(*b"skip");
const WIDE: FourCc = FourCc::from_bytes(*b"wide");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const NON_CORE_DTS_IMPORT_ONLY_FAMILY: &str = "non-core DTS-family audio; native raw direct-ingest currently supports big-endian core DTS sync frames, little-endian core DTS sync frames, transformed 14-bit core DTS sync frames, and DTS-family wrappers that expose one contiguous core substream";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mux) enum DetectedPathTrackKind {
    Mp4,
    Container(DetectedContainerPathKind),
    Raw(MuxRawCodec),
    Mp4ImportOnly(&'static str),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mux) enum DetectedContainerPathKind {
    Avi,
    Dash,
    Ghi,
    Gsf,
    Nhml,
    Nhnt,
    ProgramStream,
    Saf,
    TransportStream,
    VobSub,
}

pub(in crate::mux) fn detect_id3_wrapped_audio_from_prefix(
    prefix: &[u8],
    id3_offset: usize,
) -> Option<DetectedPathTrackKind> {
    if looks_like_mp3_prefix(prefix, id3_offset) {
        return Some(DetectedPathTrackKind::Raw(MuxRawCodec::Mp3));
    }
    if looks_like_adts_prefix(prefix, id3_offset) {
        return Some(DetectedPathTrackKind::Raw(MuxRawCodec::Aac));
    }
    None
}

pub(in crate::mux) fn detect_path_track_kind_from_prefix(prefix: &[u8]) -> DetectedPathTrackKind {
    if looks_like_mp4_prefix(prefix) {
        return DetectedPathTrackKind::Mp4;
    }
    if looks_like_avi_prefix(prefix) {
        return DetectedPathTrackKind::Container(DetectedContainerPathKind::Avi);
    }
    if looks_like_transport_stream_prefix(prefix) {
        return DetectedPathTrackKind::Container(DetectedContainerPathKind::TransportStream);
    }
    if looks_like_program_stream_prefix(prefix) {
        return DetectedPathTrackKind::Container(DetectedContainerPathKind::ProgramStream);
    }
    if looks_like_vobsub_prefix(prefix) {
        return DetectedPathTrackKind::Container(DetectedContainerPathKind::VobSub);
    }
    let id3_offset = id3v2_size_from_prefix(prefix).unwrap_or(0);
    if let Some(kind) = detect_id3_wrapped_audio_from_prefix(prefix, id3_offset) {
        return kind;
    }
    if looks_like_truehd_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Truehd);
    }
    if let Some(kind) = detect_dolby_audio_prefix(prefix) {
        return kind;
    }
    if let Some(kind) = detect_amr_prefix(prefix) {
        return kind;
    }
    if looks_like_qcp_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Qcp);
    }
    if looks_like_jpeg_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Jpeg);
    }
    if looks_like_png_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Png);
    }
    if looks_like_bmp_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Bmp);
    }
    if looks_like_prores_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Prores);
    }
    if looks_like_y4m_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Y4m);
    }
    if looks_like_j2k_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::J2k);
    }
    if looks_like_latm_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Latm);
    }
    if looks_like_pcm_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Pcm);
    }
    if let Some(kind) = detect_dts_prefix(prefix) {
        return kind;
    }
    if looks_like_mhas_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::MpegH);
    }
    if looks_like_iamf_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Iamf);
    }
    if looks_like_h263_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::H263);
    }
    if looks_like_mp4v_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Mp4v);
    }
    if looks_like_mpeg2v_prefix(prefix) {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Mpeg2v);
    }
    if let Some(kind) = detect_annex_b_video_prefix(prefix) {
        return kind;
    }
    if prefix.starts_with(b"fLaC") {
        return DetectedPathTrackKind::Raw(MuxRawCodec::Flac);
    }
    if let Some(kind) = detect_ivf_prefix(prefix) {
        return kind;
    }
    DetectedPathTrackKind::Unknown
}

pub(in crate::mux) fn detect_container_path_kind_from_path_and_prefix(
    path: &std::path::Path,
    prefix: &[u8],
) -> Option<DetectedContainerPathKind> {
    if looks_like_ghi_path(path, prefix) {
        return Some(DetectedContainerPathKind::Ghi);
    }
    if looks_like_gsf_path(path, prefix) {
        return Some(DetectedContainerPathKind::Gsf);
    }
    if looks_like_dash_manifest_path(path, prefix) {
        return Some(DetectedContainerPathKind::Dash);
    }
    if looks_like_saf_path(path) {
        return Some(DetectedContainerPathKind::Saf);
    }
    None
}

fn looks_like_ghi_path(path: &std::path::Path, prefix: &[u8]) -> bool {
    if prefix.starts_with(b"GHID") {
        return true;
    }
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    extension.eq_ignore_ascii_case("ghi") || extension.eq_ignore_ascii_case("ghix")
}

fn looks_like_gsf_path(path: &std::path::Path, prefix: &[u8]) -> bool {
    if prefix.starts_with(b"GS5F") {
        return true;
    }
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    extension.eq_ignore_ascii_case("gsf")
}

fn looks_like_saf_path(path: &std::path::Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    extension.eq_ignore_ascii_case("saf") || extension.eq_ignore_ascii_case("lsr")
}

fn looks_like_avi_prefix(prefix: &[u8]) -> bool {
    prefix.len() >= 12 && &prefix[..4] == b"RIFF" && &prefix[8..12] == b"AVI "
}

fn looks_like_program_stream_prefix(prefix: &[u8]) -> bool {
    prefix.len() >= 4 && prefix[..4] == [0x00, 0x00, 0x01, 0xBA]
}

fn looks_like_transport_stream_prefix(prefix: &[u8]) -> bool {
    prefix.len() >= 376 && prefix[0] == 0x47 && prefix[188] == 0x47
}

fn looks_like_bmp_prefix(prefix: &[u8]) -> bool {
    prefix.len() >= 2 && prefix[0] == b'B' && prefix[1] == b'M'
}

fn looks_like_prores_prefix(prefix: &[u8]) -> bool {
    if prefix.len() < 8 {
        return false;
    }
    let frame_size = u32::from_be_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]);
    frame_size >= 28 && &prefix[4..8] == b"icpf"
}

fn looks_like_y4m_prefix(prefix: &[u8]) -> bool {
    prefix.starts_with(b"YUV4MPEG2 ")
}

fn looks_like_j2k_prefix(prefix: &[u8]) -> bool {
    if prefix.len() >= 12
        && prefix[..4] == 12_u32.to_be_bytes()
        && &prefix[4..8] == b"jP  "
        && prefix[8..12] == [0x0D, 0x0A, 0x87, 0x0A]
    {
        return true;
    }
    prefix.len() >= 4
        && prefix[0] == 0xFF
        && prefix[1] == 0x4F
        && prefix[2] == 0xFF
        && prefix[3] == 0x51
}

pub(in crate::mux) fn id3v2_size_from_prefix(prefix: &[u8]) -> Option<usize> {
    if prefix.len() < 10 || &prefix[..3] != b"ID3" {
        return None;
    }
    let size = [prefix[6], prefix[7], prefix[8], prefix[9]];
    if size.iter().any(|byte| byte & 0x80 != 0) {
        return None;
    }
    Some(
        10 + ((usize::from(size[0]) & 0x7F) << 21)
            + ((usize::from(size[1]) & 0x7F) << 14)
            + ((usize::from(size[2]) & 0x7F) << 7)
            + (usize::from(size[3]) & 0x7F),
    )
}

fn looks_like_mp4_prefix(prefix: &[u8]) -> bool {
    let mut offset = 0usize;
    for _ in 0..4 {
        if prefix.len().saturating_sub(offset) < 8 {
            return false;
        }
        let size = u32::from_be_bytes([
            prefix[offset],
            prefix[offset + 1],
            prefix[offset + 2],
            prefix[offset + 3],
        ]);
        let box_type = FourCc::from_bytes([
            prefix[offset + 4],
            prefix[offset + 5],
            prefix[offset + 6],
            prefix[offset + 7],
        ]);
        if matches!(box_type, FTYP | STYP | MOOV | MOOF | MDAT) {
            return true;
        }
        if !matches!(box_type, FREE | SKIP | WIDE) {
            return false;
        }
        let size = match size {
            0 => return false,
            1 => {
                if prefix.len().saturating_sub(offset) < 16 {
                    return false;
                }
                u64::from_be_bytes([
                    prefix[offset + 8],
                    prefix[offset + 9],
                    prefix[offset + 10],
                    prefix[offset + 11],
                    prefix[offset + 12],
                    prefix[offset + 13],
                    prefix[offset + 14],
                    prefix[offset + 15],
                ]) as usize
            }
            value => value as usize,
        };
        if size < 8 {
            return false;
        }
        offset = match offset.checked_add(size) {
            Some(value) => value,
            None => return false,
        };
        if offset >= prefix.len() {
            break;
        }
    }
    false
}

fn looks_like_adts_prefix(prefix: &[u8], offset: usize) -> bool {
    if prefix.len().saturating_sub(offset) < 7 {
        return false;
    }
    prefix[offset] == 0xFF
        && prefix[offset + 1] & 0xF0 == 0xF0
        && ((prefix[offset + 2] >> 2) & 0x0F) < 13
}

fn looks_like_mp3_prefix(prefix: &[u8], offset: usize) -> bool {
    if prefix.len().saturating_sub(offset) < 4 {
        return false;
    }
    let header = u32::from_be_bytes([
        prefix[offset],
        prefix[offset + 1],
        prefix[offset + 2],
        prefix[offset + 3],
    ]);
    let sync = (header >> 21) & 0x07FF;
    let layer = (header >> 17) & 0x03;
    let bitrate_index = (header >> 12) & 0x0F;
    let sample_rate_index = (header >> 10) & 0x03;
    sync == 0x07FF
        && layer != 0
        && bitrate_index != 0
        && bitrate_index != 0x0F
        && sample_rate_index != 0x03
}

fn looks_like_pcm_prefix(prefix: &[u8]) -> bool {
    prefix.len() >= 12
        && ((&prefix[..4] == b"RIFF" && &prefix[8..12] == b"WAVE")
            || (&prefix[..4] == b"FORM"
                && (&prefix[8..12] == b"AIFF" || &prefix[8..12] == b"AIFC")))
}

fn looks_like_qcp_prefix(prefix: &[u8]) -> bool {
    prefix.len() >= 12 && &prefix[..4] == b"RIFF" && &prefix[8..12] == b"QLCM"
}

fn looks_like_jpeg_prefix(prefix: &[u8]) -> bool {
    prefix.len() >= 3 && prefix[0] == 0xFF && prefix[1] == 0xD8 && prefix[2] == 0xFF
}

fn looks_like_png_prefix(prefix: &[u8]) -> bool {
    prefix.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
}

fn looks_like_latm_prefix(prefix: &[u8]) -> bool {
    prefix.len() >= 3 && prefix[0] == 0x56 && (prefix[1] >> 5) == 0x07
}

fn looks_like_truehd_prefix(prefix: &[u8]) -> bool {
    const TRUEHD_SYNC: [u8; 4] = [0xF8, 0x72, 0x6F, 0xBA];
    const TRUEHD_SIGNATURE: [u8; 2] = [0xB7, 0x52];

    if prefix.len() < 20 {
        return false;
    }
    for offset in 0..=prefix.len() - 20 {
        if prefix[offset + 4..offset + 8] != TRUEHD_SYNC {
            continue;
        }
        if prefix[offset + 12..offset + 14] != TRUEHD_SIGNATURE {
            continue;
        }
        let packed = u16::from_be_bytes([prefix[offset], prefix[offset + 1]]);
        let frame_size = u32::from(packed & 0x0FFF) * 2;
        if frame_size < 20 {
            continue;
        }
        let format_info = u32::from_be_bytes([
            prefix[offset + 8],
            prefix[offset + 9],
            prefix[offset + 10],
            prefix[offset + 11],
        ]);
        let sample_rate_code = ((format_info >> 28) & 0x0F) as u8;
        if matches!(sample_rate_code, 0 | 1 | 2 | 8 | 9 | 10) {
            return true;
        }
    }
    false
}

fn detect_amr_prefix(prefix: &[u8]) -> Option<DetectedPathTrackKind> {
    if prefix.starts_with(b"#!AMR\n") {
        return Some(DetectedPathTrackKind::Raw(MuxRawCodec::Amr));
    }
    if prefix.starts_with(b"#!AMR-WB\n") {
        return Some(DetectedPathTrackKind::Raw(MuxRawCodec::AmrWb));
    }
    None
}

fn detect_dolby_audio_prefix(prefix: &[u8]) -> Option<DetectedPathTrackKind> {
    if prefix.len() < 6 || prefix[0] != 0x0B || prefix[1] != 0x77 {
        if prefix.len() >= 2 {
            let syncword = u16::from_be_bytes([prefix[0], prefix[1]]);
            if syncword == 0xAC40 || syncword == 0xAC41 {
                return Some(DetectedPathTrackKind::Raw(MuxRawCodec::Ac4));
            }
        }
        return None;
    }
    let bsid = (prefix[5] >> 3) & 0x1F;
    if bsid <= 10 {
        Some(DetectedPathTrackKind::Raw(MuxRawCodec::Ac3))
    } else if bsid <= 16 {
        Some(DetectedPathTrackKind::Raw(MuxRawCodec::Eac3))
    } else {
        None
    }
}

fn detect_annex_b_video_prefix(prefix: &[u8]) -> Option<DetectedPathTrackKind> {
    let mut index = 0usize;
    while index + 4 <= prefix.len() {
        let start_code_len = if prefix[index..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if prefix[index..].starts_with(&[0, 0, 1]) {
            3
        } else {
            index += 1;
            continue;
        };
        let header_index = index + start_code_len;
        let &nal_header = prefix.get(header_index)?;
        let h264_type = nal_header & 0x1F;
        if matches!(h264_type, 1..=5 | 7..=9) {
            return Some(DetectedPathTrackKind::Raw(MuxRawCodec::H264));
        }
        let h265_type = (nal_header >> 1) & 0x3F;
        if matches!(h265_type, 16..=21 | 32..=35) {
            return Some(DetectedPathTrackKind::Raw(MuxRawCodec::H265));
        }
        let Some(&vvc_header_byte1) = prefix.get(header_index + 1) else {
            index = header_index + 1;
            continue;
        };
        let vvc_type = vvc_header_byte1 >> 3;
        if matches!(vvc_type, 7..=10 | 14..=17 | 20) {
            return Some(DetectedPathTrackKind::Raw(MuxRawCodec::Vvc));
        }
        index = header_index + 1;
    }
    None
}

fn detect_ivf_prefix(prefix: &[u8]) -> Option<DetectedPathTrackKind> {
    if prefix.len() < 32 || &prefix[..4] != b"DKIF" {
        return None;
    }
    match &prefix[8..12] {
        b"AV01" => Some(DetectedPathTrackKind::Raw(MuxRawCodec::Av1)),
        b"VP80" => Some(DetectedPathTrackKind::Raw(MuxRawCodec::Vp8)),
        b"VP90" => Some(DetectedPathTrackKind::Raw(MuxRawCodec::Vp9)),
        b"VP10" => Some(DetectedPathTrackKind::Raw(MuxRawCodec::Vp10)),
        _ => Some(DetectedPathTrackKind::Unknown),
    }
}

fn looks_like_h263_prefix(prefix: &[u8]) -> bool {
    if prefix.len() < 5 {
        return false;
    }
    if (u32::from_be_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]) >> 10) != 0x20 {
        return false;
    }
    matches!((prefix[4] >> 2) & 0x07, 1..=5)
}

fn looks_like_mpeg2v_prefix(prefix: &[u8]) -> bool {
    let mut saw_sequence_header = false;
    let mut saw_picture = false;
    let mut index = 0usize;
    while index + 4 <= prefix.len() {
        if prefix[index..].starts_with(&[0x00, 0x00, 0x01]) {
            match prefix[index + 3] {
                0xB3 => saw_sequence_header = true,
                0x00 => saw_picture = true,
                _ => {}
            }
            if saw_sequence_header && saw_picture {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn looks_like_mp4v_prefix(prefix: &[u8]) -> bool {
    let mut saw_vop = false;
    let mut saw_config = false;
    let mut index = 0usize;
    while index + 4 <= prefix.len() {
        if prefix[index..].starts_with(&[0x00, 0x00, 0x01]) {
            let start_code = prefix[index + 3];
            if start_code == 0xB6 {
                saw_vop = true;
            } else if start_code == 0xB0
                || start_code == 0xB5
                || (0x20..=0x2F).contains(&start_code)
            {
                saw_config = true;
            }
            if saw_vop && saw_config {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn looks_like_mhas_prefix(prefix: &[u8]) -> bool {
    prefix.starts_with(&[0xC0, 0x01, 0xA5])
}

fn detect_dts_prefix(prefix: &[u8]) -> Option<DetectedPathTrackKind> {
    if prefix.starts_with(&[0x7F, 0xFE, 0x80, 0x01])
        || prefix.starts_with(&[0xFE, 0x7F, 0x01, 0x80])
        || prefix.starts_with(&[0x1F, 0xFF, 0xE8, 0x00])
        || prefix.starts_with(&[0xFF, 0x1F, 0x00, 0xE8])
    {
        return Some(DetectedPathTrackKind::Raw(MuxRawCodec::Dts));
    }
    if prefix.starts_with(b"DTSHDHDR") {
        if dts_wrapper_prefix_exposes_native_core_sync(prefix) {
            return Some(DetectedPathTrackKind::Raw(MuxRawCodec::Dts));
        }
        return Some(DetectedPathTrackKind::Mp4ImportOnly(
            NON_CORE_DTS_IMPORT_ONLY_FAMILY,
        ));
    }
    None
}

fn dts_wrapper_prefix_exposes_native_core_sync(prefix: &[u8]) -> bool {
    if prefix.len() <= 8 {
        return false;
    }
    prefix[8..].windows(4).any(|window| {
        matches!(
            window,
            [0x7F, 0xFE, 0x80, 0x01]
                | [0xFE, 0x7F, 0x01, 0x80]
                | [0x1F, 0xFF, 0xE8, 0x00]
                | [0xFF, 0x1F, 0x00, 0xE8]
        )
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        DetectedContainerPathKind, DetectedPathTrackKind,
        detect_container_path_kind_from_path_and_prefix, detect_path_track_kind_from_prefix,
    };
    use crate::mux::MuxRawCodec;

    #[test]
    fn dts_wrapper_prefix_with_visible_core_sync_detects_as_native_raw() {
        let mut prefix = b"DTSHDHDRdemo".to_vec();
        prefix.extend_from_slice(&[0x7F, 0xFE, 0x80, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(
            detect_path_track_kind_from_prefix(&prefix),
            DetectedPathTrackKind::Raw(MuxRawCodec::Dts)
        );
    }

    #[test]
    fn dts_wrapper_prefix_without_visible_core_sync_stays_import_only() {
        assert!(matches!(
            detect_path_track_kind_from_prefix(b"DTSHDHDRdemo"),
            DetectedPathTrackKind::Mp4ImportOnly(_)
        ));
    }

    #[test]
    fn gsf_signature_detects_as_container_path() {
        assert_eq!(
            detect_container_path_kind_from_path_and_prefix(Path::new("demo.bin"), b"GS5F\x01demo"),
            Some(DetectedContainerPathKind::Gsf)
        );
    }

    #[test]
    fn ghi_extension_detects_as_container_path() {
        assert_eq!(
            detect_container_path_kind_from_path_and_prefix(Path::new("demo.ghix"), b"<MPD"),
            Some(DetectedContainerPathKind::Ghi)
        );
    }

    #[test]
    fn ambiguous_mpeg4_part2_prefix_prefers_mp4v_detection() {
        let prefix = [
            0x00, 0x00, 0x01, 0xB0, 0x01, 0x00, 0x00, 0x01, 0xB5, 0x89, 0x13, 0x00, 0x00, 0x01,
            0x00, 0x00, 0x00, 0x01, 0x20, 0x00, 0xC4, 0x8D, 0x88, 0x00, 0xCD, 0x0A, 0x04, 0x1E,
            0x14, 0x43, 0x00, 0x00, 0x01, 0xB2, 0x4C, 0x61, 0x76, 0x63, 0x36, 0x31, 0x2E, 0x32,
            0x2E, 0x31, 0x30, 0x30, 0x00, 0x00, 0x01, 0xB3, 0x00, 0x10, 0x07, 0x00, 0x00, 0x01,
            0xB6, 0x10, 0x60, 0xB1, 0x82, 0x99, 0xB7, 0xF1,
        ];
        assert_eq!(
            detect_path_track_kind_from_prefix(&prefix),
            DetectedPathTrackKind::Raw(MuxRawCodec::Mp4v)
        );
    }
}
