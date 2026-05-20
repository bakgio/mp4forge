#![allow(dead_code)]
#![allow(clippy::field_reassign_with_default)]

#[cfg(feature = "decrypt")]
use std::collections::BTreeMap;
use std::fs;
#[cfg(feature = "decrypt")]
use std::io::{Cursor, Seek};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "decrypt")]
use aes::Aes128;
#[cfg(feature = "decrypt")]
use aes::cipher::{Block, BlockEncrypt, KeyInit};
#[cfg(feature = "mux")]
use mp4forge::bitio::BitWriter;
use mp4forge::boxes::AnyTypeBox;
#[cfg(feature = "decrypt")]
use mp4forge::boxes::isma_cryp::{Isfm, Islt};
use mp4forge::boxes::iso14496_12::{
    AVCDecoderConfiguration, Btrt, Emeb, Emib, EventMessageSampleEntry, Frma, Ftyp, Hdlr, Mdhd,
    Mdia, Mfhd, Minf, Moof, Moov, Mvex, Mvhd, Pasp, Saio, Saiz, SampleEntry, Sbgp, SbgpEntry, Schi,
    Schm, SeigEntry, SeigEntryL, Sgpd, Silb, SilbEntry, Sinf, Stbl, Stco, Stsc, Stsd, Stsz, Stts,
    TFHD_DEFAULT_SAMPLE_DURATION_PRESENT, TFHD_DEFAULT_SAMPLE_SIZE_PRESENT, Tfdt, Tfhd, Traf, Trak,
    Trex, Trun, VisualSampleEntry,
};
#[cfg(feature = "decrypt")]
use mp4forge::boxes::iso14496_12::{StscEntry, UUID_SAMPLE_ENCRYPTION, Uuid, UuidPayload};
#[cfg(feature = "decrypt")]
use mp4forge::boxes::iso14496_12::{
    TFHD_DEFAULT_BASE_IS_MOOF, TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT, TRUN_DATA_OFFSET_PRESENT,
};
#[cfg(feature = "decrypt")]
use mp4forge::boxes::iso14496_14::Iods;
use mp4forge::boxes::iso23001_7::{
    SENC_USE_SUBSAMPLE_ENCRYPTION, Senc, SencSample, SencSubsample, Tenc,
};
#[cfg(feature = "decrypt")]
use mp4forge::boxes::oma_dcf::{
    OHDR_ENCRYPTION_METHOD_AES_CTR, OHDR_PADDING_SCHEME_NONE, Odaf, Odkm, Ohdr,
};
use mp4forge::codec::MutableBox;
use mp4forge::codec::{CodecBox, marshal};
#[cfg(feature = "decrypt")]
use mp4forge::decrypt::{DecryptionKey, NativeCommonEncryptionScheme};
#[cfg(feature = "decrypt")]
use mp4forge::encryption::{ResolvedSampleEncryptionSample, ResolvedSampleEncryptionSource};
#[cfg(feature = "decrypt")]
use mp4forge::extract::{extract_box, extract_box_as};
#[cfg(feature = "mux")]
use mp4forge::mux::{
    MuxFileConfig, MuxInterleavePolicy, MuxStagedMediaItem, MuxTrackConfig,
    plan_staged_media_items, write_mp4_mux_to_path,
};
#[cfg(feature = "decrypt")]
use mp4forge::walk::BoxPath;
use mp4forge::{BoxInfo, FourCc};

#[cfg(feature = "mux")]
const TS_PACKET_SIZE: usize = 188;

pub fn encode_supported_box<B>(box_value: &B, children: &[u8]) -> Vec<u8>
where
    B: CodecBox,
{
    let mut payload = Vec::new();
    marshal(&mut payload, box_value, None).unwrap();
    payload.extend_from_slice(children);
    encode_raw_box(box_value.box_type(), &payload)
}

pub fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Vec<u8> {
    let info = BoxInfo::new(box_type, 8 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(payload);
    bytes
}

pub fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).unwrap()
}

pub fn write_temp_file(prefix: &str, data: &[u8]) -> PathBuf {
    write_temp_file_with_extension(prefix, "mp4", data)
}

pub fn write_temp_file_with_extension(prefix: &str, extension: &str, data: &[u8]) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "mp4forge-{prefix}-{}-{unique}.{extension}",
        std::process::id()
    ));
    fs::write(&path, data).unwrap();
    path
}

#[cfg(feature = "mux")]
#[derive(Clone, Copy)]
pub struct TestMuxSample<'a> {
    pub bytes: &'a [u8],
    pub duration: u32,
    pub composition_time_offset: i32,
    pub is_sync_sample: bool,
}

#[cfg(feature = "mux")]
pub fn write_single_track_mp4_input(
    prefix: &str,
    file_config: &MuxFileConfig,
    track_config: MuxTrackConfig,
    samples: &[TestMuxSample<'_>],
) -> PathBuf {
    let source_bytes = samples
        .iter()
        .flat_map(|sample| sample.bytes)
        .copied()
        .collect::<Vec<_>>();
    let source_path = write_temp_file(&format!("{prefix}-source"), &source_bytes);
    let output_path = write_temp_file(&format!("{prefix}-output"), &[]);

    let mut source_offset = 0_u64;
    let mut decode_time = 0_u64;
    let staged_items = samples
        .iter()
        .map(|sample| {
            let item = MuxStagedMediaItem::new(
                0,
                track_config.track_id(),
                decode_time,
                sample.duration,
                source_offset,
                u32::try_from(sample.bytes.len()).unwrap(),
            )
            .with_composition_time_offset(sample.composition_time_offset)
            .with_sync_sample(sample.is_sync_sample);
            source_offset += u64::try_from(sample.bytes.len()).unwrap();
            decode_time += u64::from(sample.duration);
            item
        })
        .collect::<Vec<_>>();
    let plan = plan_staged_media_items(staged_items, MuxInterleavePolicy::DecodeTime).unwrap();

    write_mp4_mux_to_path(
        &[&source_path],
        &output_path,
        file_config,
        &[track_config],
        &plan,
    )
    .unwrap();
    output_path
}

#[cfg(feature = "mux")]
pub fn write_test_adts_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for payload in payloads {
        bytes.extend_from_slice(&build_adts_frame(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
/// Writes one deterministic AAC-LC LATM file for direct-ingest mux tests.
pub fn write_test_latm_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for (index, payload) in payloads.iter().enumerate() {
        bytes.extend_from_slice(&build_test_latm_frame(index != 0, payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
/// Writes one deterministic USAC LATM file for direct-ingest mux tests.
pub fn write_test_usac_latm_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for (index, payload) in payloads.iter().enumerate() {
        bytes.extend_from_slice(&build_test_usac_latm_frame(index != 0, payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_truehd_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let bytes = build_test_truehd_stream_bytes(payloads);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn build_test_truehd_stream_bytes(payloads: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for payload in payloads {
        bytes.extend_from_slice(&build_test_truehd_frame(payload));
    }
    bytes
}

#[cfg(feature = "mux")]
pub fn write_test_mp3_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for payload in payloads {
        bytes.extend_from_slice(&build_mp3_frame(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_mp3_44100_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for payload in payloads {
        bytes.extend_from_slice(&build_mp3_frame_44100(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_mp3_file_with_leading_id3_tag(
    prefix: &str,
    tag_payload: &[u8],
    frame_payloads: &[&[u8]],
) -> PathBuf {
    let mut bytes = build_id3v2_tag(tag_payload);
    for payload in frame_payloads {
        bytes.extend_from_slice(&build_mp3_frame(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_ac3_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for payload in payloads {
        bytes.extend_from_slice(&build_ac3_frame(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_ac3_44100_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for payload in payloads {
        bytes.extend_from_slice(&build_ac3_44100_frame(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_eac3_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for payload in payloads {
        bytes.extend_from_slice(&build_eac3_frame(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_eac3_file_with_dependent_substream(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for payload in payloads {
        bytes.extend_from_slice(&build_eac3_frame(payload));
        bytes.extend_from_slice(&build_eac3_dependent_substream_frame(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_ac4_file(prefix: &str, frame_count: usize) -> PathBuf {
    let bytes = build_test_ac4_stream_bytes(frame_count);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "mp4forge-{prefix}-{}-{unique}.ac4",
        std::process::id()
    ));
    fs::write(&path, bytes).unwrap();
    path
}

#[cfg(feature = "mux")]
fn build_test_ac4_stream_bytes(frame_count: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    let frame = decode_test_hex_bytes(TEST_AC4_FRAME_HEX);
    for _ in 0..frame_count {
        bytes.extend_from_slice(&frame);
        bytes.extend_from_slice(&[0, 0]);
    }
    bytes
}

#[cfg(feature = "mux")]
pub fn build_test_ac4_sample_payload_bytes(frame_count: usize) -> Vec<u8> {
    let frame = decode_test_hex_bytes(TEST_AC4_FRAME_HEX);
    let payload = &frame[7..];
    let mut bytes = Vec::with_capacity(payload.len() * frame_count);
    for _ in 0..frame_count {
        bytes.extend_from_slice(payload);
    }
    bytes
}

#[cfg(feature = "mux")]
pub fn write_test_amr_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"#!AMR\n");
    for payload in payloads {
        bytes.extend_from_slice(&build_test_amr_frame(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_amr_wb_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"#!AMR-WB\n");
    for payload in payloads {
        bytes.extend_from_slice(&build_test_amr_wb_frame(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestQcpCodecKind {
    Qcelp,
    Evrc,
    Smv,
}

#[cfg(feature = "mux")]
pub fn write_test_qcp_constant_file(
    prefix: &str,
    codec: TestQcpCodecKind,
    payloads: &[&[u8]],
) -> PathBuf {
    assert!(!payloads.is_empty());
    let packet_size = u16::try_from(payloads[0].len()).unwrap();
    assert!(packet_size > 0);
    for payload in payloads.iter().skip(1) {
        assert_eq!(payload.len(), usize::from(packet_size));
    }
    let packets = payloads
        .iter()
        .map(|payload| payload.to_vec())
        .collect::<Vec<_>>();
    write_temp_file(
        prefix,
        &build_test_qcp_file_bytes(
            TestQcpFileSpec {
                codec,
                decoder_version: 0,
                packet_size,
                block_size: 160,
                sample_rate: 8_000,
                rate_entries: &[],
                rate_flag: 0,
            },
            &packets,
        ),
    )
}

#[cfg(feature = "mux")]
pub fn write_test_qcp_variable_file(
    prefix: &str,
    codec: TestQcpCodecKind,
    packets: &[(u8, &[u8])],
) -> PathBuf {
    assert!(!packets.is_empty());
    let mut rate_entries = Vec::new();
    for (rate_index, payload) in packets {
        assert!(!payload.is_empty());
        let packet_size = u8::try_from(payload.len()).unwrap();
        if let Some(existing) = rate_entries
            .iter()
            .find(|(existing_index, _)| *existing_index == *rate_index)
        {
            assert_eq!(existing.1, packet_size);
        } else {
            rate_entries.push((*rate_index, packet_size));
        }
    }
    assert!(rate_entries.len() <= 8);
    let packet_bytes = packets
        .iter()
        .map(|(rate_index, payload)| {
            let mut packet = Vec::with_capacity(payload.len() + 1);
            packet.push(*rate_index);
            packet.extend_from_slice(payload);
            packet
        })
        .collect::<Vec<_>>();
    write_temp_file(
        prefix,
        &build_test_qcp_file_bytes(
            TestQcpFileSpec {
                codec,
                decoder_version: 0,
                packet_size: 0,
                block_size: 160,
                sample_rate: 8_000,
                rate_entries: &rate_entries,
                rate_flag: 1,
            },
            &packet_bytes,
        ),
    )
}

#[cfg(feature = "mux")]
pub fn write_test_dts_file(prefix: &str, frame_count: usize) -> PathBuf {
    let mut bytes = Vec::new();
    for index in 0..frame_count {
        bytes.extend_from_slice(&build_dts_frame(index));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_dts_little_endian_file(prefix: &str, frame_count: usize) -> PathBuf {
    let mut bytes = Vec::new();
    for index in 0..frame_count {
        bytes.extend_from_slice(&swap_test_dts_16bit_words(&build_dts_frame(index)));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_wrapped_dts_file(prefix: &str, frame_count: usize) -> PathBuf {
    let mut bytes = b"DTSHDHDR".to_vec();
    for index in 0..frame_count {
        bytes.extend_from_slice(&build_dts_frame(index));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_wrapped_dts_file_with_tail(
    prefix: &str,
    frame_count: usize,
    tail: &[u8],
) -> PathBuf {
    let mut bytes = b"DTSHDHDR".to_vec();
    for index in 0..frame_count {
        bytes.extend_from_slice(&build_dts_frame(index));
    }
    bytes.extend_from_slice(tail);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_dts_14bit_big_endian_file(prefix: &str, frame_count: usize) -> PathBuf {
    let mut bytes = Vec::new();
    for index in 0..frame_count {
        bytes.extend_from_slice(&pack_test_dts_14bit_words(&build_dts_frame(index), false));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_dts_14bit_little_endian_file(prefix: &str, frame_count: usize) -> PathBuf {
    let mut bytes = Vec::new();
    for index in 0..frame_count {
        bytes.extend_from_slice(&pack_test_dts_14bit_words(&build_dts_frame(index), true));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_flac_file(prefix: &str, frame_payload: &[u8]) -> PathBuf {
    write_test_flac_file_with_frames(prefix, &[frame_payload])
}

#[cfg(feature = "mux")]
pub fn write_test_flac_file_with_frames(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    write_test_flac_file_with_frames_and_block_size(prefix, 48_000, 1_024, frame_payloads)
}

#[cfg(feature = "mux")]
/// Writes a deterministic native FLAC file whose authored frame headers expose `block_size` and
/// `sample_rate` directly, so mux tests can model longer retained audio frame timing shapes.
pub fn write_test_flac_file_with_frames_and_block_size(
    prefix: &str,
    sample_rate: u32,
    block_size: u32,
    frame_payloads: &[&[u8]],
) -> PathBuf {
    assert!(!frame_payloads.is_empty());
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"fLaC");
    bytes.push(0x80);
    bytes.extend_from_slice(&34_u32.to_be_bytes()[1..]);
    bytes.extend_from_slice(&build_flac_streaminfo_block(
        sample_rate,
        2,
        16,
        u64::try_from(frame_payloads.len()).unwrap() * u64::from(block_size),
    ));
    for payload in frame_payloads {
        bytes.extend_from_slice(&build_test_flac_frame_with_block_size(payload, block_size));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_ogg_flac_file(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    let serial = 0x464C_4143_u32;
    let mut bytes = Vec::new();
    let mut header_packet = Vec::new();
    header_packet.extend_from_slice(b"fLaC");
    header_packet.push(0x80);
    header_packet.extend_from_slice(&34_u32.to_be_bytes()[1..]);
    header_packet.extend_from_slice(&build_flac_streaminfo_block(
        48_000,
        2,
        16,
        u64::try_from(frame_payloads.len()).unwrap() * 1_024,
    ));
    bytes.extend_from_slice(&build_ogg_page(serial, 0, 0x02, 0, &[header_packet]));
    let mut granule_position = 0_u64;
    for (index, payload) in frame_payloads.iter().enumerate() {
        let frame = build_test_flac_frame(payload);
        granule_position += 1_024;
        let header_type = if index + 1 == frame_payloads.len() {
            0x04
        } else {
            0
        };
        bytes.extend_from_slice(&build_ogg_page(
            serial,
            u32::try_from(index + 2).unwrap(),
            header_type,
            granule_position,
            &[frame],
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_ogg_flac_split_header_file(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    let serial = 0x464C_4144_u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_ogg_page(serial, 0, 0x02, 0, &[b"fLaC".to_vec()]));
    let mut streaminfo_packet = Vec::new();
    streaminfo_packet.push(0x80);
    streaminfo_packet.extend_from_slice(&34_u32.to_be_bytes()[1..]);
    streaminfo_packet.extend_from_slice(&build_flac_streaminfo_block(
        48_000,
        2,
        16,
        u64::try_from(frame_payloads.len()).unwrap() * 1_024,
    ));
    bytes.extend_from_slice(&build_ogg_page(serial, 1, 0, 0, &[streaminfo_packet]));
    let mut granule_position = 0_u64;
    for (index, payload) in frame_payloads.iter().enumerate() {
        let frame = build_test_flac_frame(payload);
        granule_position += 1_024;
        let header_type = if index + 1 == frame_payloads.len() {
            0x04
        } else {
            0
        };
        bytes.extend_from_slice(&build_ogg_page(
            serial,
            u32::try_from(index + 2).unwrap(),
            header_type,
            granule_position,
            &[frame],
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_ogg_flac_mapping_file(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    let serial = 0x4F47_464C_u32;
    let mut bytes = Vec::new();
    let total_samples = u64::try_from(frame_payloads.len()).unwrap() * 1_024;
    let mut header_packet = Vec::new();
    header_packet.push(0x7F);
    header_packet.extend_from_slice(b"FLAC");
    header_packet.push(1);
    header_packet.push(0);
    header_packet.extend_from_slice(&1_u16.to_be_bytes());
    header_packet.extend_from_slice(b"fLaC");
    header_packet.push(0x00);
    header_packet.extend_from_slice(&34_u32.to_be_bytes()[1..]);
    header_packet.extend_from_slice(&build_flac_streaminfo_block(48_000, 2, 16, total_samples));
    bytes.extend_from_slice(&build_ogg_page(serial, 0, 0x02, 0, &[header_packet]));
    bytes.extend_from_slice(&build_ogg_page(
        serial,
        1,
        0,
        0,
        &[build_flac_vorbis_comment_block()],
    ));
    let mut granule_position = 0_u64;
    for (index, payload) in frame_payloads.iter().enumerate() {
        let frame = build_test_flac_frame(payload);
        granule_position += 1_024;
        let header_type = if index + 1 == frame_payloads.len() {
            0x04
        } else {
            0
        };
        bytes.extend_from_slice(&build_ogg_page(
            serial,
            u32::try_from(index + 1).unwrap(),
            header_type,
            granule_position,
            &[frame],
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_ogg_opus_file(prefix: &str, audio_payloads: &[&[u8]]) -> PathBuf {
    let serial = 0x4F50_5553_u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_ogg_page(
        serial,
        0,
        0x02,
        0,
        &[build_opus_head_packet(2)],
    ));
    bytes.extend_from_slice(&build_ogg_page(serial, 1, 0, 0, &[b"OpusTags".to_vec()]));
    let mut granule_position = 0_u64;
    for (index, payload) in audio_payloads.iter().enumerate() {
        let mut packet = vec![0x00];
        packet.extend_from_slice(payload);
        granule_position += 480;
        let header_type = if index + 1 == audio_payloads.len() {
            0x04
        } else {
            0
        };
        bytes.extend_from_slice(&build_ogg_page(
            serial,
            u32::try_from(index + 2).unwrap(),
            header_type,
            granule_position,
            &[packet],
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_wave_pcm_file(prefix: &str, frames: &[[i16; 2]]) -> PathBuf {
    let channel_count = 2_u16;
    let sample_rate = 48_000_u32;
    let bits_per_sample = 16_u16;
    let block_align = channel_count * (bits_per_sample / 8);
    let byte_rate = sample_rate * u32::from(block_align);

    let mut data = Vec::with_capacity(frames.len() * usize::from(block_align));
    for frame in frames {
        for sample in frame {
            data.extend_from_slice(&sample.to_le_bytes());
        }
    }

    let fmt_chunk_size = 16_u32;
    let data_chunk_size = u32::try_from(data.len()).unwrap();
    let riff_size = 4_u32
        .checked_add(8 + fmt_chunk_size)
        .and_then(|value| value.checked_add(8 + data_chunk_size))
        .unwrap();

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&fmt_chunk_size.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channel_count.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&bits_per_sample.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_chunk_size.to_le_bytes());
    bytes.extend_from_slice(&data);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_aiff_pcm_file(prefix: &str, frames: &[[i16; 2]]) -> PathBuf {
    write_test_aiff_like_pcm_file(prefix, frames, None)
}

#[cfg(feature = "mux")]
pub fn write_test_aifc_pcm_file(prefix: &str, frames: &[[i16; 2]]) -> PathBuf {
    write_test_aiff_like_pcm_file(prefix, frames, Some(*b"twos"))
}

#[cfg(feature = "mux")]
pub fn write_test_aifc_float64_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    frames: &[&[f64]],
) -> PathBuf {
    write_test_aifc_float_file(prefix, sample_rate, channel_count, 64, *b"fl64", frames)
}

#[cfg(feature = "mux")]
pub fn write_test_aifc_alaw_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    packets: &[&[u8]],
) -> PathBuf {
    write_test_aifc_companded_file(prefix, sample_rate, channel_count, *b"ALAW", 8, packets)
}

#[cfg(feature = "mux")]
pub fn write_test_aifc_alaw_file_with_declared_bits(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    declared_bits_per_sample: u16,
    packets: &[&[u8]],
) -> PathBuf {
    write_test_aifc_companded_file(
        prefix,
        sample_rate,
        channel_count,
        *b"ALAW",
        declared_bits_per_sample,
        packets,
    )
}

#[cfg(feature = "mux")]
pub fn write_test_aifc_ulaw_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    packets: &[&[u8]],
) -> PathBuf {
    write_test_aifc_companded_file(prefix, sample_rate, channel_count, *b"ULAW", 8, packets)
}

#[cfg(feature = "mux")]
pub fn write_test_aifc_ulaw_file_with_declared_bits(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    declared_bits_per_sample: u16,
    packets: &[&[u8]],
) -> PathBuf {
    write_test_aifc_companded_file(
        prefix,
        sample_rate,
        channel_count,
        *b"ULAW",
        declared_bits_per_sample,
        packets,
    )
}

#[cfg(feature = "mux")]
fn write_test_aifc_companded_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    compression: [u8; 4],
    declared_bits_per_sample: u16,
    packets: &[&[u8]],
) -> PathBuf {
    let data = packets.iter().flat_map(|packet| packet.iter().copied()).collect::<Vec<_>>();
    let sample_frames = u32::try_from(data.len() / usize::from(channel_count)).unwrap();

    let mut comm_payload = Vec::new();
    comm_payload.extend_from_slice(&channel_count.to_be_bytes());
    comm_payload.extend_from_slice(&sample_frames.to_be_bytes());
    comm_payload.extend_from_slice(&declared_bits_per_sample.to_be_bytes());
    comm_payload.extend_from_slice(&encode_aiff_extended_sample_rate(sample_rate));
    comm_payload.extend_from_slice(&compression);

    let mut ssnd_payload = Vec::new();
    ssnd_payload.extend_from_slice(&0_u32.to_be_bytes());
    ssnd_payload.extend_from_slice(&0_u32.to_be_bytes());
    ssnd_payload.extend_from_slice(&data);

    let mut bytes = Vec::new();
    let total_size = 4 + (8 + comm_payload.len()) + (8 + ssnd_payload.len());
    bytes.extend_from_slice(b"FORM");
    bytes.extend_from_slice(&u32::try_from(total_size).unwrap().to_be_bytes());
    bytes.extend_from_slice(b"AIFC");
    bytes.extend_from_slice(b"COMM");
    bytes.extend_from_slice(&u32::try_from(comm_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&comm_payload);
    bytes.extend_from_slice(b"SSND");
    bytes.extend_from_slice(&u32::try_from(ssnd_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&ssnd_payload);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
fn write_test_aifc_float_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    compression: [u8; 4],
    frames: &[&[f64]],
) -> PathBuf {
    let bytes_per_sample = usize::from(bits_per_sample / 8);
    let mut data =
        Vec::with_capacity(frames.len() * usize::from(channel_count) * bytes_per_sample);
    for frame in frames {
        assert_eq!(frame.len(), usize::from(channel_count));
        for &sample in *frame {
            match bits_per_sample {
                64 => data.extend_from_slice(&sample.to_be_bytes()),
                32 => data.extend_from_slice(&(sample as f32).to_be_bytes()),
                _ => unreachable!(),
            }
        }
    }
    let sample_frames = u32::try_from(frames.len()).unwrap();

    let mut comm_payload = Vec::new();
    comm_payload.extend_from_slice(&channel_count.to_be_bytes());
    comm_payload.extend_from_slice(&sample_frames.to_be_bytes());
    comm_payload.extend_from_slice(&bits_per_sample.to_be_bytes());
    comm_payload.extend_from_slice(&encode_aiff_extended_sample_rate(sample_rate));
    comm_payload.extend_from_slice(&compression);

    let mut ssnd_payload = Vec::new();
    ssnd_payload.extend_from_slice(&0_u32.to_be_bytes());
    ssnd_payload.extend_from_slice(&0_u32.to_be_bytes());
    ssnd_payload.extend_from_slice(&data);

    let mut bytes = Vec::new();
    let total_size = 4 + (8 + comm_payload.len()) + (8 + ssnd_payload.len());
    bytes.extend_from_slice(b"FORM");
    bytes.extend_from_slice(&u32::try_from(total_size).unwrap().to_be_bytes());
    bytes.extend_from_slice(b"AIFC");
    bytes.extend_from_slice(b"COMM");
    bytes.extend_from_slice(&u32::try_from(comm_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&comm_payload);
    bytes.extend_from_slice(b"SSND");
    bytes.extend_from_slice(&u32::try_from(ssnd_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&ssnd_payload);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
fn write_test_aiff_like_pcm_file(
    prefix: &str,
    frames: &[[i16; 2]],
    compression: Option<[u8; 4]>,
) -> PathBuf {
    let channel_count = 2_u16;
    let sample_rate = 48_000_u32;
    let bits_per_sample = 16_u16;

    let mut data = Vec::with_capacity(frames.len() * usize::from(channel_count) * 2);
    for frame in frames {
        for sample in frame {
            data.extend_from_slice(&sample.to_be_bytes());
        }
    }

    let mut comm_payload = Vec::new();
    comm_payload.extend_from_slice(&channel_count.to_be_bytes());
    comm_payload.extend_from_slice(&u32::try_from(frames.len()).unwrap().to_be_bytes());
    comm_payload.extend_from_slice(&bits_per_sample.to_be_bytes());
    comm_payload.extend_from_slice(&encode_aiff_extended_sample_rate(sample_rate));
    if let Some(compression) = compression {
        comm_payload.extend_from_slice(&compression);
    }

    let mut ssnd_payload = Vec::new();
    ssnd_payload.extend_from_slice(&0_u32.to_be_bytes());
    ssnd_payload.extend_from_slice(&0_u32.to_be_bytes());
    ssnd_payload.extend_from_slice(&data);

    let form_type = if compression.is_some() {
        *b"AIFC"
    } else {
        *b"AIFF"
    };
    let mut bytes = Vec::new();
    let total_size = 4 + (8 + comm_payload.len()) + (8 + ssnd_payload.len());
    bytes.extend_from_slice(b"FORM");
    bytes.extend_from_slice(&u32::try_from(total_size).unwrap().to_be_bytes());
    bytes.extend_from_slice(&form_type);
    bytes.extend_from_slice(b"COMM");
    bytes.extend_from_slice(&u32::try_from(comm_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&comm_payload);
    bytes.extend_from_slice(b"SSND");
    bytes.extend_from_slice(&u32::try_from(ssnd_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&ssnd_payload);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
fn encode_aiff_extended_sample_rate(sample_rate: u32) -> [u8; 10] {
    let msb_index = 31_u32 - sample_rate.leading_zeros();
    let exponent = 16383_u16 + u16::try_from(msb_index).unwrap();
    let mantissa = u64::from(sample_rate) << (63 - msb_index);

    let mut bytes = [0_u8; 10];
    bytes[..2].copy_from_slice(&exponent.to_be_bytes());
    bytes[2..].copy_from_slice(&mantissa.to_be_bytes());
    bytes
}

#[cfg(feature = "mux")]
pub struct TestAviPcmStream<'a> {
    pub sample_rate: u32,
    pub channel_count: u16,
    pub bits_per_sample: u16,
    pub chunks: &'a [&'a [u8]],
}

#[cfg(feature = "mux")]
pub struct TestAviMp4vStream<'a> {
    pub width: u16,
    pub height: u16,
    pub frame_scale: u32,
    pub frame_rate: u32,
    pub compression: [u8; 4],
    pub decoder_specific_info: &'a [u8],
    pub frames: &'a [&'a [u8]],
}

#[cfg(feature = "mux")]
pub struct TestAviH264Stream<'a> {
    pub width: u16,
    pub height: u16,
    pub frame_scale: u32,
    pub frame_rate: u32,
    pub compression: [u8; 4],
    pub sample_payloads: &'a [&'a [u8]],
}

#[cfg(feature = "mux")]
pub struct TestAviAvc1Stream<'a> {
    pub width: u16,
    pub height: u16,
    pub frame_scale: u32,
    pub frame_rate: u32,
    pub sample_payloads: &'a [&'a [u8]],
}

#[cfg(feature = "mux")]
pub fn write_test_avi_mp3_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    payloads: &[&[u8]],
) -> PathBuf {
    let frames = payloads
        .iter()
        .map(|payload| build_mp3_frame(payload))
        .collect::<Vec<_>>();
    let frame_refs = frames.iter().map(Vec::as_slice).collect::<Vec<_>>();
    write_test_avi_framed_audio_file(prefix, 0x0055, sample_rate, channel_count, 16, &frame_refs)
}

#[cfg(feature = "mux")]
pub fn write_test_avi_ac3_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    payloads: &[&[u8]],
) -> PathBuf {
    let frames = payloads
        .iter()
        .map(|payload| build_ac3_frame(payload))
        .collect::<Vec<_>>();
    let frame_refs = frames.iter().map(Vec::as_slice).collect::<Vec<_>>();
    write_test_avi_framed_audio_file(prefix, 0x2000, sample_rate, channel_count, 16, &frame_refs)
}

#[cfg(feature = "mux")]
pub fn write_test_avi_pcm_file(prefix: &str, streams: &[TestAviPcmStream<'_>]) -> PathBuf {
    let avih = build_test_avi_avih_payload(
        streams.len(),
        streams
            .iter()
            .flat_map(|stream| stream.chunks.iter().map(|chunk| chunk.len()))
            .max()
            .unwrap_or(0),
    );
    let mut hdrl_children = encode_riff_chunk(*b"avih", &avih);
    for (index, stream) in streams.iter().enumerate() {
        hdrl_children.extend_from_slice(&encode_riff_list(
            *b"strl",
            &build_test_avi_pcm_stream_list(index, stream),
        ));
    }
    let hdrl = encode_riff_list(*b"hdrl", &hdrl_children);
    let movi = encode_riff_list(*b"movi", &build_test_avi_movi_payload(streams));

    let mut riff_payload = Vec::new();
    riff_payload.extend_from_slice(b"AVI ");
    riff_payload.extend_from_slice(&hdrl);
    riff_payload.extend_from_slice(&movi);
    write_temp_file(prefix, &encode_riff_chunk(*b"RIFF", &riff_payload))
}

#[cfg(feature = "mux")]
pub fn write_test_avi_alaw_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    chunks: &[&[u8]],
) -> PathBuf {
    write_test_avi_companded_audio_file(prefix, 0x0006, sample_rate, channel_count, chunks)
}

#[cfg(feature = "mux")]
pub fn write_test_avi_mulaw_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    chunks: &[&[u8]],
) -> PathBuf {
    write_test_avi_companded_audio_file(prefix, 0x0007, sample_rate, channel_count, chunks)
}

#[cfg(feature = "mux")]
pub fn write_test_avi_extensible_pcm_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    chunks: &[&[u8]],
) -> PathBuf {
    write_test_avi_extensible_audio_file(
        prefix,
        sample_rate,
        channel_count,
        bits_per_sample,
        chunks,
        &[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38,
            0x9B, 0x71,
        ],
    )
}

#[cfg(feature = "mux")]
pub fn write_test_avi_extensible_float_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    chunks: &[&[u8]],
) -> PathBuf {
    write_test_avi_extensible_audio_file(
        prefix,
        sample_rate,
        channel_count,
        bits_per_sample,
        chunks,
        &[
            0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38,
            0x9B, 0x71,
        ],
    )
}

#[cfg(feature = "mux")]
pub fn write_test_avi_extensible_alaw_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    chunks: &[&[u8]],
) -> PathBuf {
    write_test_avi_extensible_audio_file(
        prefix,
        sample_rate,
        channel_count,
        8,
        chunks,
        &[
            0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38,
            0x9B, 0x71,
        ],
    )
}

#[cfg(feature = "mux")]
pub fn write_test_avi_extensible_mulaw_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    chunks: &[&[u8]],
) -> PathBuf {
    write_test_avi_extensible_audio_file(
        prefix,
        sample_rate,
        channel_count,
        8,
        chunks,
        &[
            0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38,
            0x9B, 0x71,
        ],
    )
}

#[cfg(feature = "mux")]
fn write_test_avi_companded_audio_file(
    prefix: &str,
    format_tag: u16,
    sample_rate: u32,
    channel_count: u16,
    chunks: &[&[u8]],
) -> PathBuf {
    let stream = TestAviPcmStream {
        sample_rate,
        channel_count,
        bits_per_sample: 8,
        chunks,
    };
    let avih =
        build_test_avi_avih_payload(1, chunks.iter().map(|chunk| chunk.len()).max().unwrap_or(0));
    let mut hdrl_children = encode_riff_chunk(*b"avih", &avih);
    hdrl_children.extend_from_slice(&encode_riff_list(
        *b"strl",
        &build_test_avi_pcm_stream_list_with_format_tag(0, &stream, format_tag),
    ));
    let hdrl = encode_riff_list(*b"hdrl", &hdrl_children);
    let movi = encode_riff_list(*b"movi", &build_test_avi_audio_movi_payload(chunks));

    let mut riff_payload = Vec::new();
    riff_payload.extend_from_slice(b"AVI ");
    riff_payload.extend_from_slice(&hdrl);
    riff_payload.extend_from_slice(&movi);
    write_temp_file(prefix, &encode_riff_chunk(*b"RIFF", &riff_payload))
}

#[cfg(feature = "mux")]
fn write_test_avi_extensible_audio_file(
    prefix: &str,
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    chunks: &[&[u8]],
    subtype_guid: &[u8; 16],
) -> PathBuf {
    let stream = TestAviPcmStream {
        sample_rate,
        channel_count,
        bits_per_sample,
        chunks,
    };
    let avih =
        build_test_avi_avih_payload(1, chunks.iter().map(|chunk| chunk.len()).max().unwrap_or(0));
    let mut hdrl_children = encode_riff_chunk(*b"avih", &avih);
    hdrl_children.extend_from_slice(&encode_riff_list(
        *b"strl",
        &build_test_avi_pcm_stream_list_with_extensible_subtype(0, &stream, subtype_guid),
    ));
    let hdrl = encode_riff_list(*b"hdrl", &hdrl_children);
    let movi = encode_riff_list(*b"movi", &build_test_avi_audio_movi_payload(chunks));

    let mut riff_payload = Vec::new();
    riff_payload.extend_from_slice(b"AVI ");
    riff_payload.extend_from_slice(&hdrl);
    riff_payload.extend_from_slice(&movi);
    write_temp_file(prefix, &encode_riff_chunk(*b"RIFF", &riff_payload))
}

#[cfg(feature = "mux")]
struct TestAviVideoFileSpec<'a> {
    width: u16,
    height: u16,
    frame_scale: u32,
    frame_rate: u32,
    compression: [u8; 4],
    decoder_specific_info: &'a [u8],
    frames: &'a [&'a [u8]],
}

#[cfg(feature = "mux")]
pub fn write_test_avi_h263_file(
    prefix: &str,
    width: u16,
    height: u16,
    frame_scale: u32,
    frame_rate: u32,
    sample_payloads: &[&[u8]],
) -> PathBuf {
    let frames = sample_payloads
        .iter()
        .enumerate()
        .map(|(index, payload)| build_test_h263_frame(u8::try_from(index).unwrap(), payload))
        .collect::<Vec<_>>();
    let frame_refs = frames.iter().map(Vec::as_slice).collect::<Vec<_>>();
    write_test_avi_video_file(
        prefix,
        TestAviVideoFileSpec {
            width,
            height,
            frame_scale,
            frame_rate,
            compression: *b"H263",
            decoder_specific_info: &[],
            frames: &frame_refs,
        },
    )
}

#[cfg(feature = "mux")]
pub fn write_test_avi_jpeg_file(
    prefix: &str,
    width: u16,
    height: u16,
    frame_scale: u32,
    frame_rate: u32,
    frames: &[&[u8]],
) -> PathBuf {
    write_test_avi_video_file(
        prefix,
        TestAviVideoFileSpec {
            width,
            height,
            frame_scale,
            frame_rate,
            compression: *b"MJPG",
            decoder_specific_info: &[],
            frames,
        },
    )
}

#[cfg(feature = "mux")]
pub fn write_test_avi_png_file(
    prefix: &str,
    width: u16,
    height: u16,
    frame_scale: u32,
    frame_rate: u32,
    frames: &[&[u8]],
) -> PathBuf {
    write_test_avi_video_file(
        prefix,
        TestAviVideoFileSpec {
            width,
            height,
            frame_scale,
            frame_rate,
            compression: *b"PNG ",
            decoder_specific_info: &[],
            frames,
        },
    )
}

#[cfg(feature = "mux")]
pub fn write_test_avi_video_tag_file(
    prefix: &str,
    width: u16,
    height: u16,
    frame_scale: u32,
    frame_rate: u32,
    compression: [u8; 4],
    frames: &[&[u8]],
) -> PathBuf {
    write_test_avi_video_file(
        prefix,
        TestAviVideoFileSpec {
            width,
            height,
            frame_scale,
            frame_rate,
            compression,
            decoder_specific_info: &[],
            frames,
        },
    )
}

#[cfg(feature = "mux")]
pub fn write_test_avi_raw_bgr_file(
    prefix: &str,
    width: u16,
    height: u16,
    frame_scale: u32,
    frame_rate: u32,
    frames: &[&[u8]],
) -> PathBuf {
    write_test_avi_video_tag_file(
        prefix,
        width,
        height,
        frame_scale,
        frame_rate,
        [0, 0, 0, 0],
        frames,
    )
}

#[cfg(feature = "mux")]
pub fn write_test_avi_mp4v_file(prefix: &str, stream: &TestAviMp4vStream<'_>) -> PathBuf {
    let avih = build_test_avi_avih_payload(
        1,
        stream
            .frames
            .iter()
            .map(|frame| frame.len())
            .max()
            .unwrap_or(0),
    );
    let mut hdrl_children = encode_riff_chunk(*b"avih", &avih);
    hdrl_children.extend_from_slice(&encode_riff_list(
        *b"strl",
        &build_test_avi_mp4v_stream_list(stream),
    ));
    let hdrl = encode_riff_list(*b"hdrl", &hdrl_children);
    let movi = encode_riff_list(*b"movi", &build_test_avi_mp4v_movi_payload(stream));

    let mut riff_payload = Vec::new();
    riff_payload.extend_from_slice(b"AVI ");
    riff_payload.extend_from_slice(&hdrl);
    riff_payload.extend_from_slice(&movi);
    write_temp_file(prefix, &encode_riff_chunk(*b"RIFF", &riff_payload))
}

#[cfg(feature = "mux")]
pub fn write_test_avi_h264_file(prefix: &str, stream: &TestAviH264Stream<'_>) -> PathBuf {
    let frames = build_test_h264_annexb_chunks(stream.sample_payloads);
    let frame_refs = frames.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let avih = build_test_avi_avih_payload(
        1,
        frame_refs
            .iter()
            .map(|frame| frame.len())
            .max()
            .unwrap_or(0),
    );
    let mut hdrl_children = encode_riff_chunk(*b"avih", &avih);
    hdrl_children.extend_from_slice(&encode_riff_list(
        *b"strl",
        &build_test_avi_video_stream_list(
            stream.width,
            stream.height,
            stream.frame_scale,
            stream.frame_rate,
            stream.compression,
            &[],
            &frame_refs,
        ),
    ));
    let hdrl = encode_riff_list(*b"hdrl", &hdrl_children);
    let movi = encode_riff_list(*b"movi", &build_test_avi_video_movi_payload(&frame_refs));

    let mut riff_payload = Vec::new();
    riff_payload.extend_from_slice(b"AVI ");
    riff_payload.extend_from_slice(&hdrl);
    riff_payload.extend_from_slice(&movi);
    write_temp_file(prefix, &encode_riff_chunk(*b"RIFF", &riff_payload))
}

#[cfg(feature = "mux")]
pub fn write_test_avi_avc1_file(prefix: &str, stream: &TestAviAvc1Stream<'_>) -> PathBuf {
    let frames = build_test_h264_avc1_chunks(stream.sample_payloads);
    let frame_refs = frames.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let avih = build_test_avi_avih_payload(
        1,
        frame_refs
            .iter()
            .map(|frame| frame.len())
            .max()
            .unwrap_or(0),
    );
    let mut hdrl_children = encode_riff_chunk(*b"avih", &avih);
    hdrl_children.extend_from_slice(&encode_riff_list(
        *b"strl",
        &build_test_avi_video_stream_list(
            stream.width,
            stream.height,
            stream.frame_scale,
            stream.frame_rate,
            *b"AVC1",
            &build_test_avcc_decoder_specific_info(),
            &frame_refs,
        ),
    ));
    let hdrl = encode_riff_list(*b"hdrl", &hdrl_children);
    let movi = encode_riff_list(*b"movi", &build_test_avi_video_movi_payload(&frame_refs));

    let mut riff_payload = Vec::new();
    riff_payload.extend_from_slice(b"AVI ");
    riff_payload.extend_from_slice(&hdrl);
    riff_payload.extend_from_slice(&movi);
    write_temp_file(prefix, &encode_riff_chunk(*b"RIFF", &riff_payload))
}

#[cfg(feature = "mux")]
fn write_test_avi_framed_audio_file(
    prefix: &str,
    format_tag: u16,
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    frames: &[&[u8]],
) -> PathBuf {
    let avih =
        build_test_avi_avih_payload(1, frames.iter().map(|frame| frame.len()).max().unwrap_or(0));
    let mut hdrl_children = encode_riff_chunk(*b"avih", &avih);
    hdrl_children.extend_from_slice(&encode_riff_list(
        *b"strl",
        &build_test_avi_framed_audio_stream_list(
            format_tag,
            sample_rate,
            channel_count,
            bits_per_sample,
            frames,
        ),
    ));
    let hdrl = encode_riff_list(*b"hdrl", &hdrl_children);
    let movi = encode_riff_list(*b"movi", &build_test_avi_audio_movi_payload(frames));

    let mut riff_payload = Vec::new();
    riff_payload.extend_from_slice(b"AVI ");
    riff_payload.extend_from_slice(&hdrl);
    riff_payload.extend_from_slice(&movi);
    write_temp_file(prefix, &encode_riff_chunk(*b"RIFF", &riff_payload))
}

#[cfg(feature = "mux")]
pub fn write_test_avi_audio_tag_file(
    prefix: &str,
    format_tag: u16,
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    frames: &[&[u8]],
) -> PathBuf {
    write_test_avi_framed_audio_file(
        prefix,
        format_tag,
        sample_rate,
        channel_count,
        bits_per_sample,
        frames,
    )
}

#[cfg(feature = "mux")]
fn write_test_avi_video_file(prefix: &str, spec: TestAviVideoFileSpec<'_>) -> PathBuf {
    let avih = build_test_avi_avih_payload(
        1,
        spec.frames
            .iter()
            .map(|frame| frame.len())
            .max()
            .unwrap_or(0),
    );
    let mut hdrl_children = encode_riff_chunk(*b"avih", &avih);
    hdrl_children.extend_from_slice(&encode_riff_list(
        *b"strl",
        &build_test_avi_video_stream_list(
            spec.width,
            spec.height,
            spec.frame_scale,
            spec.frame_rate,
            spec.compression,
            spec.decoder_specific_info,
            spec.frames,
        ),
    ));
    let hdrl = encode_riff_list(*b"hdrl", &hdrl_children);
    let movi = encode_riff_list(*b"movi", &build_test_avi_video_movi_payload(spec.frames));

    let mut riff_payload = Vec::new();
    riff_payload.extend_from_slice(b"AVI ");
    riff_payload.extend_from_slice(&hdrl);
    riff_payload.extend_from_slice(&movi);
    write_temp_file(prefix, &encode_riff_chunk(*b"RIFF", &riff_payload))
}

#[cfg(feature = "mux")]
pub fn write_test_mp4v_file(prefix: &str, bytes: &[u8]) -> PathBuf {
    write_temp_file(prefix, bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_mpeg2v_file(prefix: &str, bytes: &[u8]) -> PathBuf {
    write_temp_file(prefix, bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_saf_aac_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    const STREAM_ID: u16 = 1;
    const TIMESCALE: u32 = 48_000;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_test_saf_declaration_au(TestSafDeclaration {
        au_sn: 0,
        cts: 0,
        au_type: 1,
        stream_id: STREAM_ID,
        object_type_indication: 0x40,
        stream_type: 0x05,
        timescale: TIMESCALE,
        decoder_specific_info: &[0x11, 0x90],
    }));
    for (index, payload) in payloads.iter().enumerate() {
        bytes.extend_from_slice(&build_test_saf_data_au(
            u16::try_from(index + 1).unwrap(),
            u32::try_from(index).unwrap() * 1_024,
            STREAM_ID,
            index == 0,
            payload,
        ));
    }
    write_temp_file_with_extension(prefix, "saf", &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_saf_scene_plus_mp4v_file(
    prefix: &str,
    scene_payloads: &[&[u8]],
    video_payloads: &[&[u8]],
) -> PathBuf {
    const SCENE_STREAM_ID: u16 = 1;
    const VIDEO_STREAM_ID: u16 = 2;
    const TIMESCALE: u32 = 1_000;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_test_saf_declaration_au(TestSafDeclaration {
        au_sn: 0,
        cts: 0,
        au_type: 1,
        stream_id: SCENE_STREAM_ID,
        object_type_indication: 0x01,
        stream_type: 0x03,
        timescale: TIMESCALE,
        decoder_specific_info: &[0x12, 0x34],
    }));
    bytes.extend_from_slice(&build_test_saf_declaration_au(TestSafDeclaration {
        au_sn: 1,
        cts: 0,
        au_type: 1,
        stream_id: VIDEO_STREAM_ID,
        object_type_indication: 0x20,
        stream_type: 0x04,
        timescale: TIMESCALE,
        decoder_specific_info: &build_test_mp4v_decoder_specific_info(320, 180),
    }));
    let mut au_sn = 2_u16;
    for (index, payload) in scene_payloads.iter().enumerate() {
        let cts = u32::try_from(index).unwrap() * 1_000;
        bytes.extend_from_slice(&build_test_saf_data_au(
            au_sn,
            cts,
            SCENE_STREAM_ID,
            true,
            payload,
        ));
        au_sn += 1;
    }
    for (index, payload) in video_payloads.iter().enumerate() {
        let cts = u32::try_from(index).unwrap() * 1_000;
        bytes.extend_from_slice(&build_test_saf_data_au(
            au_sn,
            cts,
            VIDEO_STREAM_ID,
            index == 0,
            payload,
        ));
        au_sn += 1;
    }
    write_temp_file_with_extension(prefix, "saf", &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_saf_remote_url_file(prefix: &str) -> PathBuf {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_test_saf_declaration_au(TestSafDeclaration {
        au_sn: 0,
        cts: 0,
        au_type: 7,
        stream_id: 1,
        object_type_indication: 0x01,
        stream_type: 0x03,
        timescale: 1_000,
        decoder_specific_info: b"https://example.invalid/scene.lsr",
    }));
    write_temp_file_with_extension(prefix, "saf", &bytes)
}

#[cfg(feature = "mux")]
struct TestSafDeclaration<'a> {
    au_sn: u16,
    cts: u32,
    au_type: u8,
    stream_id: u16,
    object_type_indication: u8,
    stream_type: u8,
    timescale: u32,
    decoder_specific_info: &'a [u8],
}

#[cfg(feature = "mux")]
fn build_test_saf_declaration_au(declaration: TestSafDeclaration<'_>) -> Vec<u8> {
    let mut payload = vec![
        declaration.object_type_indication,
        declaration.stream_type,
        u8::try_from((declaration.timescale >> 16) & 0xFF).unwrap(),
        u8::try_from((declaration.timescale >> 8) & 0xFF).unwrap(),
        u8::try_from(declaration.timescale & 0xFF).unwrap(),
        0,
        0,
    ];
    payload.extend_from_slice(declaration.decoder_specific_info);
    build_test_saf_au(
        true,
        declaration.au_sn,
        declaration.cts,
        declaration.au_type,
        declaration.stream_id,
        &payload,
    )
}

#[cfg(feature = "mux")]
fn build_test_saf_data_au(
    au_sn: u16,
    cts: u32,
    stream_id: u16,
    is_rap: bool,
    payload: &[u8],
) -> Vec<u8> {
    build_test_saf_au(is_rap, au_sn, cts, 4, stream_id, payload)
}

#[cfg(feature = "mux")]
fn build_test_saf_au(
    is_rap: bool,
    au_sn: u16,
    cts: u32,
    au_type: u8,
    stream_id: u16,
    payload: &[u8],
) -> Vec<u8> {
    let payload_size = u16::try_from(payload.len() + 2).unwrap();
    let outer = ((u64::from(is_rap as u8)) << 63)
        | ((u64::from(au_sn & 0x7FFF)) << 48)
        | ((u64::from(cts & 0x3FFF_FFFF)) << 16)
        | u64::from(payload_size);
    let inner = (u16::from(au_type & 0x0F) << 12) | (stream_id & 0x0FFF);
    let mut bytes = outer.to_be_bytes().to_vec();
    bytes.extend_from_slice(&inner.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
pub fn build_test_mp4v_decoder_specific_info(width: u16, height: u16) -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, 1, 8);
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, 1, 4);
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, 0, 2);
    writer.write_bit(true).unwrap();
    write_test_bits_u64(&mut writer, 1_000, 16);
    writer.write_bit(true).unwrap();
    writer.write_bit(false).unwrap();
    writer.write_bit(true).unwrap();
    write_test_bits_u64(&mut writer, u64::from(width), 13);
    writer.write_bit(true).unwrap();
    write_test_bits_u64(&mut writer, u64::from(height), 13);
    writer.write_bit(true).unwrap();
    align_test_bit_writer(&mut writer);

    let mut bytes = vec![0x00, 0x00, 0x01, 0x20];
    bytes.extend_from_slice(&writer.into_inner().unwrap());
    bytes
}

#[cfg(feature = "mux")]
pub fn build_test_mp4v_decoder_specific_info_with_vol_control(width: u16, height: u16) -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, 1, 8);
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, 1, 4);
    writer.write_bit(true).unwrap();
    write_test_bits_u64(&mut writer, 1, 2);
    writer.write_bit(false).unwrap();
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, 0, 2);
    writer.write_bit(true).unwrap();
    write_test_bits_u64(&mut writer, 1_000, 16);
    writer.write_bit(true).unwrap();
    writer.write_bit(false).unwrap();
    writer.write_bit(true).unwrap();
    write_test_bits_u64(&mut writer, u64::from(width), 13);
    writer.write_bit(true).unwrap();
    write_test_bits_u64(&mut writer, u64::from(height), 13);
    writer.write_bit(true).unwrap();
    align_test_bit_writer(&mut writer);

    let mut bytes = vec![0x00, 0x00, 0x01, 0x20];
    bytes.extend_from_slice(&writer.into_inner().unwrap());
    bytes
}

#[cfg(feature = "mux")]
pub fn build_test_mpeg2v_bytes(width: u16, height: u16, sample_payloads: &[&[u8]]) -> Vec<u8> {
    let mut bytes = vec![
        0x00,
        0x00,
        0x01,
        0xB3,
        u8::try_from(width >> 4).unwrap(),
        u8::try_from(((width & 0x0F) << 4) | (height >> 8)).unwrap(),
        u8::try_from(height & 0xFF).unwrap(),
        0x13,
        0x00,
        0x00,
        0x01,
        0xB5,
        0x14,
        0x80,
        0x00,
        0x00,
        0x00,
        0x00,
    ];
    for (index, payload) in sample_payloads.iter().enumerate() {
        bytes.extend_from_slice(&build_test_mpeg2v_picture_bytes(index, payload));
    }
    bytes
}

#[cfg(feature = "mux")]
fn build_test_mpeg2v_picture_bytes(index: usize, payload: &[u8]) -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    write_test_bits_u64(&mut writer, u64::try_from(index).unwrap(), 10);
    write_test_bits_u64(&mut writer, 1, 3);
    write_test_bits_u64(&mut writer, 0xFFFF, 16);
    align_test_bit_writer(&mut writer);

    let mut bytes = vec![0x00, 0x00, 0x01, 0x00];
    bytes.extend_from_slice(&writer.into_inner().unwrap());
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0x01]);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_mp3_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    for payload in payloads {
        bytes.extend_from_slice(&build_test_program_stream_mp3_pes_packet(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_mp2_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    for payload in payloads {
        bytes.extend_from_slice(&build_test_program_stream_mp2_pes_packet(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_ac3_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    for payload in payloads {
        bytes.extend_from_slice(&build_test_program_stream_ac3_pes_packet(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_lpcm_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    for payload in payloads {
        bytes.extend_from_slice(&build_test_program_stream_lpcm_pes_packet(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_mp4v_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    for payload in payloads {
        bytes.extend_from_slice(&build_test_program_stream_mp4v_pes_packet(payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_mpeg2v_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    for (index, payload) in sample_payloads.iter().enumerate() {
        let mut elementary_sample = if index == 0 {
            build_test_mpeg2v_bytes(320, 180, &[*payload])
        } else {
            build_test_mpeg2v_picture_bytes(index, payload)
        };
        if index + 1 == sample_payloads.len() {
            elementary_sample.extend_from_slice(&[0x00, 0x00, 0x01, 0xB7]);
        }
        bytes.extend_from_slice(&build_test_program_stream_video_pes_packet_with_pts(
            u64::try_from(index).unwrap() * 3_600,
            &elementary_sample,
        ));
    }
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xB9]);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_mpeg2v_pts_dts_file(
    prefix: &str,
    sample_payloads: &[&[u8]],
) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    for (index, payload) in sample_payloads.iter().enumerate() {
        let mut elementary_sample = if index == 0 {
            build_test_mpeg2v_bytes(320, 180, &[*payload])
        } else {
            build_test_mpeg2v_picture_bytes(index, payload)
        };
        if index + 1 == sample_payloads.len() {
            elementary_sample.extend_from_slice(&[0x00, 0x00, 0x01, 0xB7]);
        }
        let timestamp = u64::try_from(index).unwrap() * 3_600;
        bytes.extend_from_slice(
            &build_test_program_stream_video_pes_packet_with_pts_and_dts(
                timestamp,
                timestamp,
                &elementary_sample,
            ),
        );
    }
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xB9]);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_h264_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    bytes.extend_from_slice(&build_test_program_stream_video_pes_packet(
        &build_test_h264_annexb_bytes(sample_payloads),
    ));
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_h264_open_ended_file(
    prefix: &str,
    sample_payloads: &[&[u8]],
) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    bytes.extend_from_slice(&build_test_program_stream_open_ended_video_pes_packet(
        &build_test_h264_annexb_bytes(sample_payloads),
    ));
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xB9]);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_h265_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    bytes.extend_from_slice(&build_test_program_stream_video_pes_packet(
        &build_test_h265_annexb_bytes(sample_payloads),
    ));
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_vvc_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = build_test_program_stream_pack_header();
    let raw_vvc = fixture_path("mux/raw_vvc_idr.vvc");
    let mut annex_b = fs::read(raw_vvc).unwrap();
    for extra in sample_payloads {
        annex_b.extend_from_slice(extra);
    }
    bytes.extend_from_slice(&build_test_program_stream_video_pes_packet(&annex_b));
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_mp3_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for payload in payloads {
        let pes_packet = build_test_transport_stream_mp3_pes_packet(payload);
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_latm_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x11,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for (index, payload) in payloads.iter().enumerate() {
        let pes_packet = build_test_transport_stream_mpeg_audio_pes_packet_with_pts(
            u64::try_from(index).unwrap() * 1_920,
            &build_test_latm_frame(index != 0, payload),
        );
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_latm_other_data_file(
    prefix: &str,
    payloads: &[&[u8]],
) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x11,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for (index, payload) in payloads.iter().enumerate() {
        let pes_packet = build_test_transport_stream_mpeg_audio_pes_packet_with_pts(
            u64::try_from(index).unwrap() * 1_920,
            &build_test_latm_frame_with_options(index != 0, payload, 2, true, false),
        );
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_mhas_file(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    assert!(!frame_payloads.is_empty());
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x2D,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    let mut first_payload = Vec::new();
    first_payload.extend_from_slice(&build_mhas_packet(6, &[0xA5]));
    first_payload.extend_from_slice(&build_mhas_packet(1, &build_test_mhas_config_payload()));
    for (index, frame_payload) in frame_payloads.iter().enumerate() {
        let mut frame = Vec::with_capacity(frame_payload.len() + 1);
        frame.push(0x80);
        frame.extend_from_slice(frame_payload);
        let carried_frame = build_mhas_packet(2, &frame);
        if index == 0 {
            first_payload.extend_from_slice(&carried_frame);
        }
    }
    let pes_packet = build_test_transport_stream_mpeg_audio_pes_packet_with_pts(0, &first_payload);
    bytes.extend_from_slice(&packetize_test_transport_stream_pes(
        0x0101,
        &mut continuity_counter,
        &pes_packet,
    ));
    for (index, frame_payload) in frame_payloads.iter().enumerate().skip(1) {
        let mut frame = Vec::with_capacity(frame_payload.len() + 1);
        frame.push(0x80);
        frame.extend_from_slice(frame_payload);
        let pes_packet = build_test_transport_stream_mpeg_audio_pes_packet_with_pts(
            u64::try_from(index).unwrap() * 1_920,
            &build_mhas_packet(2, &frame),
        );
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_ac3_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x81,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for payload in payloads {
        let pes_packet =
            build_test_transport_stream_private_data_pes_packet(&build_ac3_frame(payload));
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_eac3_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x84,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for payload in payloads {
        let pes_packet =
            build_test_transport_stream_private_data_pes_packet(&build_eac3_frame(payload));
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_mp4v_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x10,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for payload in payloads {
        let pes_packet = build_test_transport_stream_mp4v_pes_packet(payload);
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_mpeg2v_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x02,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for (index, payload) in sample_payloads.iter().enumerate() {
        let elementary_sample = if index == 0 {
            build_test_mpeg2v_bytes(320, 180, &[*payload])
        } else {
            build_test_mpeg2v_picture_bytes(index, payload)
        };
        let pes_packet = build_test_transport_stream_mpeg2v_pes_packet_with_pts(
            u64::try_from(index).unwrap() * 3_600,
            &elementary_sample,
        );
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_av1_file(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_private_data(
        continuity_counter,
        &[
            build_test_transport_stream_registration_descriptor(*b"AV01"),
            build_test_transport_stream_private_data_specifier_descriptor(*b"AOMS"),
            build_test_transport_stream_av1_video_descriptor(),
        ]
        .concat(),
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for (index, payload) in frame_payloads.iter().enumerate() {
        let elementary_sample = build_test_transport_stream_av1_sample_bytes(payload);
        let pes_packet = build_test_transport_stream_video_pes_packet_with_pts(
            u64::try_from(index).unwrap() * 3_600,
            &elementary_sample,
        );
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_avs3_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    let sequence_header = build_test_avs3_sequence_header_bytes(320, 180, 0x03);
    let decoder_config = build_test_transport_stream_avs3_decoder_config(&sequence_header);
    bytes.extend_from_slice(
        &build_test_transport_stream_pmt_packet_for_stream_type_with_descriptors(
            continuity_counter,
            0xD4,
            &build_test_transport_stream_avs3_registration_descriptor(&decoder_config),
        ),
    );
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for (index, payload) in sample_payloads.iter().enumerate() {
        let elementary_sample = if index == 0 {
            [
                sequence_header.clone(),
                build_test_avs3_picture_bytes(true, payload),
            ]
            .concat()
        } else {
            build_test_avs3_picture_bytes(false, payload)
        };
        let pes_packet = build_test_transport_stream_mpeg2v_pes_packet_with_pts(
            u64::try_from(index).unwrap() * 3_600,
            &elementary_sample,
        );
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_h264_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x1B,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    let pes_packet = build_test_transport_stream_video_pes_packet(&build_test_h264_annexb_bytes(
        sample_payloads,
    ));
    bytes.extend_from_slice(&packetize_test_transport_stream_pes(
        0x0101,
        &mut continuity_counter,
        &pes_packet,
    ));
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_h265_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x24,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    let pes_packet = build_test_transport_stream_video_pes_packet(&build_test_h265_annexb_bytes(
        sample_payloads,
    ));
    bytes.extend_from_slice(&packetize_test_transport_stream_pes(
        0x0101,
        &mut continuity_counter,
        &pes_packet,
    ));
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_vvc_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x33,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    let raw_vvc = fixture_path("mux/raw_vvc_idr.vvc");
    let mut annex_b = fs::read(raw_vvc).unwrap();
    for extra in sample_payloads {
        annex_b.extend_from_slice(extra);
    }
    let pes_packet = build_test_transport_stream_video_pes_packet(&annex_b);
    bytes.extend_from_slice(&packetize_test_transport_stream_pes(
        0x0101,
        &mut continuity_counter,
        &pes_packet,
    ));
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_dts_file(prefix: &str, frame_count: usize) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_private_data(
        continuity_counter,
        &build_test_transport_stream_registration_descriptor(*b"DTS1"),
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for index in 0..frame_count {
        let pes_packet =
            build_test_transport_stream_private_data_pes_packet(&build_dts_frame(index));
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_dts_stream_type_file(
    prefix: &str,
    frame_count: usize,
) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x82,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for index in 0..frame_count {
        let pes_packet =
            build_test_transport_stream_private_data_pes_packet(&build_dts_frame(index));
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_ac4_file(prefix: &str, frame_count: usize) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_private_data(
        continuity_counter,
        &build_test_transport_stream_registration_descriptor(*b"AC-4"),
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    let pes_packet = build_test_transport_stream_private_data_pes_packet(
        &build_test_ac4_stream_bytes(frame_count),
    );
    bytes.extend_from_slice(&packetize_test_transport_stream_pes(
        0x0101,
        &mut continuity_counter,
        &pes_packet,
    ));
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_truehd_file(prefix: &str, payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_stream_type(
        continuity_counter,
        0x83,
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    let pes_packet = build_test_transport_stream_private_data_pes_packet(
        &build_test_truehd_stream_bytes(payloads),
    );
    bytes.extend_from_slice(&packetize_test_transport_stream_pes(
        0x0101,
        &mut continuity_counter,
        &pes_packet,
    ));
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_dvb_subtitle_file(
    prefix: &str,
    subtitle_payloads: &[&[u8]],
) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_private_data(
        continuity_counter,
        &build_test_transport_stream_dvb_subtitle_descriptor(*b"eng", 0x10, 0x0123, 0x0456),
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for payload in subtitle_payloads {
        let pes_packet = build_test_transport_stream_private_data_pes_packet(payload);
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_transport_stream_dvb_teletext_file(
    prefix: &str,
    teletext_payloads: &[&[u8]],
) -> PathBuf {
    let mut bytes = Vec::new();
    let mut continuity_counter = 0_u8;
    bytes.extend_from_slice(&build_test_transport_stream_pat_packet(continuity_counter));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    bytes.extend_from_slice(&build_test_transport_stream_pmt_packet_for_private_data(
        continuity_counter,
        &build_test_transport_stream_dvb_teletext_descriptor(*b"eng", 0x10, 0x01),
    ));
    continuity_counter = (continuity_counter + 1) & 0x0F;
    for payload in teletext_payloads {
        let pes_packet = build_test_transport_stream_private_data_pes_packet(payload);
        bytes.extend_from_slice(&packetize_test_transport_stream_pes(
            0x0101,
            &mut continuity_counter,
            &pes_packet,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_vobsub_files(
    prefix: &str,
    start_times_ms: &[u32],
    sample_payloads: &[&[u8]],
) -> (PathBuf, PathBuf) {
    assert_eq!(start_times_ms.len(), sample_payloads.len());

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base =
        std::env::temp_dir().join(format!("mp4forge-{prefix}-{}-{unique}", std::process::id()));
    let idx_path = base.with_extension("idx");
    let sub_path = base.with_extension("sub");

    let mut sub_bytes = Vec::new();
    let mut positions = Vec::with_capacity(sample_payloads.len());
    for (start_ms, payload) in start_times_ms
        .iter()
        .copied()
        .zip(sample_payloads.iter().copied())
    {
        let filepos = u64::try_from(sub_bytes.len()).unwrap();
        positions.push((start_ms, filepos));
        let packet = build_test_vobsub_packet(payload);
        sub_bytes.extend_from_slice(&packetize_test_vobsub_subpicture(
            u64::from(start_ms) * 90,
            0x20,
            &packet,
        ));
    }

    let mut idx = String::from("# VobSub index file, v7 (do not modify this line!)\n#\n");
    idx.push_str("size: 720x480\n");
    idx.push_str(
        "palette: 000000, 101010, 202020, 303030, 404040, 505050, 606060, 707070, 808080, 909090, A0A0A0, B0B0B0, C0C0C0, D0D0D0, E0E0E0, F0F0F0\n",
    );
    idx.push_str("id: en, index: 0\n");
    for (start_ms, filepos) in positions {
        idx.push_str(&format!(
            "timestamp: {}, filepos: {:09X}\n",
            format_vobsub_timestamp_ms(start_ms),
            filepos
        ));
    }

    fs::write(&idx_path, idx.as_bytes()).unwrap();
    fs::write(&sub_path, &sub_bytes).unwrap();
    (idx_path, sub_path)
}

#[cfg(feature = "mux")]
pub fn write_test_program_stream_vobsub_file(
    prefix: &str,
    start_times_ms: &[u32],
    sample_payloads: &[&[u8]],
) -> PathBuf {
    assert_eq!(start_times_ms.len(), sample_payloads.len());

    let mut bytes = build_test_program_stream_pack_header();
    for (start_ms, payload) in start_times_ms
        .iter()
        .copied()
        .zip(sample_payloads.iter().copied())
    {
        let packet = build_test_vobsub_packet(payload);
        bytes.extend_from_slice(&build_test_program_stream_vobsub_pes_packet(
            u64::from(start_ms) * 90,
            0x20,
            &packet,
        ));
    }
    write_temp_file_with_extension(prefix, "ps", &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_ogg_vorbis_file(prefix: &str, audio_payloads: &[&[u8]]) -> PathBuf {
    let serial = 0x564F_5242_u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_ogg_page(
        serial,
        0,
        0x02,
        0,
        &[build_vorbis_identification_packet()],
    ));
    bytes.extend_from_slice(&build_ogg_page(
        serial,
        1,
        0,
        0,
        &[build_vorbis_comment_packet()],
    ));
    bytes.extend_from_slice(&build_ogg_page(
        serial,
        2,
        0,
        0,
        &[build_vorbis_setup_packet()],
    ));
    let mut granule_position = 0_u64;
    for (index, payload) in audio_payloads.iter().enumerate() {
        granule_position += 64;
        let header_type = if index + 1 == audio_payloads.len() {
            0x04
        } else {
            0
        };
        bytes.extend_from_slice(&build_ogg_page(
            serial,
            u32::try_from(index + 3).unwrap(),
            header_type,
            granule_position,
            &[build_vorbis_audio_packet(payload)],
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
fn format_vobsub_timestamp_ms(total_ms: u32) -> String {
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms / 60_000) % 60;
    let seconds = (total_ms / 1_000) % 60;
    let milliseconds = total_ms % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}:{milliseconds:03}")
}

#[cfg(feature = "mux")]
fn build_test_vobsub_packet(payload: &[u8]) -> Vec<u8> {
    let control_offset = 4_u16 + u16::try_from(payload.len()).unwrap();
    let packet_size = control_offset + 6;
    let mut packet = Vec::with_capacity(usize::from(packet_size));
    packet.extend_from_slice(&packet_size.to_be_bytes());
    packet.extend_from_slice(&control_offset.to_be_bytes());
    packet.extend_from_slice(payload);
    packet.extend_from_slice(&0_u16.to_be_bytes());
    packet.extend_from_slice(&control_offset.to_be_bytes());
    packet.extend_from_slice(&[0x00, 0xFF]);
    packet
}

#[cfg(feature = "mux")]
fn packetize_test_vobsub_subpicture(pts: u64, substream_id: u8, data: &[u8]) -> Vec<u8> {
    let ptsbuf = [
        (((pts >> 29) & 0x0E) as u8) | 0x21,
        ((pts >> 22) & 0xFF) as u8,
        (((pts >> 14) & 0xFE) as u8) | 0x01,
        ((pts >> 7) & 0xFF) as u8,
        (((pts << 1) & 0xFE) as u8) | 0x01,
    ];
    let mut packetized = Vec::new();
    let mut remaining = data;
    let mut emit_pts = true;
    while !remaining.is_empty() {
        let mut sector = [0_u8; 0x800];
        sector[..5].copy_from_slice(&[0x00, 0x00, 0x01, 0xBA, 0x40]);

        let mut write = 14usize;
        sector[write..write + 4].copy_from_slice(&[0x00, 0x00, 0x01, 0xBD]);
        write += 4;

        let mut data_len = sector.len() - 14 - 4 - 2 - 3 - 1;
        if emit_pts {
            data_len -= 5;
        }
        let mut pad_len = 0usize;
        if remaining.len() <= data_len {
            pad_len = data_len - remaining.len();
            data_len = remaining.len();
        }

        let pes_header_extension_len =
            if emit_pts { 5 } else { 0 } + usize::from(pad_len < 6) * pad_len;
        let pes_packet_size =
            3 + if emit_pts { 5 } else { 0 } + 1 + data_len + if pad_len < 6 { pad_len } else { 0 };
        sector[write..write + 2]
            .copy_from_slice(&(u16::try_from(pes_packet_size).unwrap()).to_be_bytes());
        write += 2;
        sector[write] = 0x80;
        sector[write + 1] = if emit_pts { 0x80 } else { 0x00 };
        sector[write + 2] = u8::try_from(pes_header_extension_len).unwrap();
        write += 3;

        if emit_pts {
            sector[write..write + 5].copy_from_slice(&ptsbuf);
            write += 5;
        }

        if pad_len < 6 {
            write += pad_len;
        }

        sector[write] = substream_id;
        write += 1;
        sector[write..write + data_len].copy_from_slice(&remaining[..data_len]);
        write += data_len;
        remaining = &remaining[data_len..];

        if pad_len >= 6 {
            let stream_padding = pad_len - 6;
            sector[write..write + 4].copy_from_slice(&[0x00, 0x00, 0x01, 0xBE]);
            sector[write + 4..write + 6]
                .copy_from_slice(&(u16::try_from(stream_padding).unwrap()).to_be_bytes());
        }

        packetized.extend_from_slice(&sector);
        emit_pts = false;
    }
    packetized
}

#[cfg(feature = "mux")]
pub fn write_test_ogg_speex_file(prefix: &str, audio_payloads: &[&[u8]]) -> PathBuf {
    let serial = 0x5350_5858_u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_ogg_page(
        serial,
        0,
        0x02,
        0,
        &[build_speex_header_packet()],
    ));
    bytes.extend_from_slice(&build_ogg_page(serial, 1, 0, 0, &[b"SpeexTags".to_vec()]));
    let mut granule_position = 0_u64;
    for (index, payload) in audio_payloads.iter().enumerate() {
        granule_position += 160;
        let header_type = if index + 1 == audio_payloads.len() {
            0x04
        } else {
            0
        };
        bytes.extend_from_slice(&build_ogg_page(
            serial,
            u32::try_from(index + 2).unwrap(),
            header_type,
            granule_position,
            &[payload.to_vec()],
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_ogg_theora_file(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    let serial = 0x5448_454F_u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_ogg_page(
        serial,
        0,
        0x02,
        0,
        &[build_theora_identification_packet(4, 3)],
    ));
    bytes.extend_from_slice(&build_ogg_page(
        serial,
        1,
        0,
        0,
        &[build_theora_comment_packet()],
    ));
    bytes.extend_from_slice(&build_ogg_page(
        serial,
        2,
        0,
        0,
        &[build_theora_setup_packet()],
    ));
    let mut granule_position = 0_u64;
    for (index, payload) in frame_payloads.iter().enumerate() {
        granule_position += 1;
        let header_type = if index + 1 == frame_payloads.len() {
            0x04
        } else {
            0
        };
        bytes.extend_from_slice(&build_ogg_page(
            serial,
            u32::try_from(index + 3).unwrap(),
            header_type,
            granule_position,
            &[build_theora_frame_packet(payload)],
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
/// Writes one deterministic 1x1 JPEG fixture for direct-ingest mux tests.
pub fn write_test_jpeg_file(prefix: &str) -> PathBuf {
    write_temp_file(prefix, include_bytes!("../fixtures/generated-1x1.jpg"))
}

#[cfg(feature = "mux")]
pub fn write_test_png_file(prefix: &str) -> PathBuf {
    write_temp_file(
        prefix,
        &[
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, b'I', b'D', b'A', b'T', 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
        ],
    )
}

#[cfg(feature = "mux")]
pub fn write_test_iamf_file(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_test_iamf_obu(
        31,
        &build_test_iamf_sequence_header_payload(),
    ));
    bytes.extend_from_slice(&build_test_iamf_obu(
        0,
        &build_test_iamf_codec_config_payload(),
    ));
    bytes.extend_from_slice(&build_test_iamf_obu(
        1,
        &build_test_iamf_audio_element_payload(),
    ));
    for payload in frame_payloads {
        bytes.extend_from_slice(&build_test_iamf_obu(4, &[]));
        bytes.extend_from_slice(&build_test_iamf_obu(5, payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_caf_alac_file(prefix: &str, packets: &[&[u8]]) -> PathBuf {
    assert!(!packets.is_empty());
    let bytes_per_packet = u32::try_from(packets[0].len()).unwrap();
    assert!(bytes_per_packet > 0);
    for packet in &packets[1..] {
        assert_eq!(packet.len(), usize::try_from(bytes_per_packet).unwrap());
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"caff");
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&0_u16.to_be_bytes());

    let desc_payload = build_caf_alac_description_chunk(bytes_per_packet, 1_024, 2, 16, 48_000.0);
    bytes.extend_from_slice(b"desc");
    bytes.extend_from_slice(&u64::try_from(desc_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&desc_payload);

    let cookie = b"alac-cookie";
    bytes.extend_from_slice(b"kuki");
    bytes.extend_from_slice(&u64::try_from(cookie.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(cookie);

    let mut data_payload =
        Vec::with_capacity(4 + packets.iter().map(|packet| packet.len()).sum::<usize>());
    data_payload.extend_from_slice(&0_u32.to_be_bytes());
    for packet in packets {
        data_payload.extend_from_slice(packet);
    }
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&u64::try_from(data_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&data_payload);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_caf_alac_variable_packet_file(prefix: &str, packets: &[&[u8]]) -> PathBuf {
    assert!(!packets.is_empty());
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"caff");
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&0_u16.to_be_bytes());

    let desc_payload = build_caf_alac_description_chunk(0, 4_096, 0, 0, 44_100.0);
    bytes.extend_from_slice(b"desc");
    bytes.extend_from_slice(&u64::try_from(desc_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&desc_payload);

    let cookie = build_caf_alac_magic_cookie(4_096, 16, 1, 44_100);
    bytes.extend_from_slice(b"kuki");
    bytes.extend_from_slice(&u64::try_from(cookie.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&cookie);

    let chan_payload = 0_u32.to_be_bytes();
    bytes.extend_from_slice(b"chan");
    bytes.extend_from_slice(&u64::try_from(chan_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&chan_payload);

    let mut data_payload =
        Vec::with_capacity(4 + packets.iter().map(|packet| packet.len()).sum::<usize>());
    data_payload.extend_from_slice(&0_u32.to_be_bytes());
    for packet in packets {
        data_payload.extend_from_slice(packet);
    }
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&u64::try_from(data_payload.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&data_payload);

    let packet_table = build_caf_packet_table(
        u64::try_from(packets.len()).unwrap(),
        u64::try_from(packets.len()).unwrap() * 4_096,
        0,
        0,
        &packets
            .iter()
            .map(|packet| u32::try_from(packet.len()).unwrap())
            .collect::<Vec<_>>(),
    );
    bytes.extend_from_slice(b"pakt");
    bytes.extend_from_slice(&u64::try_from(packet_table.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(&packet_table);
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_mhas_file(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_mhas_packet(6, &[0xA5]));
    bytes.extend_from_slice(&build_mhas_packet(1, &build_test_mhas_config_payload()));
    for payload in frame_payloads {
        let mut frame_payload = Vec::with_capacity(payload.len() + 1);
        frame_payload.push(0x80);
        frame_payload.extend_from_slice(payload);
        bytes.extend_from_slice(&build_mhas_packet(2, &frame_payload));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_h265_annexb_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    write_temp_file(prefix, &build_test_h265_annexb_bytes(sample_payloads))
}

#[cfg(feature = "mux")]
pub fn write_test_h264_annexb_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    write_temp_file(prefix, &build_test_h264_annexb_bytes(sample_payloads))
}

#[cfg(feature = "mux")]
fn build_test_h264_annexb_bytes(sample_payloads: &[&[u8]]) -> Vec<u8> {
    build_test_h264_annexb_chunks(sample_payloads)
        .into_iter()
        .flatten()
        .collect()
}

#[cfg(feature = "mux")]
fn build_test_h264_annexb_chunks(sample_payloads: &[&[u8]]) -> Vec<Vec<u8>> {
    const START_CODE: &[u8] = &[0, 0, 0, 1];
    const SPS: &[u8] = &[
        0x67, 0x64, 0x00, 0x0c, 0xac, 0xd9, 0x41, 0x41, 0x9f, 0x9f, 0x01, 0x6c, 0x80, 0x00, 0x00,
        0x03, 0x00, 0x80, 0x00, 0x00, 0x0a, 0x07, 0x8a, 0x14, 0xcb,
    ];
    const PPS: &[u8] = &[0x68, 0xeb, 0xec, 0xb2, 0x2c];
    const AUD: &[u8] = &[0x09, 0xf0];

    let mut chunks = Vec::with_capacity(sample_payloads.len());
    for (index, payload) in sample_payloads.iter().enumerate() {
        let mut chunk = Vec::new();
        if index == 0 {
            for nal in [SPS, PPS] {
                chunk.extend_from_slice(START_CODE);
                chunk.extend_from_slice(nal);
            }
        } else {
            chunk.extend_from_slice(START_CODE);
            chunk.extend_from_slice(AUD);
        }
        chunk.extend_from_slice(START_CODE);
        chunk.extend_from_slice(&[0x65, 0x80]);
        chunk.extend_from_slice(payload);
        chunks.push(chunk);
    }
    chunks
}

#[cfg(feature = "mux")]
fn build_test_h264_avc1_chunks(sample_payloads: &[&[u8]]) -> Vec<Vec<u8>> {
    sample_payloads
        .iter()
        .enumerate()
        .map(|(index, payload)| {
            let nal = if index == 0 {
                build_test_h264_idr_nal(payload)
            } else {
                build_test_h264_non_idr_nal(payload)
            };
            let mut chunk = Vec::with_capacity(4 + nal.len());
            chunk.extend_from_slice(&u32::try_from(nal.len()).unwrap().to_be_bytes());
            chunk.extend_from_slice(&nal);
            chunk
        })
        .collect()
}

#[cfg(feature = "mux")]
fn build_test_h264_idr_nal(payload: &[u8]) -> Vec<u8> {
    let mut nal = Vec::with_capacity(payload.len() + 2);
    nal.extend_from_slice(&[0x65, 0x80]);
    nal.extend_from_slice(payload);
    nal
}

#[cfg(feature = "mux")]
fn build_test_h264_non_idr_nal(payload: &[u8]) -> Vec<u8> {
    let mut nal = Vec::with_capacity(payload.len() + 2);
    nal.extend_from_slice(&[0x41, 0x80]);
    nal.extend_from_slice(payload);
    nal
}

#[cfg(feature = "mux")]
fn build_test_avcc_decoder_specific_info() -> Vec<u8> {
    const SPS: &[u8] = &[
        0x67, 0x64, 0x00, 0x0c, 0xac, 0xd9, 0x41, 0x41, 0x9f, 0x9f, 0x01, 0x6c, 0x80, 0x00, 0x00,
        0x03, 0x00, 0x80, 0x00, 0x00, 0x0a, 0x07, 0x8a, 0x14, 0xcb,
    ];
    const PPS: &[u8] = &[0x68, 0xeb, 0xec, 0xb2, 0x2c];

    let mut bytes = vec![1, SPS[1], SPS[2], SPS[3], 0xFF, 0xE1];
    bytes.extend_from_slice(&u16::try_from(SPS.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(SPS);
    bytes.push(1);
    bytes.extend_from_slice(&u16::try_from(PPS.len()).unwrap().to_be_bytes());
    bytes.extend_from_slice(PPS);
    bytes
}

#[cfg(feature = "mux")]
pub fn write_test_h263_file(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for (index, payload) in sample_payloads.iter().enumerate() {
        bytes.extend_from_slice(&build_test_h263_frame(
            u8::try_from(index).unwrap(),
            payload,
        ));
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
fn build_test_h263_frame(temporal_reference: u8, payload: &[u8]) -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    write_test_bits_u64(&mut writer, 0x20, 22);
    write_test_bits_u64(&mut writer, u64::from(temporal_reference), 8);
    write_test_bits_u64(&mut writer, 0, 5);
    write_test_bits_u64(&mut writer, 2, 3);
    write_test_bits_u64(&mut writer, 0, 2);
    let mut bytes = writer.into_inner().unwrap();
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
pub fn write_test_h265_annexb_file_with_timing(prefix: &str, sample_payloads: &[&[u8]]) -> PathBuf {
    write_test_h265_annexb_file_with_sps(
        prefix,
        &[
            0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00,
            0x00, 0x03, 0x00, 0x5d, 0xa0, 0x02, 0x80, 0x80, 0x24, 0x1f, 0x26, 0x59, 0x99, 0xa4,
            0x93, 0x2b, 0xff, 0xc0, 0xd5, 0xc0, 0xd6, 0x40, 0x40, 0x00, 0x00, 0x03, 0x00, 0x40,
            0x00, 0x00, 0x06, 0x02,
        ],
        sample_payloads,
    )
}

#[cfg(feature = "mux")]
fn write_test_h265_annexb_file_with_sps(
    prefix: &str,
    sps: &[u8],
    sample_payloads: &[&[u8]],
) -> PathBuf {
    write_temp_file(
        prefix,
        &build_test_h265_annexb_bytes_with_sps(sps, sample_payloads),
    )
}

#[cfg(feature = "mux")]
fn build_test_h265_annexb_bytes(sample_payloads: &[&[u8]]) -> Vec<u8> {
    build_test_h265_annexb_bytes_with_sps(
        &[
            0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00,
            0x00, 0x03, 0x00, 0x78, 0xa0, 0x03, 0xc0, 0x80, 0x10, 0xe5, 0x96, 0x66, 0x69, 0x24,
            0xca, 0xe0, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x01, 0xe0, 0x80,
        ],
        sample_payloads,
    )
}

#[cfg(feature = "mux")]
fn build_test_h265_annexb_bytes_with_sps(sps: &[u8], sample_payloads: &[&[u8]]) -> Vec<u8> {
    const START_CODE: &[u8] = &[0, 0, 0, 1];
    const AUD: &[u8] = &[0x46, 0x01, 0x50];
    const VPS: &[u8] = &[
        0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00,
        0x03, 0x00, 0x00, 0x03, 0x00, 0x78, 0x99, 0x98, 0x09,
    ];
    const PPS: &[u8] = &[0x44, 0x01, 0xc1, 0x72, 0xb4, 0x62, 0x40];

    let mut bytes = Vec::new();
    for nal in [VPS, sps, PPS] {
        bytes.extend_from_slice(START_CODE);
        bytes.extend_from_slice(nal);
    }
    for (index, payload) in sample_payloads.iter().enumerate() {
        if index != 0 {
            bytes.extend_from_slice(START_CODE);
            bytes.extend_from_slice(AUD);
        }
        bytes.extend_from_slice(START_CODE);
        bytes.extend_from_slice(&[0x26, 0x01]);
        bytes.extend_from_slice(payload);
    }
    bytes
}

#[cfg(feature = "mux")]
pub fn write_test_av1_ivf_file(
    prefix: &str,
    width: u16,
    height: u16,
    frame_timestamps: &[u64],
    frame_payloads: &[&[u8]],
) -> PathBuf {
    write_test_ivf_file(
        prefix,
        *b"AV01",
        IvfHeaderFields {
            width,
            height,
            timescale: 1_000,
            timestamp_scale: 1,
        },
        frame_timestamps,
        frame_payloads,
    )
}

#[cfg(feature = "mux")]
pub fn write_test_av1_obu_file(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    let mut bytes = Vec::new();
    for payload in frame_payloads {
        bytes.extend_from_slice(&build_test_av1_temporal_delimiter_obu());
        bytes.extend_from_slice(payload);
    }
    write_temp_file_with_extension(prefix, "obu", &bytes)
}

#[cfg(feature = "mux")]
pub fn write_test_av1_annex_b_file(prefix: &str, frame_payloads: &[&[u8]]) -> PathBuf {
    write_temp_file_with_extension(
        prefix,
        "av1b",
        &build_test_av1_annex_b_file_bytes(frame_payloads),
    )
}

#[cfg(feature = "mux")]
pub fn build_test_av1_temporal_delimiter_obu() -> Vec<u8> {
    vec![0x12, 0x00]
}

#[cfg(feature = "mux")]
pub fn build_test_av1_annex_b_file_bytes(frame_payloads: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for payload in frame_payloads {
        let obu_units = split_test_av1_obu_units(payload);
        let frame_unit_payload = obu_units
            .iter()
            .flat_map(|obu| {
                let mut bytes = encode_test_leb128(u32::try_from(obu.len()).unwrap());
                bytes.extend_from_slice(obu);
                bytes
            })
            .collect::<Vec<_>>();
        let mut temporal_unit_payload =
            encode_test_leb128(u32::try_from(frame_unit_payload.len()).unwrap());
        temporal_unit_payload.extend_from_slice(&frame_unit_payload);
        bytes.extend_from_slice(&encode_test_leb128(
            u32::try_from(temporal_unit_payload.len()).unwrap(),
        ));
        bytes.extend_from_slice(&temporal_unit_payload);
    }
    bytes
}

#[cfg(feature = "mux")]
pub fn write_test_vp8_ivf_file(
    prefix: &str,
    width: u16,
    height: u16,
    frame_timestamps: &[u64],
    frame_payloads: &[&[u8]],
) -> PathBuf {
    write_test_ivf_file(
        prefix,
        *b"VP80",
        IvfHeaderFields {
            width,
            height,
            timescale: 1_000,
            timestamp_scale: 1,
        },
        frame_timestamps,
        frame_payloads,
    )
}

#[cfg(feature = "mux")]
pub fn write_test_vp9_ivf_file(
    prefix: &str,
    width: u16,
    height: u16,
    frame_timestamps: &[u64],
    frame_payloads: &[&[u8]],
) -> PathBuf {
    write_test_ivf_file(
        prefix,
        *b"VP90",
        IvfHeaderFields {
            width,
            height,
            timescale: 1_000,
            timestamp_scale: 1,
        },
        frame_timestamps,
        frame_payloads,
    )
}

#[cfg(feature = "mux")]
pub fn write_test_vp10_ivf_file(
    prefix: &str,
    width: u16,
    height: u16,
    frame_timestamps: &[u64],
    frame_payloads: &[&[u8]],
) -> PathBuf {
    write_test_ivf_file(
        prefix,
        *b"VP10",
        IvfHeaderFields {
            width,
            height,
            timescale: 1_000,
            timestamp_scale: 1,
        },
        frame_timestamps,
        frame_payloads,
    )
}

#[cfg(feature = "mux")]
pub fn build_test_av1_sequence_header_obu(width: u16, height: u16) -> Vec<u8> {
    let mut payload_writer = BitWriter::new(Vec::new());
    write_test_bits_u64(&mut payload_writer, 0, 3);
    payload_writer.write_bit(true).unwrap();
    payload_writer.write_bit(true).unwrap();
    write_test_bits_u64(&mut payload_writer, 0, 5);
    write_test_bits_u64(&mut payload_writer, 9, 4);
    write_test_bits_u64(&mut payload_writer, 8, 4);
    write_test_bits_u64(&mut payload_writer, u64::from(width.saturating_sub(1)), 10);
    write_test_bits_u64(&mut payload_writer, u64::from(height.saturating_sub(1)), 9);
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut payload_writer, 0, 2);
    payload_writer.write_bit(false).unwrap();
    payload_writer.write_bit(false).unwrap();
    align_test_bit_writer(&mut payload_writer);
    let payload = payload_writer.into_inner().unwrap();

    let mut obu = Vec::with_capacity(2 + payload.len());
    obu.push(0x0A);
    obu.push(u8::try_from(payload.len()).unwrap());
    obu.extend_from_slice(&payload);
    obu
}

#[cfg(feature = "mux")]
fn split_test_av1_obu_units(sample_payload: &[u8]) -> Vec<Vec<u8>> {
    let mut units = Vec::new();
    let mut offset = 0usize;
    while offset < sample_payload.len() {
        let header = sample_payload[offset];
        let extension_flag = (header >> 2) & 0x01 != 0;
        let has_size_field = (header >> 1) & 0x01 != 0;
        assert!(
            has_size_field,
            "test AV1 OBU payloads must use explicit size fields"
        );
        let mut cursor = offset + 1;
        if extension_flag {
            cursor += 1;
        }
        let (payload_size, leb_size) = decode_test_leb128(&sample_payload[cursor..]);
        cursor += leb_size;
        let obu_end = cursor + usize::try_from(payload_size).unwrap();
        units.push(sample_payload[offset..obu_end].to_vec());
        offset = obu_end;
    }
    units
}

#[cfg(feature = "mux")]
fn encode_test_leb128(mut value: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = u8::try_from(value & 0x7F).unwrap();
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            return bytes;
        }
    }
}

#[cfg(feature = "mux")]
fn decode_test_leb128(bytes: &[u8]) -> (u32, usize) {
    let mut value = 0u32;
    let mut shift = 0u32;
    for (index, byte) in bytes.iter().copied().enumerate() {
        value |= u32::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return (value, index + 1);
        }
        shift += 7;
    }
    panic!("unterminated test leb128");
}

#[cfg(feature = "mux")]
pub fn build_test_vp8_keyframe(width: u16, height: u16, profile: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(10 + payload.len());
    let first_partition_size = u32::try_from(payload.len()).unwrap();
    let frame_tag =
        (u32::from(profile & 0x07) << 1) | (1 << 4) | ((first_partition_size & 0x7FFFF) << 5);
    frame.extend_from_slice(&[
        u8::try_from(frame_tag & 0xFF).unwrap(),
        u8::try_from((frame_tag >> 8) & 0xFF).unwrap(),
        u8::try_from((frame_tag >> 16) & 0xFF).unwrap(),
    ]);
    frame.extend_from_slice(&[0x9D, 0x01, 0x2A]);
    frame.extend_from_slice(&(width & 0x3FFF).to_le_bytes());
    frame.extend_from_slice(&(height & 0x3FFF).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

#[cfg(feature = "mux")]
pub fn build_test_vp9_keyframe(width: u16, height: u16, profile: u8) -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    write_test_bits_u64(&mut writer, 0b10, 2);
    writer.write_bit(profile & 0x01 != 0).unwrap();
    writer.write_bit(profile & 0x02 != 0).unwrap();
    if profile == 3 {
        writer.write_bit(false).unwrap();
    }
    writer.write_bit(false).unwrap();
    writer.write_bit(false).unwrap();
    writer.write_bit(true).unwrap();
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, 0x49_83_42, 24);
    if profile >= 2 {
        writer.write_bit(false).unwrap();
    }
    write_test_bits_u64(&mut writer, 1, 3);
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, u64::from(width.saturating_sub(1)), 16);
    write_test_bits_u64(&mut writer, u64::from(height.saturating_sub(1)), 16);
    writer.write_bit(false).unwrap();
    align_test_bit_writer(&mut writer);
    writer.into_inner().unwrap()
}

#[cfg(feature = "mux")]
pub fn build_test_vp10_keyframe(width: u16, height: u16, profile: u8) -> Vec<u8> {
    build_test_vp9_keyframe(width, height, profile)
}

#[cfg(feature = "mux")]
struct IvfHeaderFields {
    width: u16,
    height: u16,
    timescale: u32,
    timestamp_scale: u32,
}

#[cfg(feature = "mux")]
fn write_test_ivf_file(
    prefix: &str,
    codec_fourcc: [u8; 4],
    header: IvfHeaderFields,
    frame_timestamps: &[u64],
    frame_payloads: &[&[u8]],
) -> PathBuf {
    assert_eq!(frame_timestamps.len(), frame_payloads.len());
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"DKIF");
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&32_u16.to_le_bytes());
    bytes.extend_from_slice(&codec_fourcc);
    bytes.extend_from_slice(&header.width.to_le_bytes());
    bytes.extend_from_slice(&header.height.to_le_bytes());
    bytes.extend_from_slice(&header.timescale.to_le_bytes());
    bytes.extend_from_slice(&header.timestamp_scale.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(frame_payloads.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    for (timestamp, payload) in frame_timestamps.iter().zip(frame_payloads.iter()) {
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&timestamp.to_le_bytes());
        bytes.extend_from_slice(payload);
    }
    write_temp_file(prefix, &bytes)
}

#[cfg(feature = "mux")]
fn build_flac_streaminfo_block(
    sample_rate: u32,
    channel_count: u8,
    bits_per_sample: u8,
    total_samples: u64,
) -> [u8; 34] {
    assert!(sample_rate > 0 && sample_rate < (1 << 20));
    assert!((1..=8).contains(&channel_count));
    assert!((1..=32).contains(&bits_per_sample));
    assert!(total_samples < (1_u64 << 36));

    let mut block = [0_u8; 34];
    block[0..2].copy_from_slice(&0x0400_u16.to_be_bytes());
    block[2..4].copy_from_slice(&0x0400_u16.to_be_bytes());
    block[10] = u8::try_from((sample_rate >> 12) & 0xFF).unwrap();
    block[11] = u8::try_from((sample_rate >> 4) & 0xFF).unwrap();
    block[12] = (u8::try_from(sample_rate & 0x0F).unwrap() << 4)
        | (((channel_count - 1) & 0x07) << 1)
        | (((bits_per_sample - 1) >> 4) & 0x01);
    block[13] =
        (((bits_per_sample - 1) & 0x0F) << 4) | u8::try_from((total_samples >> 32) & 0x0F).unwrap();
    block[14] = u8::try_from((total_samples >> 24) & 0xFF).unwrap();
    block[15] = u8::try_from((total_samples >> 16) & 0xFF).unwrap();
    block[16] = u8::try_from((total_samples >> 8) & 0xFF).unwrap();
    block[17] = u8::try_from(total_samples & 0xFF).unwrap();
    block
}

#[cfg(feature = "mux")]
fn build_test_amr_frame(payload: &[u8]) -> Vec<u8> {
    build_test_amr_like_frame(7, 31, payload)
}

#[cfg(feature = "mux")]
fn build_test_latm_frame(use_same_stream_mux: bool, payload: &[u8]) -> Vec<u8> {
    build_test_latm_frame_with_audio_object_type(use_same_stream_mux, payload, 2)
}

#[cfg(feature = "mux")]
fn build_test_usac_latm_frame(use_same_stream_mux: bool, payload: &[u8]) -> Vec<u8> {
    build_test_latm_frame_with_audio_object_type(use_same_stream_mux, payload, 42)
}

#[cfg(feature = "mux")]
fn build_test_latm_frame_with_audio_object_type(
    use_same_stream_mux: bool,
    payload: &[u8],
    audio_object_type: u8,
) -> Vec<u8> {
    build_test_latm_frame_with_options(
        use_same_stream_mux,
        payload,
        audio_object_type,
        false,
        false,
    )
}

#[cfg(feature = "mux")]
fn build_test_latm_frame_with_options(
    use_same_stream_mux: bool,
    payload: &[u8],
    audio_object_type: u8,
    other_data_present: bool,
    crc_check_present: bool,
) -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    writer.write_bit(use_same_stream_mux).unwrap();
    if !use_same_stream_mux {
        writer.write_bit(false).unwrap();
        writer.write_bit(true).unwrap();
        write_test_bits_u64(&mut writer, 0, 6);
        write_test_bits_u64(&mut writer, 0, 4);
        write_test_bits_u64(&mut writer, 0, 3);
        write_test_latm_audio_specific_config(&mut writer, audio_object_type, 3, 2);
        write_test_bits_u64(&mut writer, 0, 3);
        write_test_bits_u64(&mut writer, 0, 8);
        writer.write_bit(other_data_present).unwrap();
        writer.write_bit(crc_check_present).unwrap();
    }
    write_test_latm_payload_length(&mut writer, payload.len());
    for byte in payload {
        write_test_bits_u64(&mut writer, u64::from(*byte), 8);
    }
    align_test_bit_writer(&mut writer);
    let body = writer.into_inner().unwrap();
    let mux_size = u16::try_from(body.len()).unwrap();
    assert!(mux_size < 0x2000);

    let mut frame = Vec::with_capacity(3 + body.len());
    frame.push(0x56);
    frame.push(0xE0 | u8::try_from((mux_size >> 8) & 0x1F).unwrap());
    frame.push(u8::try_from(mux_size & 0x00FF).unwrap());
    frame.extend_from_slice(&body);
    frame
}

#[cfg(feature = "mux")]
fn write_test_latm_audio_specific_config(
    writer: &mut BitWriter<Vec<u8>>,
    audio_object_type: u8,
    sample_rate_index: u8,
    channel_configuration: u8,
) {
    if audio_object_type >= 32 {
        write_test_bits_u64(writer, 31, 5);
        write_test_bits_u64(writer, u64::from(audio_object_type - 32), 6);
    } else {
        write_test_bits_u64(writer, u64::from(audio_object_type), 5);
    }
    write_test_bits_u64(writer, u64::from(sample_rate_index), 4);
    write_test_bits_u64(writer, u64::from(channel_configuration), 4);
    write_test_bits_u64(writer, 0, 3);
}

#[cfg(feature = "mux")]
fn write_test_latm_payload_length(writer: &mut BitWriter<Vec<u8>>, payload_len: usize) {
    let mut remaining = payload_len;
    while remaining >= 255 {
        write_test_bits_u64(writer, 255, 8);
        remaining -= 255;
    }
    write_test_bits_u64(writer, u64::try_from(remaining).unwrap(), 8);
}

#[cfg(feature = "mux")]
fn build_test_truehd_frame(payload: &[u8]) -> Vec<u8> {
    const TRUEHD_TEST_FRAME_HEADER_BYTES: usize = 20;

    let frame_size = u16::try_from(TRUEHD_TEST_FRAME_HEADER_BYTES + payload.len()).unwrap();
    assert_eq!(frame_size & 1, 0, "TrueHD test frame size must be even");

    let mut writer = BitWriter::new(Vec::new());
    write_test_bits_u64(&mut writer, 0, 4);
    write_test_bits_u64(&mut writer, u64::from(frame_size / 2), 12);
    write_test_bits_u64(&mut writer, 0, 16);
    write_test_bits_u64(&mut writer, 0xF872_6FBA, 32);
    write_test_bits_u64(&mut writer, 0, 4);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 2);
    write_test_bits_u64(&mut writer, 0, 2);
    write_test_bits_u64(&mut writer, 0, 2);
    write_test_bits_u64(&mut writer, 0, 5);
    write_test_bits_u64(&mut writer, 0, 2);
    write_test_bits_u64(&mut writer, 0, 13);
    write_test_bits_u64(&mut writer, 0xB752, 16);
    write_test_bits_u64(&mut writer, 0, 16);
    write_test_bits_u64(&mut writer, 0, 16);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 120, 15);
    align_test_bit_writer(&mut writer);
    let mut frame = writer.into_inner().unwrap();
    assert_eq!(frame.len(), TRUEHD_TEST_FRAME_HEADER_BYTES);
    frame.extend_from_slice(payload);
    frame
}

#[cfg(feature = "mux")]
fn build_test_amr_wb_frame(payload: &[u8]) -> Vec<u8> {
    build_test_amr_like_frame(8, 60, payload)
}

#[cfg(feature = "mux")]
struct TestQcpFileSpec<'a> {
    codec: TestQcpCodecKind,
    decoder_version: u8,
    packet_size: u16,
    block_size: u16,
    sample_rate: u16,
    rate_entries: &'a [(u8, u8)],
    rate_flag: u32,
}

#[cfg(feature = "mux")]
fn build_test_qcp_file_bytes(spec: TestQcpFileSpec<'_>, packets: &[Vec<u8>]) -> Vec<u8> {
    let mut fmt_payload = Vec::with_capacity(150);
    fmt_payload.push(1);
    fmt_payload.push(0);
    fmt_payload.extend_from_slice(test_qcp_codec_guid(spec.codec));
    fmt_payload.extend_from_slice(&u16::from(spec.decoder_version).to_le_bytes());
    let mut name = [0_u8; 80];
    let label: &[u8] = match spec.codec {
        TestQcpCodecKind::Qcelp => b"QCELP",
        TestQcpCodecKind::Evrc => b"EVRC",
        TestQcpCodecKind::Smv => b"SMV",
    };
    name[..label.len()].copy_from_slice(label);
    fmt_payload.extend_from_slice(&name);
    let avg_bps = if packets.is_empty() {
        0
    } else {
        let avg = packets
            .iter()
            .map(|packet| packet.len() as u64)
            .sum::<u64>()
            * 8
            * u64::from(spec.sample_rate)
            / (u64::from(spec.block_size) * u64::try_from(packets.len()).unwrap());
        u16::try_from(avg).unwrap_or(u16::MAX)
    };
    fmt_payload.extend_from_slice(&avg_bps.to_le_bytes());
    fmt_payload.extend_from_slice(&spec.packet_size.to_le_bytes());
    fmt_payload.extend_from_slice(&spec.block_size.to_le_bytes());
    fmt_payload.extend_from_slice(&spec.sample_rate.to_le_bytes());
    fmt_payload.extend_from_slice(&16_u16.to_le_bytes());
    fmt_payload.extend_from_slice(
        &u32::try_from(spec.rate_entries.len())
            .unwrap()
            .to_le_bytes(),
    );
    for index in 0..8 {
        if let Some((rate_index, payload_size)) = spec.rate_entries.get(index) {
            fmt_payload.push(*payload_size);
            fmt_payload.push(*rate_index);
        } else {
            fmt_payload.extend_from_slice(&[0, 0]);
        }
    }
    fmt_payload.extend_from_slice(&[0_u8; 20]);
    debug_assert_eq!(fmt_payload.len(), 150);

    let mut vrat_payload = Vec::with_capacity(8);
    vrat_payload.extend_from_slice(&spec.rate_flag.to_le_bytes());
    vrat_payload.extend_from_slice(&u32::from(spec.packet_size).to_le_bytes());

    let mut data_payload = Vec::new();
    for packet in packets {
        data_payload.extend_from_slice(packet);
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(b"QLCM");
    append_test_riff_chunk(&mut bytes, b"fmt ", &fmt_payload);
    append_test_riff_chunk(&mut bytes, b"vrat", &vrat_payload);
    append_test_riff_chunk(&mut bytes, b"data", &data_payload);
    let riff_size = u32::try_from(bytes.len() - 8).unwrap();
    bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
    bytes
}

#[cfg(feature = "mux")]
fn append_test_riff_chunk(bytes: &mut Vec<u8>, chunk_type: &[u8; 4], payload: &[u8]) {
    bytes.extend_from_slice(chunk_type);
    bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(payload);
    if !payload.len().is_multiple_of(2) {
        bytes.push(0);
    }
}

#[cfg(feature = "mux")]
fn test_qcp_codec_guid(codec: TestQcpCodecKind) -> &'static [u8; 16] {
    match codec {
        TestQcpCodecKind::Qcelp => {
            b"\x41\x6D\x7F\x5E\x15\xB1\xD0\x11\xBA\x91\x00\x80\x5F\xB4\xB9\x7E"
        }
        TestQcpCodecKind::Evrc => {
            b"\x8D\xD4\x89\xE6\x76\x90\xB5\x46\x91\xEF\x73\x6A\x51\x00\xCE\xB4"
        }
        TestQcpCodecKind::Smv => {
            b"\x75\x2B\x7C\x8D\x97\xA7\x46\xED\x98\x5E\xD5\x3C\x8C\xC7\x5F\x84"
        }
    }
}

#[cfg(feature = "mux")]
fn build_test_amr_like_frame(frame_type: u8, payload_len: usize, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + payload_len);
    frame.push((frame_type & 0x0F) << 3);
    frame.extend((0..payload_len).map(|index| payload.get(index).copied().unwrap_or(index as u8)));
    frame
}

#[cfg(feature = "mux")]
fn build_flac_vorbis_comment_block() -> Vec<u8> {
    let mut block = Vec::new();
    block.push(0x84);
    block.extend_from_slice(&8_u32.to_be_bytes()[1..]);
    block.extend_from_slice(&0_u32.to_le_bytes());
    block.extend_from_slice(&0_u32.to_le_bytes());
    block
}

#[cfg(feature = "mux")]
pub fn build_test_flac_frame(seed_payload: &[u8]) -> Vec<u8> {
    build_test_flac_frame_with_block_size(seed_payload, 1_024)
}

#[cfg(feature = "mux")]
pub fn build_test_flac_frame_with_block_size(seed_payload: &[u8], block_size: u32) -> Vec<u8> {
    assert!((1..=u32::from(u16::MAX) + 1).contains(&block_size));
    let mut writer = BitWriter::new(Vec::new());
    write_test_bits_u64(&mut writer, 0x7FFC, 15);
    writer.write_bit(false).unwrap();
    if block_size == 1_024 {
        write_test_bits_u64(&mut writer, 10, 4);
    } else {
        write_test_bits_u64(&mut writer, 7, 4);
    }
    write_test_bits_u64(&mut writer, 0, 4);
    write_test_bits_u64(&mut writer, 1, 4);
    write_test_bits_u64(&mut writer, 4, 3);
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, 0, 8);
    if block_size != 1_024 {
        write_test_bits_u64(&mut writer, u64::from(block_size - 1), 16);
    }
    align_test_bit_writer(&mut writer);
    let mut frame = writer.into_inner().unwrap();
    let header_crc = flac_crc8_for_test(&frame);
    frame.push(header_crc);

    let left_sample = u16::from(*seed_payload.first().unwrap_or(&0x11));
    let right_sample = u16::from(*seed_payload.get(1).unwrap_or(&0x22));

    let mut subframe_writer = BitWriter::new(Vec::new());
    for sample in [left_sample, right_sample] {
        subframe_writer.write_bit(false).unwrap();
        write_test_bits_u64(&mut subframe_writer, 0, 6);
        subframe_writer.write_bit(false).unwrap();
        write_test_bits_u64(&mut subframe_writer, u64::from(sample), 16);
    }
    align_test_bit_writer(&mut subframe_writer);
    frame.extend_from_slice(&subframe_writer.into_inner().unwrap());
    let footer_crc = flac_crc16_for_test(&frame);
    frame.extend_from_slice(&footer_crc.to_be_bytes());
    frame
}

#[cfg(feature = "mux")]
fn build_test_mhas_config_payload() -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    write_test_bits_u64(&mut writer, 12, 8);
    write_test_bits_u64(&mut writer, 3, 5);
    write_test_bits_u64(&mut writer, 1, 3);
    writer.write_bit(false).unwrap();
    writer.write_bit(false).unwrap();
    write_test_bits_u64(&mut writer, 1, 2);
    write_test_mhas_escaped_value(&mut writer, 1, 5, 8, 16);
    align_test_bit_writer(&mut writer);
    writer.into_inner().unwrap()
}

#[cfg(feature = "mux")]
fn build_mhas_packet(packet_type: u64, payload: &[u8]) -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    write_test_mhas_escaped_value(&mut writer, packet_type, 3, 8, 8);
    write_test_mhas_escaped_value(&mut writer, 0, 2, 8, 32);
    write_test_mhas_escaped_value(
        &mut writer,
        u64::try_from(payload.len()).unwrap(),
        11,
        24,
        24,
    );
    align_test_bit_writer(&mut writer);
    let mut packet = writer.into_inner().unwrap();
    packet.extend_from_slice(payload);
    packet
}

#[cfg(feature = "mux")]
fn write_test_mhas_escaped_value(
    writer: &mut BitWriter<Vec<u8>>,
    value: u64,
    first_width: usize,
    second_width: usize,
    third_width: usize,
) {
    let first_max = (1_u64 << first_width) - 1;
    if value < first_max {
        write_test_bits_u64(writer, value, first_width);
        return;
    }
    write_test_bits_u64(writer, first_max, first_width);
    let remainder = value - first_max;
    let second_max = (1_u64 << second_width) - 1;
    if remainder < second_max {
        write_test_bits_u64(writer, remainder, second_width);
        return;
    }
    write_test_bits_u64(writer, second_max, second_width);
    write_test_bits_u64(writer, remainder - second_max, third_width);
}

#[cfg(feature = "mux")]
fn write_test_bits_u64(writer: &mut BitWriter<Vec<u8>>, value: u64, width: usize) {
    writer.write_bits(&value.to_be_bytes(), width).unwrap();
}

#[cfg(feature = "mux")]
fn align_test_bit_writer(writer: &mut BitWriter<Vec<u8>>) {
    while !writer.is_aligned() {
        writer.write_bit(false).unwrap();
    }
}

#[cfg(feature = "mux")]
fn build_opus_head_packet(channel_count: u8) -> Vec<u8> {
    let mut packet = Vec::with_capacity(19);
    packet.extend_from_slice(b"OpusHead");
    packet.push(1);
    packet.push(channel_count);
    packet.extend_from_slice(&312_u16.to_le_bytes());
    packet.extend_from_slice(&48_000_u32.to_le_bytes());
    packet.extend_from_slice(&0_i16.to_le_bytes());
    packet.push(0);
    packet
}

#[cfg(feature = "mux")]
fn build_vorbis_identification_packet() -> Vec<u8> {
    let mut packet = Vec::with_capacity(30);
    packet.push(0x01);
    packet.extend_from_slice(b"vorbis");
    packet.extend_from_slice(&0_u32.to_le_bytes());
    packet.push(2);
    packet.extend_from_slice(&48_000_u32.to_le_bytes());
    packet.extend_from_slice(&0_i32.to_le_bytes());
    packet.extend_from_slice(&0_i32.to_le_bytes());
    packet.extend_from_slice(&0_i32.to_le_bytes());
    packet.push(0x76);
    packet.push(1);
    packet
}

#[cfg(feature = "mux")]
fn build_vorbis_comment_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    packet.push(0x03);
    packet.extend_from_slice(b"vorbis");
    packet
}

#[cfg(feature = "mux")]
fn build_vorbis_setup_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    packet.push(0x05);
    packet.extend_from_slice(b"vorbis");

    let mut writer = TestLsbBitWriter::default();
    writer.write(0, 8);
    writer.write(0, 24);
    writer.write(1, 16);
    writer.write(1, 24);
    writer.write(0, 1);
    writer.write(0, 1);
    writer.write(0, 5);
    writer.write(0, 4);
    writer.write(0, 6);
    writer.write(0, 16);
    writer.write(0, 6);
    writer.write(0, 16);
    writer.write(0, 8);
    writer.write(0, 16);
    writer.write(0, 16);
    writer.write(0, 6);
    writer.write(0, 8);
    writer.write(0, 4);
    writer.write(0, 8);
    writer.write(0, 6);
    writer.write(0, 16);
    writer.write(0, 24);
    writer.write(0, 24);
    writer.write(0, 24);
    writer.write(0, 6);
    writer.write(0, 8);
    writer.write(0, 3);
    writer.write(0, 1);
    writer.write(0, 6);
    writer.write(0, 16);
    writer.write(0, 1);
    writer.write(0, 1);
    writer.write(0, 2);
    writer.write(0, 8);
    writer.write(0, 8);
    writer.write(0, 8);
    writer.write(1, 6);
    writer.write(0, 1);
    writer.write(0, 16);
    writer.write(0, 16);
    writer.write(0, 8);
    writer.write(1, 1);
    writer.write(0, 16);
    writer.write(0, 16);
    writer.write(0, 8);

    packet.extend_from_slice(&writer.finish());
    packet
}

#[cfg(feature = "mux")]
fn build_vorbis_audio_packet(payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(payload.len() + 1);
    packet.push(0x02);
    packet.extend_from_slice(payload);
    packet
}

#[cfg(feature = "mux")]
fn build_speex_header_packet() -> Vec<u8> {
    let mut packet = vec![0_u8; 80];
    packet[..8].copy_from_slice(b"Speex   ");
    packet[8..28].copy_from_slice(b"mp4forge-test\0\0\0\0\0\0\0");
    packet[28..32].copy_from_slice(&1_u32.to_le_bytes());
    packet[32..36].copy_from_slice(&80_u32.to_le_bytes());
    packet[36..40].copy_from_slice(&16_000_u32.to_le_bytes());
    packet[40..44].copy_from_slice(&0_u32.to_le_bytes());
    packet[44..48].copy_from_slice(&1_u32.to_le_bytes());
    packet[48..52].copy_from_slice(&1_u32.to_le_bytes());
    packet[52..56].copy_from_slice(&0_i32.to_le_bytes());
    packet[56..60].copy_from_slice(&160_u32.to_le_bytes());
    packet[60..64].copy_from_slice(&0_u32.to_le_bytes());
    packet[64..68].copy_from_slice(&1_u32.to_le_bytes());
    packet[68..72].copy_from_slice(&0_u32.to_le_bytes());
    packet[72..76].copy_from_slice(&0_u32.to_le_bytes());
    packet[76..80].copy_from_slice(&0_u32.to_le_bytes());
    packet
}

#[cfg(feature = "mux")]
fn build_theora_identification_packet(sar_num: u32, sar_den: u32) -> Vec<u8> {
    let mut packet = vec![0_u8; 42];
    packet[0] = 0x80;
    packet[1..7].copy_from_slice(b"theora");
    packet[7] = 3;
    packet[10..12].copy_from_slice(&(320_u16 / 16).to_be_bytes());
    packet[12..14].copy_from_slice(&(240_u16 / 16).to_be_bytes());
    packet[22..26].copy_from_slice(&30_000_u32.to_be_bytes());
    packet[26..30].copy_from_slice(&1_001_u32.to_be_bytes());
    packet[30] = u8::try_from((sar_num >> 16) & 0xFF).unwrap();
    packet[31] = u8::try_from((sar_num >> 8) & 0xFF).unwrap();
    packet[32] = u8::try_from(sar_num & 0xFF).unwrap();
    packet[33] = u8::try_from((sar_den >> 16) & 0xFF).unwrap();
    packet[34] = u8::try_from((sar_den >> 8) & 0xFF).unwrap();
    packet[35] = u8::try_from(sar_den & 0xFF).unwrap();
    packet
}

#[cfg(feature = "mux")]
fn build_theora_comment_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    packet.push(0x81);
    packet.extend_from_slice(b"theora");
    packet
}

#[cfg(feature = "mux")]
fn build_theora_setup_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    packet.push(0x82);
    packet.extend_from_slice(b"theora");
    packet
}

#[cfg(feature = "mux")]
fn build_theora_frame_packet(payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(payload.len() + 1);
    packet.push(0x00);
    packet.extend_from_slice(payload);
    packet
}

#[cfg(feature = "mux")]
fn build_test_iamf_sequence_header_payload() -> Vec<u8> {
    let mut payload = Vec::with_capacity(6);
    payload.extend_from_slice(b"iamf");
    payload.push(0);
    payload.push(0);
    payload
}

#[cfg(feature = "mux")]
fn build_test_iamf_codec_config_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    append_leb128_for_test(&mut payload, 0);
    payload.extend_from_slice(b"Opus");
    append_leb128_for_test(&mut payload, 960);
    payload.extend_from_slice(&0_i16.to_be_bytes());
    payload
}

#[cfg(feature = "mux")]
fn build_test_iamf_audio_element_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    append_leb128_for_test(&mut payload, 0);
    payload.push(0);
    append_leb128_for_test(&mut payload, 0);
    append_leb128_for_test(&mut payload, 1);
    payload
}

#[cfg(feature = "mux")]
fn build_test_iamf_obu(obu_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(obu_type << 3);
    append_leb128_for_test(&mut bytes, u64::try_from(payload.len()).unwrap());
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
fn build_ogg_page(
    serial: u32,
    sequence_number: u32,
    header_type: u8,
    granule_position: u64,
    packets: &[Vec<u8>],
) -> Vec<u8> {
    let mut lacing_values = Vec::new();
    let mut payload = Vec::new();
    for packet in packets {
        let mut remaining = packet.len();
        while remaining >= 255 {
            lacing_values.push(255_u8);
            remaining -= 255;
        }
        lacing_values.push(u8::try_from(remaining).unwrap());
        payload.extend_from_slice(packet);
    }

    let mut page = Vec::with_capacity(27 + lacing_values.len() + payload.len());
    page.extend_from_slice(b"OggS");
    page.push(0);
    page.push(header_type);
    page.extend_from_slice(&granule_position.to_le_bytes());
    page.extend_from_slice(&serial.to_le_bytes());
    page.extend_from_slice(&sequence_number.to_le_bytes());
    page.extend_from_slice(&0_u32.to_le_bytes());
    page.push(u8::try_from(lacing_values.len()).unwrap());
    page.extend_from_slice(&lacing_values);
    page.extend_from_slice(&payload);
    let crc = compute_ogg_page_crc_for_test(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

#[cfg(feature = "mux")]
fn compute_ogg_page_crc_for_test(page_bytes: &[u8]) -> u32 {
    let mut crc = 0_u32;
    for byte in page_bytes {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(feature = "mux")]
fn append_leb128_for_test(bytes: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = u8::try_from(value & 0x7F).unwrap();
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            break;
        }
    }
}

#[cfg(feature = "mux")]
#[derive(Default)]
struct TestLsbBitWriter {
    bytes: Vec<u8>,
    current: u8,
    bit_offset: u8,
}

#[cfg(feature = "mux")]
impl TestLsbBitWriter {
    fn write(&mut self, mut value: u32, width: u8) {
        for _ in 0..width {
            if value & 1 != 0 {
                self.current |= 1 << self.bit_offset;
            }
            self.bit_offset += 1;
            if self.bit_offset == 8 {
                self.bytes.push(self.current);
                self.current = 0;
                self.bit_offset = 0;
            }
            value >>= 1;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bit_offset != 0 {
            self.bytes.push(self.current);
        }
        self.bytes
    }
}

#[cfg(feature = "mux")]
fn flac_crc8_for_test(data: &[u8]) -> u8 {
    let mut crc = 0_u8;
    for byte in data {
        crc ^= *byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(feature = "mux")]
fn flac_crc16_for_test(data: &[u8]) -> u16 {
    let mut crc = 0_u16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(feature = "mux")]
fn build_caf_alac_description_chunk(
    bytes_per_packet: u32,
    frames_per_packet: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    sample_rate: f64,
) -> [u8; 32] {
    let mut bytes = [0_u8; 32];
    bytes[..8].copy_from_slice(&sample_rate.to_bits().to_be_bytes());
    bytes[8..12].copy_from_slice(b"alac");
    bytes[16..20].copy_from_slice(&bytes_per_packet.to_be_bytes());
    bytes[20..24].copy_from_slice(&frames_per_packet.to_be_bytes());
    bytes[24..28].copy_from_slice(&channels_per_frame.to_be_bytes());
    bytes[28..32].copy_from_slice(&bits_per_channel.to_be_bytes());
    bytes
}

#[cfg(feature = "mux")]
fn build_caf_alac_magic_cookie(
    frame_length: u32,
    bit_depth: u8,
    channel_count: u8,
    sample_rate: u32,
) -> Vec<u8> {
    let mut cookie = Vec::new();
    cookie.extend_from_slice(&12_u32.to_be_bytes());
    cookie.extend_from_slice(b"frma");
    cookie.extend_from_slice(b"alac");

    let mut payload = Vec::with_capacity(28);
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&frame_length.to_be_bytes());
    payload.push(0);
    payload.push(bit_depth);
    payload.push(40);
    payload.push(10);
    payload.push(14);
    payload.push(channel_count);
    payload.extend_from_slice(&0_u16.to_be_bytes());
    payload.extend_from_slice(&(frame_length * u32::from(channel_count) * 2).to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&sample_rate.to_be_bytes());

    cookie.extend_from_slice(&u32::try_from(payload.len() + 8).unwrap().to_be_bytes());
    cookie.extend_from_slice(b"alac");
    cookie.extend_from_slice(&payload);
    cookie
}

#[cfg(feature = "mux")]
fn build_caf_packet_table(
    number_packets: u64,
    number_valid_frames: u64,
    priming_frames: u32,
    remainder_frames: u32,
    packet_sizes: &[u32],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&number_packets.to_be_bytes());
    bytes.extend_from_slice(&number_valid_frames.to_be_bytes());
    bytes.extend_from_slice(&priming_frames.to_be_bytes());
    bytes.extend_from_slice(&remainder_frames.to_be_bytes());
    for packet_size in packet_sizes {
        bytes.extend_from_slice(&encode_caf_packet_size_vlint(*packet_size));
    }
    bytes
}

#[cfg(feature = "mux")]
fn encode_caf_packet_size_vlint(value: u32) -> Vec<u8> {
    let mut parts = Vec::new();
    let mut remaining = value;
    parts.push(u8::try_from(remaining & 0x7F).unwrap());
    remaining >>= 7;
    while remaining != 0 {
        parts.push(u8::try_from(remaining & 0x7F).unwrap() | 0x80);
        remaining >>= 7;
    }
    parts.reverse();
    parts
}

#[cfg(feature = "mux")]
fn build_adts_frame(payload: &[u8]) -> Vec<u8> {
    let profile = 1_u8;
    let sampling_frequency_index = 4_u8;
    let channel_configuration = 2_u8;
    let frame_length = payload.len() + 7;

    let mut header = [0_u8; 7];
    header[0] = 0xFF;
    header[1] = 0xF1;
    header[2] =
        (profile << 6) | (sampling_frequency_index << 2) | ((channel_configuration >> 2) & 0x01);
    header[3] =
        ((channel_configuration & 0x03) << 6) | u8::try_from((frame_length >> 11) & 0x03).unwrap();
    header[4] = u8::try_from((frame_length >> 3) & 0xFF).unwrap();
    header[5] = (u8::try_from(frame_length & 0x07).unwrap() << 5) | 0x1F;
    header[6] = 0xFC;

    let mut frame = header.to_vec();
    frame.extend_from_slice(payload);
    frame
}

#[cfg(feature = "mux")]
fn build_mp3_frame(payload: &[u8]) -> Vec<u8> {
    const FRAME_LENGTH: usize = 384;
    assert!(payload.len() <= FRAME_LENGTH - 4);
    let mut frame = vec![0_u8; FRAME_LENGTH];
    frame[0] = 0xFF;
    frame[1] = 0xFB;
    frame[2] = 0x94;
    frame[3] = 0x00;
    frame[4..4 + payload.len()].copy_from_slice(payload);
    frame
}

#[cfg(feature = "mux")]
fn build_mp2_frame(payload: &[u8]) -> Vec<u8> {
    const FRAME_LENGTH: usize = 1_152;
    assert!(payload.len() <= FRAME_LENGTH - 4);
    let mut frame = vec![0_u8; FRAME_LENGTH];
    frame[0] = 0xFF;
    frame[1] = 0xFD;
    frame[2] = 0xE4;
    frame[3] = 0x44;
    frame[4..4 + payload.len()].copy_from_slice(payload);
    frame
}

#[cfg(feature = "mux")]
fn build_mp3_frame_44100(payload: &[u8]) -> Vec<u8> {
    const FRAME_LENGTH: usize = 417;
    assert!(payload.len() <= FRAME_LENGTH - 4);
    let mut frame = vec![0_u8; FRAME_LENGTH];
    frame[0] = 0xFF;
    frame[1] = 0xFB;
    frame[2] = 0x90;
    frame[3] = 0x00;
    frame[4..4 + payload.len()].copy_from_slice(payload);
    frame
}

#[cfg(feature = "mux")]
fn build_id3v2_tag(payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= 0x0FFF_FFFF);
    let size = payload.len();
    let mut tag = vec![
        b'I',
        b'D',
        b'3',
        3,
        0,
        0,
        u8::try_from((size >> 21) & 0x7F).unwrap(),
        u8::try_from((size >> 14) & 0x7F).unwrap(),
        u8::try_from((size >> 7) & 0x7F).unwrap(),
        u8::try_from(size & 0x7F).unwrap(),
    ];
    tag.extend_from_slice(payload);
    tag
}

#[cfg(feature = "mux")]
fn build_ac3_frame(payload: &[u8]) -> Vec<u8> {
    const FRAME_LENGTH: usize = 256;
    assert!(payload.len() <= FRAME_LENGTH - 7);
    let mut frame = vec![0_u8; FRAME_LENGTH];
    frame[0] = 0x0B;
    frame[1] = 0x77;
    frame[4] = 0x08;
    frame[5] = 0x40;
    frame[6] = 0x44;
    frame[7..7 + payload.len()].copy_from_slice(payload);
    frame
}

#[cfg(feature = "mux")]
fn build_ac3_44100_frame(payload: &[u8]) -> Vec<u8> {
    const FRAME_LENGTH: usize = 138;
    assert!(payload.len() <= FRAME_LENGTH - 7);
    let mut frame = vec![0_u8; FRAME_LENGTH];
    frame[0] = 0x0B;
    frame[1] = 0x77;
    frame[4] = 0x40;
    frame[5] = 0x40;
    frame[6] = 0x44;
    frame[7..7 + payload.len()].copy_from_slice(payload);
    frame
}

#[cfg(feature = "mux")]
fn build_eac3_frame(payload: &[u8]) -> Vec<u8> {
    const FRAME_LENGTH: usize = 64;
    assert!(payload.len() <= FRAME_LENGTH - 6);
    let mut header_writer = BitWriter::new(Vec::new());
    header_writer.write_bits(&[0_u8], 2).unwrap();
    header_writer.write_bits(&[0_u8], 3).unwrap();
    header_writer
        .write_bits(
            &u16::try_from((FRAME_LENGTH / 2) - 1).unwrap().to_be_bytes(),
            11,
        )
        .unwrap();
    header_writer.write_bits(&[0_u8], 2).unwrap();
    header_writer.write_bits(&[3_u8], 2).unwrap();
    header_writer.write_bits(&[2_u8], 3).unwrap();
    header_writer.write_bits(&[1_u8], 1).unwrap();
    header_writer.write_bits(&[16_u8], 5).unwrap();
    header_writer.write_bits(&[0_u8], 3).unwrap();
    let header_suffix = header_writer.into_inner().unwrap();

    let mut frame = vec![0_u8; FRAME_LENGTH];
    frame[0] = 0x0B;
    frame[1] = 0x77;
    frame[2..2 + header_suffix.len()].copy_from_slice(&header_suffix);
    frame[6..6 + payload.len()].copy_from_slice(payload);
    frame
}

#[cfg(feature = "mux")]
fn build_eac3_dependent_substream_frame(payload: &[u8]) -> Vec<u8> {
    const FRAME_LENGTH: usize = 64;
    let mut header_writer = BitWriter::new(Vec::new());
    header_writer.write_bits(&[1_u8], 2).unwrap();
    header_writer.write_bits(&[0_u8], 3).unwrap();
    header_writer
        .write_bits(
            &u16::try_from((FRAME_LENGTH / 2) - 1).unwrap().to_be_bytes(),
            11,
        )
        .unwrap();
    header_writer.write_bits(&[0_u8], 2).unwrap();
    header_writer.write_bits(&[3_u8], 2).unwrap();
    header_writer.write_bits(&[2_u8], 3).unwrap();
    header_writer.write_bits(&[1_u8], 1).unwrap();
    header_writer.write_bits(&[16_u8], 5).unwrap();
    header_writer.write_bits(&[0_u8], 5).unwrap();
    header_writer.write_bits(&[0_u8], 1).unwrap();
    header_writer.write_bits(&[1_u8], 1).unwrap();
    header_writer.write_bits(&(1_u16 << 9).to_be_bytes(), 16).unwrap();
    header_writer.write_bits(&[0_u8], 1).unwrap();
    header_writer.write_bits(&[0_u8], 1).unwrap();
    header_writer.write_bits(&[0_u8], 1).unwrap();
    align_test_bit_writer(&mut header_writer);
    let header_suffix = header_writer.into_inner().unwrap();
    assert!(payload.len() <= FRAME_LENGTH - 2 - header_suffix.len());

    let mut frame = vec![0_u8; FRAME_LENGTH];
    frame[0] = 0x0B;
    frame[1] = 0x77;
    frame[2..2 + header_suffix.len()].copy_from_slice(&header_suffix);
    let payload_offset = 2 + header_suffix.len();
    frame[payload_offset..payload_offset + payload.len()].copy_from_slice(payload);
    frame
}

#[cfg(feature = "mux")]
fn build_dts_frame(seed: usize) -> Vec<u8> {
    const FRAME_LENGTH: usize = 2_048;
    let mut writer = BitWriter::new(Vec::new());
    write_test_bits_u64(&mut writer, 0x7FFE_8001, 32);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 5);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 31, 7);
    write_test_bits_u64(&mut writer, u64::try_from(FRAME_LENGTH - 1).unwrap(), 14);
    write_test_bits_u64(&mut writer, 2, 6);
    write_test_bits_u64(&mut writer, 13, 4);
    write_test_bits_u64(&mut writer, 15, 5);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 3);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 2);
    align_test_bit_writer(&mut writer);
    let mut frame = writer.into_inner().unwrap();
    frame.resize(FRAME_LENGTH, 0);
    for (offset, byte) in frame[11..].iter_mut().enumerate() {
        *byte = u8::try_from((seed + offset) & 0xFF).unwrap();
    }
    frame
}

#[cfg(feature = "mux")]
fn swap_test_dts_16bit_words(frame: &[u8]) -> Vec<u8> {
    assert!(frame.len().is_multiple_of(2));
    let mut swapped = vec![0_u8; frame.len()];
    for (index, chunk) in frame.chunks_exact(2).enumerate() {
        swapped[index * 2] = chunk[1];
        swapped[index * 2 + 1] = chunk[0];
    }
    swapped
}

#[cfg(feature = "mux")]
fn pack_test_dts_14bit_words(frame: &[u8], little_endian: bool) -> Vec<u8> {
    let packed_word_count = (frame.len() * 8).div_ceil(14);
    let mut words = Vec::with_capacity(packed_word_count * 2);
    let mut bit_buffer = 0_u64;
    let mut buffered_bits = 0usize;
    let mut word_index = 0usize;
    for &byte in frame {
        bit_buffer = (bit_buffer << 8) | u64::from(byte);
        buffered_bits += 8;
        while buffered_bits >= 14 {
            buffered_bits -= 14;
            let mut payload = ((bit_buffer >> buffered_bits) & 0x3FFF) as u16;
            if word_index != 0 {
                payload |= 0xC000;
            }
            let bytes = if little_endian {
                payload.to_le_bytes()
            } else {
                payload.to_be_bytes()
            };
            words.extend_from_slice(&bytes);
            bit_buffer &= (1_u64 << buffered_bits).saturating_sub(1);
            word_index += 1;
        }
    }
    if buffered_bits != 0 {
        let mut payload = ((bit_buffer << (14 - buffered_bits)) & 0x3FFF) as u16;
        if word_index != 0 {
            payload |= 0xC000;
        }
        let bytes = if little_endian {
            payload.to_le_bytes()
        } else {
            payload.to_be_bytes()
        };
        words.extend_from_slice(&bytes);
    }
    words
}

#[cfg(feature = "mux")]
const TEST_AC4_FRAME_HEX: &str = concat!(
    "ac41ffff00015cbfcee7984004a7012e2c20304d805c8458d0a0c06013b58354cb613912144b0232be85",
    "4b4800025c71fd3eaacd4a86324c1498a4bd6021dfa8b016b42115ba6b684770fd34e31a264f66703f14",
    "090541b22397fd7c837ef68f05211a79862d48d5c46d87857bedd9f69bbdb26682bcf49b036bccb100ab84",
    "4568e5a54fc32e4302233b9144cb4bd0ca86c64794cf4e7eca5191e8d8c48ccef686868ae56b5f5e416097",
    "07ad77775b5bfa5b61bff5f32ed963f6caee5ac968a743e60e578f5a4892c90101e18a7246f88c51161028",
    "870564d088f0799f9d11701ecd86f202692868b8649e14e10f0304bc20f4b47d06b3ba58fcd3c950fecd1a",
    "137dd410334797b62d82ed35073d1131e2f10a02ce51c269e1248e423c299956b2c53ad26a6c5ddcb1d7cd",
    "c999265bb1954775fbc72cd8cf322a47091169f3fff19ff6aca15a5894fe68d2fa20c1f55000000000f010",
    "4a51e02094a880a3c134b5ff00",
);

#[cfg(feature = "mux")]
fn decode_test_hex_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2));
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for index in (0..hex.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&hex[index..index + 2], 16).unwrap());
    }
    bytes
}

pub fn temp_output_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mp4forge-{prefix}-{}-{unique}", std::process::id()))
}

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[cfg(feature = "decrypt")]
pub struct RetainedDecryptFileFixture {
    pub encrypted_path: PathBuf,
    pub decrypted_path: PathBuf,
    pub keys: Vec<DecryptionKey>,
}

#[cfg(feature = "decrypt")]
pub struct RetainedFragmentedDecryptFixture {
    pub fragments_info_path: PathBuf,
    pub encrypted_segment_path: PathBuf,
    pub clear_segment_path: PathBuf,
    pub keys: Vec<DecryptionKey>,
}

#[cfg(feature = "decrypt")]
const COMMON_ENCRYPTION_VIDEO_KID: [u8; 16] = [
    0xeb, 0x67, 0x6a, 0xbb, 0xcb, 0x34, 0x5e, 0x96, 0xbb, 0xcf, 0x61, 0x66, 0x30, 0xf1, 0xa3, 0xda,
];

#[cfg(feature = "decrypt")]
const COMMON_ENCRYPTION_VIDEO_KEY: [u8; 16] = [
    0x10, 0x0b, 0x6c, 0x20, 0x94, 0x0f, 0x77, 0x9a, 0x45, 0x89, 0x15, 0x2b, 0x57, 0xd2, 0xda, 0xcb,
];

#[cfg(feature = "decrypt")]
const COMMON_ENCRYPTION_AUDIO_KID: [u8; 16] = [
    0x63, 0xcb, 0x5f, 0x71, 0x84, 0xdd, 0x4b, 0x68, 0x9a, 0x5c, 0x5f, 0xf1, 0x1e, 0xe6, 0xa3, 0x28,
];

#[cfg(feature = "decrypt")]
const COMMON_ENCRYPTION_AUDIO_KEY: [u8; 16] = [
    0x3b, 0xda, 0x33, 0x29, 0x15, 0x8a, 0x47, 0x89, 0x88, 0x08, 0x16, 0xa7, 0x0e, 0x7e, 0x43, 0x6d,
];

#[cfg(feature = "decrypt")]
fn retained_decrypt_file_fixture(
    encrypted_name: &str,
    decrypted_name: &str,
    keys: Vec<DecryptionKey>,
) -> RetainedDecryptFileFixture {
    RetainedDecryptFileFixture {
        encrypted_path: fixture_path(encrypted_name),
        decrypted_path: fixture_path(decrypted_name),
        keys,
    }
}

#[cfg(feature = "decrypt")]
fn retained_fragmented_decrypt_fixture(
    fragments_info_name: &str,
    encrypted_segment_name: &str,
    clear_segment_name: &str,
    keys: Vec<DecryptionKey>,
) -> RetainedFragmentedDecryptFixture {
    RetainedFragmentedDecryptFixture {
        fragments_info_path: fixture_path(fragments_info_name),
        encrypted_segment_path: fixture_path(encrypted_segment_name),
        clear_segment_path: fixture_path(clear_segment_name),
        keys,
    }
}

#[cfg(feature = "decrypt")]
pub fn oma_dcf_ctr_fixture() -> RetainedDecryptFileFixture {
    retained_decrypt_file_fixture(
        "oma_dcf_ctr_encrypted.mp4",
        "oma_dcf_ctr_decrypted.mp4",
        vec![DecryptionKey::track(1, [0x11; 16])],
    )
}

#[cfg(feature = "decrypt")]
pub fn oma_dcf_cbc_fixture() -> RetainedDecryptFileFixture {
    retained_decrypt_file_fixture(
        "oma_dcf_cbc_encrypted.mp4",
        "oma_dcf_cbc_decrypted.mp4",
        vec![DecryptionKey::track(1, [0x11; 16])],
    )
}

#[cfg(feature = "decrypt")]
pub fn oma_dcf_ctr_grpi_fixture() -> RetainedDecryptFileFixture {
    retained_decrypt_file_fixture(
        "oma_dcf_ctr_grpi_encrypted.odf",
        "oma_dcf_ctr_grpi_decrypted.odf",
        vec![DecryptionKey::track(1, [0x33; 16])],
    )
}

#[cfg(feature = "decrypt")]
pub fn oma_dcf_cbc_grpi_fixture() -> RetainedDecryptFileFixture {
    retained_decrypt_file_fixture(
        "oma_dcf_cbc_grpi_encrypted.odf",
        "oma_dcf_cbc_grpi_decrypted.odf",
        vec![DecryptionKey::track(1, [0x33; 16])],
    )
}

#[cfg(feature = "decrypt")]
pub fn isma_iaec_fixture() -> RetainedDecryptFileFixture {
    retained_decrypt_file_fixture(
        "isma_iaec_encrypted.mp4",
        "isma_iaec_decrypted.mp4",
        vec![DecryptionKey::track(1, [0x44; 16])],
    )
}

#[cfg(feature = "decrypt")]
pub fn common_encryption_single_key_fixture_keys() -> Vec<DecryptionKey> {
    vec![DecryptionKey::kid(
        COMMON_ENCRYPTION_VIDEO_KID,
        COMMON_ENCRYPTION_VIDEO_KEY,
    )]
}

#[cfg(feature = "decrypt")]
pub fn common_encryption_multi_key_fixture_keys() -> Vec<DecryptionKey> {
    vec![
        DecryptionKey::kid(COMMON_ENCRYPTION_VIDEO_KID, COMMON_ENCRYPTION_VIDEO_KEY),
        DecryptionKey::kid(COMMON_ENCRYPTION_AUDIO_KID, COMMON_ENCRYPTION_AUDIO_KEY),
    ]
}

#[cfg(feature = "decrypt")]
pub fn common_encryption_multi_track_fixture() -> RetainedDecryptFileFixture {
    retained_decrypt_file_fixture(
        "cenc-multi-track/encrypted.mp4",
        "cenc-multi-track/expected-decrypted.mp4",
        common_encryption_multi_key_fixture_keys(),
    )
}

#[cfg(feature = "decrypt")]
pub fn common_encryption_fragment_fixture(
    directory: &str,
    track: &str,
) -> RetainedFragmentedDecryptFixture {
    let keys = match directory {
        value if value.ends_with("-single") => common_encryption_single_key_fixture_keys(),
        value if value.ends_with("-multi") => common_encryption_multi_key_fixture_keys(),
        _ => panic!("unsupported Common Encryption fixture directory: {directory}"),
    };

    RetainedFragmentedDecryptFixture {
        fragments_info_path: fixture_path(directory).join(format!("{track}_init.mp4")),
        encrypted_segment_path: fixture_path(directory).join(format!("{track}_1.m4s")),
        clear_segment_path: fixture_path(directory).join(format!("{track}_1.clear.m4s")),
        keys,
    }
}

#[cfg(feature = "decrypt")]
pub fn piff_ctr_fixture() -> RetainedDecryptFileFixture {
    retained_decrypt_file_fixture(
        "piff_ctr_encrypted.mp4",
        "piff_ctr_decrypted.mp4",
        common_encryption_single_key_fixture_keys(),
    )
}

#[cfg(feature = "decrypt")]
pub fn piff_cbc_fixture() -> RetainedDecryptFileFixture {
    retained_decrypt_file_fixture(
        "piff_cbc_encrypted.mp4",
        "piff_cbc_decrypted.mp4",
        common_encryption_single_key_fixture_keys(),
    )
}

#[cfg(feature = "decrypt")]
pub fn piff_ctr_segment_fixture() -> RetainedFragmentedDecryptFixture {
    retained_fragmented_decrypt_fixture(
        "piff_ctr_init.mp4",
        "piff_ctr_media_encrypted.m4s",
        "piff_ctr_media_decrypted.m4s",
        common_encryption_single_key_fixture_keys(),
    )
}

#[cfg(feature = "decrypt")]
pub fn piff_cbc_segment_fixture() -> RetainedFragmentedDecryptFixture {
    retained_fragmented_decrypt_fixture(
        "piff_cbc_init.mp4",
        "piff_cbc_media_encrypted.m4s",
        "piff_cbc_media_decrypted.m4s",
        common_encryption_single_key_fixture_keys(),
    )
}

#[cfg(feature = "decrypt")]
pub fn marlin_ipmp_acbc_encrypted_fixture_path() -> PathBuf {
    fixture_path("marlin_ipmp_acbc_encrypted.mp4")
}

#[cfg(feature = "decrypt")]
pub fn marlin_ipmp_acbc_decrypted_fixture_path() -> PathBuf {
    fixture_path("marlin_ipmp_acbc_decrypted.mp4")
}

#[cfg(feature = "decrypt")]
pub fn marlin_ipmp_acbc_fixture_keys() -> Vec<DecryptionKey> {
    vec![
        DecryptionKey::track(
            1,
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ],
        ),
        DecryptionKey::track(
            2,
            [
                0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87, 0x98, 0xa9, 0xba, 0xbc, 0xbd, 0xdc,
                0xed, 0xfe,
            ],
        ),
    ]
}

#[cfg(feature = "decrypt")]
pub fn marlin_ipmp_acbc_fixture() -> RetainedDecryptFileFixture {
    retained_decrypt_file_fixture(
        "marlin_ipmp_acbc_encrypted.mp4",
        "marlin_ipmp_acbc_decrypted.mp4",
        marlin_ipmp_acbc_fixture_keys(),
    )
}

#[cfg(feature = "decrypt")]
pub fn marlin_ipmp_acgk_encrypted_fixture_path() -> PathBuf {
    fixture_path("marlin_ipmp_acgk_encrypted.mp4")
}

#[cfg(feature = "decrypt")]
pub fn marlin_ipmp_acgk_decrypted_fixture_path() -> PathBuf {
    fixture_path("marlin_ipmp_acgk_decrypted.mp4")
}

#[cfg(feature = "decrypt")]
pub fn marlin_ipmp_acgk_fixture_keys() -> Vec<DecryptionKey> {
    vec![DecryptionKey::track(
        0,
        [
            0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22,
            0x11, 0x00,
        ],
    )]
}

#[cfg(feature = "decrypt")]
pub fn marlin_ipmp_acgk_fixture() -> RetainedDecryptFileFixture {
    retained_decrypt_file_fixture(
        "marlin_ipmp_acgk_encrypted.mp4",
        "marlin_ipmp_acgk_decrypted.mp4",
        marlin_ipmp_acgk_fixture_keys(),
    )
}

#[cfg(feature = "decrypt")]
pub struct ProtectedMovieTopologyFixture {
    pub encrypted: Vec<u8>,
    pub decrypted: Vec<u8>,
    pub keys: Vec<DecryptionKey>,
}

#[cfg(feature = "decrypt")]
struct SampleEntryMovieTrackSpec {
    track_id: u32,
    width: u16,
    height: u16,
    sample_entry: Vec<u8>,
    samples: Vec<Vec<u8>>,
    chunk_sample_counts: Vec<u32>,
}

#[cfg(feature = "decrypt")]
#[derive(Clone)]
enum RetainedTrackChunkOffsetState {
    Stco {
        info: BoxInfo,
        box_value: Stco,
    },
    Co64 {
        info: BoxInfo,
        box_value: mp4forge::boxes::iso14496_12::Co64,
    },
}

#[cfg(feature = "decrypt")]
#[derive(Clone)]
struct RetainedMarlinTrackLayout {
    track_id: u32,
    trak_info: BoxInfo,
    mdia_info: BoxInfo,
    minf_info: BoxInfo,
    stbl_info: BoxInfo,
    stsd_info: BoxInfo,
    stsc_info: BoxInfo,
    stsc: Stsc,
    stsz_info: BoxInfo,
    stsz: Stsz,
    chunk_offsets: RetainedTrackChunkOffsetState,
}

#[cfg(feature = "decrypt")]
struct GeneratedProtectedMovieTrackLayout {
    track_id: u32,
    trak_info: BoxInfo,
    mdia_info: BoxInfo,
    minf_info: BoxInfo,
    stbl_info: BoxInfo,
    stsd_info: BoxInfo,
    stsc_info: BoxInfo,
    stsz_info: BoxInfo,
    stsz: Stsz,
    chunk_offsets: RetainedTrackChunkOffsetState,
}

#[cfg(feature = "decrypt")]
pub fn build_marlin_ipmp_acbc_broader_movie_fixture() -> ProtectedMovieTopologyFixture {
    build_broader_marlin_movie_fixture(&marlin_ipmp_acbc_fixture())
}

#[cfg(feature = "decrypt")]
pub fn build_marlin_ipmp_acgk_broader_movie_fixture() -> ProtectedMovieTopologyFixture {
    build_broader_marlin_movie_fixture(&marlin_ipmp_acgk_fixture())
}

#[cfg(feature = "decrypt")]
pub fn build_marlin_ipmp_acbc_sample_description_index_movie_fixture()
-> ProtectedMovieTopologyFixture {
    build_sample_description_index_marlin_movie_fixture(&marlin_ipmp_acbc_fixture())
}

#[cfg(feature = "decrypt")]
pub fn build_marlin_ipmp_acgk_sample_description_index_movie_fixture()
-> ProtectedMovieTopologyFixture {
    build_sample_description_index_marlin_movie_fixture(&marlin_ipmp_acgk_fixture())
}

#[cfg(feature = "decrypt")]
fn build_broader_marlin_movie_fixture(
    retained: &RetainedDecryptFileFixture,
) -> ProtectedMovieTopologyFixture {
    let trailing_free = encode_raw_box(fourcc("free"), &[0x4d, 0x34, 0x34, 0x34]);
    let encrypted = fs::read(&retained.encrypted_path).unwrap();
    let decrypted = fs::read(&retained.decrypted_path).unwrap();

    ProtectedMovieTopologyFixture {
        encrypted: broaden_retained_marlin_movie_bytes(&encrypted, &trailing_free),
        decrypted: insert_root_box_before_single_mdat_and_shift_offsets(&decrypted, &trailing_free),
        keys: retained.keys.clone(),
    }
}

#[cfg(feature = "decrypt")]
fn build_sample_description_index_marlin_movie_fixture(
    retained: &RetainedDecryptFileFixture,
) -> ProtectedMovieTopologyFixture {
    let broader = build_broader_marlin_movie_fixture(retained);
    ProtectedMovieTopologyFixture {
        encrypted: patch_marlin_od_track_sample_description_index(&broader.encrypted),
        decrypted: broader.decrypted,
        keys: broader.keys,
    }
}

#[cfg(feature = "decrypt")]
fn broaden_retained_marlin_movie_bytes(input: &[u8], trailing_root_box: &[u8]) -> Vec<u8> {
    let root_boxes = read_root_box_infos(input);
    let moov_info = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == fourcc("moov"))
        .unwrap();
    let mdat_info = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == fourcc("mdat"))
        .unwrap();

    let iods = extract_single_as_from_bytes::<Iods>(
        input,
        None,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    let od_track_id = iods
        .initial_object_descriptor()
        .unwrap()
        .sub_descriptors
        .iter()
        .find_map(|descriptor| descriptor.es_id_inc_descriptor())
        .unwrap()
        .track_id;

    let trak_infos =
        extract_infos_from_bytes(input, None, BoxPath::from([fourcc("moov"), fourcc("trak")]));
    let track_layouts = trak_infos
        .into_iter()
        .map(|trak_info| analyze_retained_marlin_track_layout(input, trak_info))
        .collect::<Vec<_>>();
    let od_track = track_layouts
        .iter()
        .find(|layout| layout.track_id == od_track_id)
        .cloned()
        .unwrap();

    let original_sample_size = if od_track.stsz.sample_size == 0 {
        u32::try_from(od_track.stsz.entry_size[0]).unwrap()
    } else {
        od_track.stsz.sample_size
    };
    let original_offset = retained_track_chunk_offsets(&od_track.chunk_offsets)[0];
    let extra_sample = read_sample_bytes(input, original_offset, original_sample_size).to_vec();
    let appended_sample_offset = mdat_info.offset() + mdat_info.size();

    let placeholder_od_track = rebuild_retained_marlin_track(
        input,
        &od_track,
        patch_retained_track_stsz(&od_track.stsz, u64::try_from(extra_sample.len()).unwrap()),
        patch_retained_track_chunk_offsets(
            &od_track.chunk_offsets,
            0,
            Some(appended_sample_offset),
        ),
        None,
        None,
    );
    let placeholder_moov = rebuild_container_box_with_replacements(
        input,
        moov_info,
        &Moov,
        &BTreeMap::from([(od_track.trak_info.offset(), placeholder_od_track)]),
    );
    let moov_shift = u64::try_from(placeholder_moov.len()).unwrap() - moov_info.size();

    let mut moov_replacements = BTreeMap::new();
    for track in &track_layouts {
        let extra_offset =
            (track.track_id == od_track_id).then_some(appended_sample_offset + moov_shift);
        let stsz = if track.track_id == od_track_id {
            patch_retained_track_stsz(&track.stsz, u64::try_from(extra_sample.len()).unwrap())
        } else {
            track.stsz.clone()
        };
        let rebuilt_trak = rebuild_retained_marlin_track(
            input,
            track,
            stsz,
            patch_retained_track_chunk_offsets(&track.chunk_offsets, moov_shift, extra_offset),
            None,
            None,
        );
        moov_replacements.insert(track.trak_info.offset(), rebuilt_trak);
    }
    let rebuilt_moov =
        rebuild_container_box_with_replacements(input, moov_info, &Moov, &moov_replacements);

    let mdat_payload = slice_box_bytes(input, mdat_info)
        [usize::try_from(mdat_info.header_size()).unwrap()..]
        .iter()
        .copied()
        .chain(extra_sample)
        .collect::<Vec<_>>();
    let rebuilt_mdat = encode_raw_box(fourcc("mdat"), &mdat_payload);

    let mut output = Vec::new();
    for root_info in root_boxes {
        if root_info.offset() == moov_info.offset() {
            output.extend_from_slice(&rebuilt_moov);
        } else if root_info.offset() == mdat_info.offset() {
            output.extend_from_slice(&rebuilt_mdat);
        } else {
            output.extend_from_slice(slice_box_bytes(input, root_info));
        }
    }
    output.extend_from_slice(trailing_root_box);
    output
}

#[cfg(feature = "decrypt")]
fn patch_marlin_od_track_sample_description_index(input: &[u8]) -> Vec<u8> {
    let root_boxes = read_root_box_infos(input);
    let moov_info = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == fourcc("moov"))
        .unwrap();

    let iods = extract_single_as_from_bytes::<Iods>(
        input,
        None,
        BoxPath::from([fourcc("moov"), fourcc("iods")]),
    );
    let od_track_id = iods
        .initial_object_descriptor()
        .unwrap()
        .sub_descriptors
        .iter()
        .find_map(|descriptor| descriptor.es_id_inc_descriptor())
        .unwrap()
        .track_id;

    let trak_infos =
        extract_infos_from_bytes(input, None, BoxPath::from([fourcc("moov"), fourcc("trak")]));
    let track_layouts = trak_infos
        .into_iter()
        .map(|trak_info| analyze_retained_marlin_track_layout(input, trak_info))
        .collect::<Vec<_>>();

    let placeholder_replacements = track_layouts
        .iter()
        .map(|track| {
            let (stsd_replacement, stsc_replacement) = if track.track_id == od_track_id {
                (
                    Some(duplicate_retained_marlin_od_track_sample_entry(
                        input, track,
                    )),
                    Some(patch_retained_track_stsc_sample_description_index(
                        &track.stsc,
                        2,
                    )),
                )
            } else {
                (None, None)
            };
            (
                track.trak_info.offset(),
                rebuild_retained_marlin_track(
                    input,
                    track,
                    track.stsz.clone(),
                    patch_retained_track_chunk_offsets(&track.chunk_offsets, 0, None),
                    stsd_replacement,
                    stsc_replacement,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let placeholder_moov =
        rebuild_container_box_with_replacements(input, moov_info, &Moov, &placeholder_replacements);
    let moov_shift = u64::try_from(placeholder_moov.len()).unwrap() - moov_info.size();

    let moov_replacements = track_layouts
        .iter()
        .map(|track| {
            let (stsd_replacement, stsc_replacement) = if track.track_id == od_track_id {
                (
                    Some(duplicate_retained_marlin_od_track_sample_entry(
                        input, track,
                    )),
                    Some(patch_retained_track_stsc_sample_description_index(
                        &track.stsc,
                        2,
                    )),
                )
            } else {
                (None, None)
            };
            (
                track.trak_info.offset(),
                rebuild_retained_marlin_track(
                    input,
                    track,
                    track.stsz.clone(),
                    patch_retained_track_chunk_offsets(&track.chunk_offsets, moov_shift, None),
                    stsd_replacement,
                    stsc_replacement,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let rebuilt_moov =
        rebuild_container_box_with_replacements(input, moov_info, &Moov, &moov_replacements);

    let mut output = Vec::new();
    for root_info in root_boxes {
        if root_info.offset() == moov_info.offset() {
            output.extend_from_slice(&rebuilt_moov);
        } else {
            output.extend_from_slice(slice_box_bytes(input, root_info));
        }
    }
    output
}

#[cfg(feature = "decrypt")]
fn read_root_box_infos(input: &[u8]) -> Vec<BoxInfo> {
    let mut reader = Cursor::new(input);
    let mut boxes = Vec::new();
    while usize::try_from(reader.stream_position().unwrap())
        .ok()
        .is_some_and(|offset| offset < input.len())
    {
        let info = BoxInfo::read(&mut reader).unwrap();
        info.seek_to_end(&mut reader).unwrap();
        boxes.push(info);
    }
    boxes
}

#[cfg(feature = "decrypt")]
fn slice_box_bytes(input: &[u8], info: BoxInfo) -> &[u8] {
    let start = usize::try_from(info.offset()).unwrap();
    let end = usize::try_from(info.offset() + info.size()).unwrap();
    &input[start..end]
}

#[cfg(feature = "decrypt")]
fn extract_infos_from_bytes(input: &[u8], parent: Option<&BoxInfo>, path: BoxPath) -> Vec<BoxInfo> {
    let mut reader = Cursor::new(input);
    extract_box(&mut reader, parent, path).unwrap()
}

#[cfg(feature = "decrypt")]
fn extract_single_info_from_bytes(
    input: &[u8],
    parent: Option<&BoxInfo>,
    path: BoxPath,
) -> BoxInfo {
    let infos = extract_infos_from_bytes(input, parent, path);
    assert_eq!(infos.len(), 1);
    infos[0]
}

#[cfg(feature = "decrypt")]
fn extract_single_as_from_bytes<T>(input: &[u8], parent: Option<&BoxInfo>, path: BoxPath) -> T
where
    T: CodecBox + Clone + 'static,
{
    let mut reader = Cursor::new(input);
    let mut values = extract_box_as::<_, T>(&mut reader, parent, path).unwrap();
    assert_eq!(values.len(), 1);
    values.remove(0)
}

#[cfg(feature = "decrypt")]
fn analyze_retained_marlin_track_layout(
    input: &[u8],
    trak_info: BoxInfo,
) -> RetainedMarlinTrackLayout {
    let tkhd = extract_single_as_from_bytes::<mp4forge::boxes::iso14496_12::Tkhd>(
        input,
        Some(&trak_info),
        BoxPath::from([fourcc("tkhd")]),
    );
    let mdia_info =
        extract_single_info_from_bytes(input, Some(&trak_info), BoxPath::from([fourcc("mdia")]));
    let minf_info = extract_single_info_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([fourcc("mdia"), fourcc("minf")]),
    );
    let stbl_info = extract_single_info_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([fourcc("mdia"), fourcc("minf"), fourcc("stbl")]),
    );
    let stsd_info = extract_single_info_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
        ]),
    );
    let stsc_info = extract_single_info_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsc"),
        ]),
    );
    let stsc = extract_single_as_from_bytes::<Stsc>(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsc"),
        ]),
    );
    let stsz_info = extract_single_info_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsz"),
        ]),
    );
    let stsz = extract_single_as_from_bytes::<Stsz>(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsz"),
        ]),
    );

    let stco_infos = extract_infos_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stco"),
        ]),
    );
    let co64_infos = extract_infos_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("co64"),
        ]),
    );
    let chunk_offsets = if !stco_infos.is_empty() {
        let stco = extract_single_as_from_bytes::<Stco>(
            input,
            Some(&trak_info),
            BoxPath::from([
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stco"),
            ]),
        );
        RetainedTrackChunkOffsetState::Stco {
            info: stco_infos[0],
            box_value: stco,
        }
    } else {
        let co64 = extract_single_as_from_bytes::<mp4forge::boxes::iso14496_12::Co64>(
            input,
            Some(&trak_info),
            BoxPath::from([
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("co64"),
            ]),
        );
        RetainedTrackChunkOffsetState::Co64 {
            info: co64_infos[0],
            box_value: co64,
        }
    };

    RetainedMarlinTrackLayout {
        track_id: tkhd.track_id,
        trak_info,
        mdia_info,
        minf_info,
        stbl_info,
        stsd_info,
        stsc_info,
        stsc,
        stsz_info,
        stsz,
        chunk_offsets,
    }
}

#[cfg(feature = "decrypt")]
fn retained_track_chunk_offsets(chunk_offsets: &RetainedTrackChunkOffsetState) -> Vec<u64> {
    match chunk_offsets {
        RetainedTrackChunkOffsetState::Stco { box_value, .. } => box_value.chunk_offset.to_vec(),
        RetainedTrackChunkOffsetState::Co64 { box_value, .. } => box_value.chunk_offset.clone(),
    }
}

#[cfg(feature = "decrypt")]
fn patch_retained_track_stsz(stsz: &Stsz, extra_sample_size: u64) -> Stsz {
    let mut patched = stsz.clone();
    patched.sample_count += 1;
    if patched.sample_size == 0 {
        patched.entry_size.push(extra_sample_size);
    } else if u64::from(patched.sample_size) != extra_sample_size {
        patched.entry_size = vec![u64::from(stsz.sample_size), extra_sample_size];
        patched.sample_size = 0;
    }
    patched
}

#[cfg(feature = "decrypt")]
fn patch_retained_track_chunk_offsets(
    chunk_offsets: &RetainedTrackChunkOffsetState,
    shift: u64,
    extra_offset: Option<u64>,
) -> Vec<u8> {
    match chunk_offsets {
        RetainedTrackChunkOffsetState::Stco { box_value, .. } => {
            let mut patched = box_value.clone();
            patched.chunk_offset = patched
                .chunk_offset
                .iter()
                .map(|offset| offset + shift)
                .collect();
            if let Some(extra_offset) = extra_offset {
                patched.chunk_offset.push(extra_offset);
                patched.entry_count += 1;
            }
            encode_supported_box(&patched, &[])
        }
        RetainedTrackChunkOffsetState::Co64 { box_value, .. } => {
            let mut patched = box_value.clone();
            patched.chunk_offset = patched
                .chunk_offset
                .iter()
                .map(|offset| offset + shift)
                .collect();
            if let Some(extra_offset) = extra_offset {
                patched.chunk_offset.push(extra_offset);
                patched.entry_count += 1;
            }
            encode_supported_box(&patched, &[])
        }
    }
}

#[cfg(feature = "decrypt")]
fn rebuild_retained_marlin_track(
    input: &[u8],
    track: &RetainedMarlinTrackLayout,
    stsz: Stsz,
    chunk_offset_box: Vec<u8>,
    stsd_box: Option<Vec<u8>>,
    stsc_box: Option<Vec<u8>>,
) -> Vec<u8> {
    let chunk_offset_info = match track.chunk_offsets {
        RetainedTrackChunkOffsetState::Stco { info, .. }
        | RetainedTrackChunkOffsetState::Co64 { info, .. } => info,
    };
    let mut stbl_replacements = BTreeMap::from([
        (track.stsz_info.offset(), encode_supported_box(&stsz, &[])),
        (chunk_offset_info.offset(), chunk_offset_box),
    ]);
    if let Some(stsd_box) = stsd_box {
        stbl_replacements.insert(track.stsd_info.offset(), stsd_box);
    }
    if let Some(stsc_box) = stsc_box {
        stbl_replacements.insert(track.stsc_info.offset(), stsc_box);
    }
    let stbl =
        rebuild_container_box_with_replacements(input, track.stbl_info, &Stbl, &stbl_replacements);
    let minf = rebuild_container_box_with_replacements(
        input,
        track.minf_info,
        &Minf,
        &BTreeMap::from([(track.stbl_info.offset(), stbl)]),
    );
    let mdia = rebuild_container_box_with_replacements(
        input,
        track.mdia_info,
        &Mdia,
        &BTreeMap::from([(track.minf_info.offset(), minf)]),
    );
    rebuild_container_box_with_replacements(
        input,
        track.trak_info,
        &Trak,
        &BTreeMap::from([(track.mdia_info.offset(), mdia)]),
    )
}

#[cfg(feature = "decrypt")]
fn analyze_generated_protected_movie_track_layout(
    input: &[u8],
    trak_info: BoxInfo,
) -> GeneratedProtectedMovieTrackLayout {
    let tkhd = extract_single_as_from_bytes::<mp4forge::boxes::iso14496_12::Tkhd>(
        input,
        Some(&trak_info),
        BoxPath::from([fourcc("tkhd")]),
    );
    let mdia_info =
        extract_single_info_from_bytes(input, Some(&trak_info), BoxPath::from([fourcc("mdia")]));
    let minf_info = extract_single_info_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([fourcc("mdia"), fourcc("minf")]),
    );
    let stbl_info = extract_single_info_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([fourcc("mdia"), fourcc("minf"), fourcc("stbl")]),
    );
    let stsd_info = extract_single_info_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
        ]),
    );
    let stsc_info = extract_single_info_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsc"),
        ]),
    );
    let stsz_info = extract_single_info_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsz"),
        ]),
    );
    let stsz = extract_single_as_from_bytes::<Stsz>(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsz"),
        ]),
    );
    let stco_infos = extract_infos_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stco"),
        ]),
    );
    let co64_infos = extract_infos_from_bytes(
        input,
        Some(&trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("co64"),
        ]),
    );
    let chunk_offsets = if !stco_infos.is_empty() {
        let stco = extract_single_as_from_bytes::<Stco>(
            input,
            Some(&trak_info),
            BoxPath::from([
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stco"),
            ]),
        );
        RetainedTrackChunkOffsetState::Stco {
            info: stco_infos[0],
            box_value: stco,
        }
    } else {
        let co64 = extract_single_as_from_bytes::<mp4forge::boxes::iso14496_12::Co64>(
            input,
            Some(&trak_info),
            BoxPath::from([
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("co64"),
            ]),
        );
        RetainedTrackChunkOffsetState::Co64 {
            info: co64_infos[0],
            box_value: co64,
        }
    };

    GeneratedProtectedMovieTrackLayout {
        track_id: tkhd.track_id,
        trak_info,
        mdia_info,
        minf_info,
        stbl_info,
        stsd_info,
        stsc_info,
        stsz_info,
        stsz,
        chunk_offsets,
    }
}

#[cfg(feature = "decrypt")]
fn rebuild_generated_protected_movie_track(
    input: &[u8],
    track: &GeneratedProtectedMovieTrackLayout,
    chunk_offset_box: Vec<u8>,
    stsd_box: Option<Vec<u8>>,
    stsc_box: Option<Vec<u8>>,
) -> Vec<u8> {
    let chunk_offset_info = match track.chunk_offsets {
        RetainedTrackChunkOffsetState::Stco { info, .. }
        | RetainedTrackChunkOffsetState::Co64 { info, .. } => info,
    };
    let mut stbl_replacements = BTreeMap::from([
        (
            track.stsz_info.offset(),
            encode_supported_box(&track.stsz, &[]),
        ),
        (chunk_offset_info.offset(), chunk_offset_box),
    ]);
    if let Some(stsd_box) = stsd_box {
        stbl_replacements.insert(track.stsd_info.offset(), stsd_box);
    }
    if let Some(stsc_box) = stsc_box {
        stbl_replacements.insert(track.stsc_info.offset(), stsc_box);
    }
    let stbl =
        rebuild_container_box_with_replacements(input, track.stbl_info, &Stbl, &stbl_replacements);
    let minf = rebuild_container_box_with_replacements(
        input,
        track.minf_info,
        &Minf,
        &BTreeMap::from([(track.stbl_info.offset(), stbl)]),
    );
    let mdia = rebuild_container_box_with_replacements(
        input,
        track.mdia_info,
        &Mdia,
        &BTreeMap::from([(track.minf_info.offset(), minf)]),
    );
    rebuild_container_box_with_replacements(
        input,
        track.trak_info,
        &Trak,
        &BTreeMap::from([(track.mdia_info.offset(), mdia)]),
    )
}

#[cfg(feature = "decrypt")]
fn duplicate_retained_marlin_od_track_sample_entry(
    input: &[u8],
    track: &RetainedMarlinTrackLayout,
) -> Vec<u8> {
    let mut stsd = extract_single_as_from_bytes::<Stsd>(
        input,
        Some(&track.trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
        ]),
    );
    let sample_entry_infos =
        extract_infos_from_bytes(input, Some(&track.stsd_info), BoxPath::from([FourCc::ANY]));
    assert_eq!(sample_entry_infos.len(), 1);
    let sample_entry = slice_box_bytes(input, sample_entry_infos[0]).to_vec();
    stsd.entry_count = 2;
    encode_supported_box(&stsd, &[sample_entry.clone(), sample_entry].concat())
}

#[cfg(feature = "decrypt")]
fn append_generated_protected_movie_second_sample_entry(
    input: &[u8],
    track: &GeneratedProtectedMovieTrackLayout,
) -> Vec<u8> {
    let mut stsd = extract_single_as_from_bytes::<Stsd>(
        input,
        Some(&track.trak_info),
        BoxPath::from([
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
        ]),
    );
    let sample_entry_infos =
        extract_infos_from_bytes(input, Some(&track.stsd_info), BoxPath::from([FourCc::ANY]));
    assert_eq!(sample_entry_infos.len(), 1);
    stsd.entry_count = 2;
    encode_supported_box(
        &stsd,
        &[
            slice_box_bytes(input, sample_entry_infos[0]).to_vec(),
            build_clear_avc1_sample_entry(320, 180),
        ]
        .concat(),
    )
}

#[cfg(feature = "decrypt")]
fn patch_retained_track_stsc_sample_description_index(
    stsc: &Stsc,
    sample_description_index: u32,
) -> Vec<u8> {
    let mut patched = stsc.clone();
    for entry in &mut patched.entries {
        entry.sample_description_index = sample_description_index;
    }
    encode_supported_box(&patched, &[])
}

#[cfg(feature = "decrypt")]
fn patch_standard_protected_movie_track_sample_description_index(
    input: &[u8],
    protected_track_id: u32,
) -> Vec<u8> {
    let root_boxes = read_root_box_infos(input);
    let moov_info = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == fourcc("moov"))
        .unwrap();
    let trak_infos =
        extract_infos_from_bytes(input, None, BoxPath::from([fourcc("moov"), fourcc("trak")]));
    let track_layouts = trak_infos
        .into_iter()
        .map(|trak_info| analyze_generated_protected_movie_track_layout(input, trak_info))
        .collect::<Vec<_>>();

    let build_replacement = |track: &GeneratedProtectedMovieTrackLayout, shift: u64| {
        let (stsd_replacement, stsc_replacement) = if track.track_id == protected_track_id {
            let stsc = extract_single_as_from_bytes::<Stsc>(
                input,
                Some(&track.trak_info),
                BoxPath::from([
                    fourcc("mdia"),
                    fourcc("minf"),
                    fourcc("stbl"),
                    fourcc("stsc"),
                ]),
            );
            (
                Some(append_generated_protected_movie_second_sample_entry(
                    input, track,
                )),
                Some(patch_retained_track_stsc_sample_description_index(&stsc, 2)),
            )
        } else {
            (None, None)
        };
        rebuild_generated_protected_movie_track(
            input,
            track,
            patch_retained_track_chunk_offsets(&track.chunk_offsets, shift, None),
            stsd_replacement,
            stsc_replacement,
        )
    };

    let placeholder_replacements = track_layouts
        .iter()
        .map(|track| (track.trak_info.offset(), build_replacement(track, 0)))
        .collect::<BTreeMap<_, _>>();
    let placeholder_moov =
        rebuild_container_box_with_replacements(input, moov_info, &Moov, &placeholder_replacements);
    let moov_shift =
        i64::try_from(placeholder_moov.len()).unwrap() - i64::try_from(moov_info.size()).unwrap();
    let shift = u64::try_from(moov_shift).unwrap();

    let moov_replacements = track_layouts
        .iter()
        .map(|track| (track.trak_info.offset(), build_replacement(track, shift)))
        .collect::<BTreeMap<_, _>>();
    let moov = rebuild_container_box_with_replacements(input, moov_info, &Moov, &moov_replacements);

    let mut output = Vec::new();
    for root_info in root_boxes {
        if root_info.offset() == moov_info.offset() {
            output.extend_from_slice(&moov);
        } else {
            output.extend_from_slice(slice_box_bytes(input, root_info));
        }
    }

    output
}

#[cfg(feature = "decrypt")]
fn insert_root_box_before_single_mdat_and_shift_offsets(
    input: &[u8],
    extra_root_box: &[u8],
) -> Vec<u8> {
    let root_boxes = read_root_box_infos(input);
    let moov_info = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == fourcc("moov"))
        .unwrap();
    let mdat_info = root_boxes
        .iter()
        .copied()
        .find(|info| info.box_type() == fourcc("mdat"))
        .unwrap();
    let trak_infos =
        extract_infos_from_bytes(input, None, BoxPath::from([fourcc("moov"), fourcc("trak")]));
    let track_layouts = trak_infos
        .into_iter()
        .map(|trak_info| analyze_retained_marlin_track_layout(input, trak_info))
        .collect::<Vec<_>>();
    let shift = u64::try_from(extra_root_box.len()).unwrap();
    let moov_replacements = track_layouts
        .iter()
        .map(|track| {
            (
                track.trak_info.offset(),
                rebuild_retained_marlin_track(
                    input,
                    track,
                    track.stsz.clone(),
                    patch_retained_track_chunk_offsets(&track.chunk_offsets, shift, None),
                    None,
                    None,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let rebuilt_moov =
        rebuild_container_box_with_replacements(input, moov_info, &Moov, &moov_replacements);

    let mut output = Vec::new();
    for root_info in root_boxes {
        if root_info.offset() == moov_info.offset() {
            output.extend_from_slice(&rebuilt_moov);
        } else if root_info.offset() == mdat_info.offset() {
            continue;
        } else {
            output.extend_from_slice(slice_box_bytes(input, root_info));
        }
    }
    output.extend_from_slice(extra_root_box);
    output.extend_from_slice(slice_box_bytes(input, mdat_info));
    output
}

#[cfg(feature = "decrypt")]
fn rebuild_container_box_with_replacements<B>(
    input: &[u8],
    parent_info: BoxInfo,
    box_value: &B,
    replacements: &BTreeMap<u64, Vec<u8>>,
) -> Vec<u8>
where
    B: CodecBox,
{
    let child_infos =
        extract_infos_from_bytes(input, Some(&parent_info), BoxPath::from([FourCc::ANY]));
    let mut children = Vec::new();
    for child_info in child_infos {
        if let Some(replacement) = replacements.get(&child_info.offset()) {
            children.extend_from_slice(replacement);
        } else {
            children.extend_from_slice(slice_box_bytes(input, child_info));
        }
    }
    encode_supported_box(box_value, &children)
}

#[cfg(feature = "decrypt")]
fn read_sample_bytes(input: &[u8], absolute_offset: u64, sample_size: u32) -> &[u8] {
    let start = usize::try_from(absolute_offset).unwrap();
    let end = start + usize::try_from(sample_size).unwrap();
    &input[start..end]
}

#[cfg(feature = "decrypt")]
pub fn build_oma_dcf_broader_movie_fixture() -> ProtectedMovieTopologyFixture {
    let protected_track_id = 1;
    let clear_track_id = 2;
    let key = [0x55; 16];
    let protected_samples = vec![
        vec![0x11, 0x22, 0x33, 0x44, 0x55],
        vec![0x66, 0x77, 0x88, 0x99],
        vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
    ];
    let clear_track_samples = vec![vec![0x01, 0x03, 0x05], vec![0x07, 0x09, 0x0b, 0x0d]];
    let protected_chunk_sample_counts = [2_u32, 1];
    let clear_chunk_sample_counts = [1_u32, 1];
    let protected_ivs = [
        [
            0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66,
            0x77, 0x88,
        ],
        [
            0x20, 0x42, 0x64, 0x86, 0xa8, 0xca, 0xec, 0x0e, 0x21, 0x43, 0x65, 0x87, 0xa9, 0xcb,
            0xed, 0x0f,
        ],
        [
            0x30, 0x52, 0x74, 0x96, 0xb8, 0xda, 0xfc, 0x1e, 0x31, 0x53, 0x75, 0x97, 0xb9, 0xdb,
            0xfd, 0x1f,
        ],
    ];
    let encrypted_protected_samples = protected_samples
        .iter()
        .zip(protected_ivs)
        .map(|(sample, iv)| encrypt_oma_dcf_ctr_movie_sample(sample, key, iv))
        .collect::<Vec<_>>();

    let encrypted_ftyp = Ftyp {
        major_brand: fourcc("odcf"),
        minor_version: 1,
        compatible_brands: vec![fourcc("odcf"), fourcc("opf2"), fourcc("isom")],
    };
    let clear_ftyp = Ftyp {
        major_brand: fourcc("odcf"),
        minor_version: 1,
        compatible_brands: vec![fourcc("odcf"), fourcc("isom")],
    };
    let leading_empty_mdat = encode_raw_box(fourcc("mdat"), &[]);
    let trailing_free = encode_raw_box(fourcc("free"), &[0xfa, 0xce, 0xb0, 0x0c]);
    let encrypted_protected_track = SampleEntryMovieTrackSpec {
        track_id: protected_track_id,
        width: 320,
        height: 180,
        sample_entry: build_oma_dcf_protected_sample_entry(),
        samples: encrypted_protected_samples,
        chunk_sample_counts: protected_chunk_sample_counts.to_vec(),
    };
    let clear_protected_track = SampleEntryMovieTrackSpec {
        track_id: protected_track_id,
        width: 320,
        height: 180,
        sample_entry: build_clear_avc1_sample_entry(320, 180),
        samples: protected_samples,
        chunk_sample_counts: protected_chunk_sample_counts.to_vec(),
    };
    let clear_track = SampleEntryMovieTrackSpec {
        track_id: clear_track_id,
        width: 640,
        height: 360,
        sample_entry: build_clear_avc1_sample_entry(640, 360),
        samples: clear_track_samples,
        chunk_sample_counts: clear_chunk_sample_counts.to_vec(),
    };

    let encrypted = build_two_track_sample_entry_movie(
        &encrypted_ftyp,
        &encrypted_protected_track,
        &clear_track,
        &[leading_empty_mdat],
        std::slice::from_ref(&trailing_free),
    );
    let decrypted = build_two_track_sample_entry_movie(
        &clear_ftyp,
        &clear_protected_track,
        &clear_track,
        std::slice::from_ref(&trailing_free),
        &[],
    );

    ProtectedMovieTopologyFixture {
        encrypted,
        decrypted,
        keys: vec![DecryptionKey::track(protected_track_id, key)],
    }
}

#[cfg(feature = "decrypt")]
pub fn build_oma_dcf_sample_description_index_unsupported_movie_fixture()
-> ProtectedMovieTopologyFixture {
    build_sample_description_index_unsupported_protected_movie_fixture(
        build_oma_dcf_broader_movie_fixture(),
        1,
    )
}

#[cfg(feature = "decrypt")]
pub fn build_iaec_broader_movie_fixture() -> ProtectedMovieTopologyFixture {
    let protected_track_id = 1;
    let clear_track_id = 2;
    let key = [0x66; 16];
    let salt = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
    let protected_samples = vec![
        vec![0x90, 0x91, 0x92, 0x93, 0x94, 0x95],
        vec![0xa0, 0xa1, 0xa2],
        vec![0xb0, 0xb1, 0xb2, 0xb3, 0xb4],
    ];
    let clear_track_samples = vec![vec![0x31, 0x41, 0x59, 0x26], vec![0x53, 0x58, 0x97]];
    let protected_chunk_sample_counts = [2_u32, 1];
    let clear_chunk_sample_counts = [1_u32, 1];
    let protected_ivs = [[0_u8; 8], [0_u8; 8], [0_u8; 8]];
    let encrypted_protected_samples = protected_samples
        .iter()
        .zip(protected_ivs)
        .map(|(sample, iv)| encrypt_iaec_movie_sample(sample, key, salt, iv))
        .collect::<Vec<_>>();

    let ftyp = Ftyp {
        major_brand: fourcc("isom"),
        minor_version: 1,
        compatible_brands: vec![fourcc("isom"), fourcc("mp42")],
    };
    let leading_empty_mdat = encode_raw_box(fourcc("mdat"), &[]);
    let trailing_free = encode_raw_box(fourcc("free"), &[0x12, 0x34, 0x56, 0x78]);
    let encrypted_protected_track = SampleEntryMovieTrackSpec {
        track_id: protected_track_id,
        width: 320,
        height: 180,
        sample_entry: build_iaec_protected_sample_entry(salt),
        samples: encrypted_protected_samples,
        chunk_sample_counts: protected_chunk_sample_counts.to_vec(),
    };
    let clear_protected_track = SampleEntryMovieTrackSpec {
        track_id: protected_track_id,
        width: 320,
        height: 180,
        sample_entry: build_clear_avc1_sample_entry(320, 180),
        samples: protected_samples,
        chunk_sample_counts: protected_chunk_sample_counts.to_vec(),
    };
    let clear_track = SampleEntryMovieTrackSpec {
        track_id: clear_track_id,
        width: 640,
        height: 360,
        sample_entry: build_clear_avc1_sample_entry(640, 360),
        samples: clear_track_samples,
        chunk_sample_counts: clear_chunk_sample_counts.to_vec(),
    };

    let encrypted = build_two_track_sample_entry_movie(
        &ftyp,
        &encrypted_protected_track,
        &clear_track,
        &[leading_empty_mdat],
        std::slice::from_ref(&trailing_free),
    );
    let decrypted = build_two_track_sample_entry_movie(
        &ftyp,
        &clear_protected_track,
        &clear_track,
        std::slice::from_ref(&trailing_free),
        &[],
    );

    ProtectedMovieTopologyFixture {
        encrypted,
        decrypted,
        keys: vec![DecryptionKey::track(protected_track_id, key)],
    }
}

#[cfg(feature = "decrypt")]
pub fn build_iaec_sample_description_index_unsupported_movie_fixture()
-> ProtectedMovieTopologyFixture {
    build_sample_description_index_unsupported_protected_movie_fixture(
        build_iaec_broader_movie_fixture(),
        1,
    )
}

#[cfg(feature = "decrypt")]
fn build_sample_description_index_unsupported_protected_movie_fixture(
    fixture: ProtectedMovieTopologyFixture,
    protected_track_id: u32,
) -> ProtectedMovieTopologyFixture {
    ProtectedMovieTopologyFixture {
        encrypted: patch_standard_protected_movie_track_sample_description_index(
            &fixture.encrypted,
            protected_track_id,
        ),
        decrypted: fixture.decrypted,
        keys: fixture.keys,
    }
}

#[cfg(feature = "decrypt")]
fn build_two_track_sample_entry_movie(
    ftyp: &Ftyp,
    protected_track: &SampleEntryMovieTrackSpec,
    clear_track: &SampleEntryMovieTrackSpec,
    root_boxes_before_mdat: &[Vec<u8>],
    root_boxes_after_mdat: &[Vec<u8>],
) -> Vec<u8> {
    let ftyp_bytes = encode_supported_box(ftyp, &[]);
    let protected_chunks = chunk_payloads_from_samples(
        &protected_track.samples,
        &protected_track.chunk_sample_counts,
    );
    let clear_chunks =
        chunk_payloads_from_samples(&clear_track.samples, &clear_track.chunk_sample_counts);

    let protected_placeholder_track = build_sample_entry_movie_track(
        protected_track.track_id,
        protected_track.width,
        protected_track.height,
        protected_track.sample_entry.clone(),
        sample_sizes_u64(&protected_track.samples),
        &protected_track.chunk_sample_counts,
        &vec![0; protected_chunks.len()],
    );
    let clear_placeholder_track = build_sample_entry_movie_track(
        clear_track.track_id,
        clear_track.width,
        clear_track.height,
        clear_track.sample_entry.clone(),
        sample_sizes_u64(&clear_track.samples),
        &clear_track.chunk_sample_counts,
        &vec![0; clear_chunks.len()],
    );
    let moov_placeholder =
        build_simple_movie_moov(&protected_placeholder_track, &clear_placeholder_track);
    let mdat_payload_start = u64::try_from(
        ftyp_bytes.len()
            + moov_placeholder.len()
            + root_boxes_before_mdat.iter().map(Vec::len).sum::<usize>()
            + 8,
    )
    .unwrap();

    let mut protected_offsets = Vec::with_capacity(protected_chunks.len());
    let mut clear_offsets = Vec::with_capacity(clear_chunks.len());
    let mut payload = Vec::new();
    let max_chunks = protected_chunks.len().max(clear_chunks.len());
    for index in 0..max_chunks {
        if let Some(chunk) = clear_chunks.get(index) {
            clear_offsets.push(mdat_payload_start + u64::try_from(payload.len()).unwrap());
            payload.extend_from_slice(chunk);
        }
        if let Some(chunk) = protected_chunks.get(index) {
            protected_offsets.push(mdat_payload_start + u64::try_from(payload.len()).unwrap());
            payload.extend_from_slice(chunk);
        }
    }

    let protected_track = build_sample_entry_movie_track(
        protected_track.track_id,
        protected_track.width,
        protected_track.height,
        protected_track.sample_entry.clone(),
        sample_sizes_u64(&protected_track.samples),
        &protected_track.chunk_sample_counts,
        &protected_offsets,
    );
    let clear_track = build_sample_entry_movie_track(
        clear_track.track_id,
        clear_track.width,
        clear_track.height,
        clear_track.sample_entry.clone(),
        sample_sizes_u64(&clear_track.samples),
        &clear_track.chunk_sample_counts,
        &clear_offsets,
    );
    let moov = build_simple_movie_moov(&protected_track, &clear_track);
    let mdat = encode_raw_box(fourcc("mdat"), &payload);

    let mut output = Vec::new();
    output.extend_from_slice(&ftyp_bytes);
    output.extend_from_slice(&moov);
    for root_box in root_boxes_before_mdat {
        output.extend_from_slice(root_box);
    }
    output.extend_from_slice(&mdat);
    for root_box in root_boxes_after_mdat {
        output.extend_from_slice(root_box);
    }
    output
}

#[cfg(feature = "decrypt")]
fn build_simple_movie_moov(protected_track: &[u8], clear_track: &[u8]) -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 3_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 3;
    let mvhd = encode_supported_box(&mvhd, &[]);

    encode_supported_box(
        &Moov,
        &[mvhd, protected_track.to_vec(), clear_track.to_vec()].concat(),
    )
}

#[cfg(feature = "decrypt")]
fn build_sample_entry_movie_track(
    track_id: u32,
    width: u16,
    height: u16,
    sample_entry: Vec<u8>,
    sample_sizes: Vec<u64>,
    chunk_sample_counts: &[u32],
    chunk_offsets: &[u64],
) -> Vec<u8> {
    let mut tkhd = mp4forge::boxes::iso14496_12::Tkhd::default();
    tkhd.track_id = track_id;
    tkhd.width = u32::from(width) << 16;
    tkhd.height = u32::from(height) << 16;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 3_000;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(&stsd, &sample_entry);

    let mut stco = Stco::default();
    stco.entry_count = u32::try_from(chunk_offsets.len()).unwrap();
    stco.chunk_offset = chunk_offsets.to_vec();
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 0;
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = u32::try_from(chunk_sample_counts.len()).unwrap();
    let mut first_chunk = 1u32;
    stsc.entries = chunk_sample_counts
        .iter()
        .map(|samples_per_chunk| {
            let entry = StscEntry {
                first_chunk,
                samples_per_chunk: *samples_per_chunk,
                sample_description_index: 1,
            };
            first_chunk += 1;
            entry
        })
        .collect();
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = u32::try_from(sample_sizes.len()).unwrap();
    stsz.entry_size = sample_sizes;
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(
        &Mdia,
        &[mdhd, handler_box("vide", "VideoHandler"), minf].concat(),
    );
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

#[cfg(feature = "decrypt")]
fn chunk_payloads_from_samples(samples: &[Vec<u8>], chunk_sample_counts: &[u32]) -> Vec<Vec<u8>> {
    let mut chunks = Vec::with_capacity(chunk_sample_counts.len());
    let mut cursor = 0usize;
    for &sample_count in chunk_sample_counts {
        let sample_count = usize::try_from(sample_count).unwrap();
        let end = cursor + sample_count;
        let mut chunk = Vec::new();
        for sample in &samples[cursor..end] {
            chunk.extend_from_slice(sample);
        }
        chunks.push(chunk);
        cursor = end;
    }
    assert_eq!(cursor, samples.len());
    chunks
}

#[cfg(feature = "decrypt")]
fn sample_sizes_u64(samples: &[Vec<u8>]) -> Vec<u64> {
    samples
        .iter()
        .map(|sample| u64::try_from(sample.len()).unwrap())
        .collect()
}

#[cfg(feature = "decrypt")]
fn build_clear_avc1_sample_entry(width: u16, height: u16) -> Vec<u8> {
    encode_supported_box(
        &video_sample_entry_with_type("avc1", width, height),
        &encode_supported_box(&avc_config(), &[]),
    )
}

#[cfg(feature = "decrypt")]
fn build_oma_dcf_protected_sample_entry() -> Vec<u8> {
    let mut schm = Schm::default();
    schm.set_version(0);
    schm.scheme_type = fourcc("odkm");
    schm.scheme_version = 0x0001_0000;

    let mut odaf = Odaf::default();
    odaf.set_version(0);
    odaf.selective_encryption = false;
    odaf.key_indicator_length = 0;
    odaf.iv_length = 16;

    let mut ohdr = Ohdr::default();
    ohdr.set_version(0);
    ohdr.encryption_method = OHDR_ENCRYPTION_METHOD_AES_CTR;
    ohdr.padding_scheme = OHDR_PADDING_SCHEME_NONE;
    ohdr.content_id = "oma-topology".to_owned();

    let odkm = encode_supported_box(
        &Odkm::default(),
        &[
            encode_supported_box(&odaf, &[]),
            encode_supported_box(&ohdr, &[]),
        ]
        .concat(),
    );
    let schi = encode_supported_box(&Schi, &odkm);
    let sinf = encode_supported_box(
        &Sinf,
        &[
            encode_supported_box(
                &Frma {
                    data_format: fourcc("avc1"),
                },
                &[],
            ),
            encode_supported_box(&schm, &[]),
            schi,
        ]
        .concat(),
    );

    encode_supported_box(
        &video_sample_entry_with_type("encv", 320, 180),
        &[encode_supported_box(&avc_config(), &[]), sinf].concat(),
    )
}

#[cfg(feature = "decrypt")]
fn build_iaec_protected_sample_entry(salt: [u8; 8]) -> Vec<u8> {
    let mut schm = Schm::default();
    schm.set_version(0);
    schm.scheme_type = fourcc("iAEC");
    schm.scheme_version = 0x0001_0000;

    let mut isfm = Isfm::default();
    isfm.set_version(0);
    isfm.selective_encryption = false;
    isfm.key_indicator_length = 0;
    isfm.iv_length = 8;

    let islt = Islt { salt };
    let schi = encode_supported_box(
        &Schi,
        &[
            encode_supported_box(&isfm, &[]),
            encode_supported_box(&islt, &[]),
        ]
        .concat(),
    );
    let sinf = encode_supported_box(
        &Sinf,
        &[
            encode_supported_box(
                &Frma {
                    data_format: fourcc("avc1"),
                },
                &[],
            ),
            encode_supported_box(&schm, &[]),
            schi,
        ]
        .concat(),
    );

    encode_supported_box(
        &video_sample_entry_with_type("encv", 320, 180),
        &[encode_supported_box(&avc_config(), &[]), sinf].concat(),
    )
}

#[cfg(feature = "decrypt")]
fn encrypt_oma_dcf_ctr_movie_sample(sample: &[u8], key: [u8; 16], iv: [u8; 16]) -> Vec<u8> {
    let aes = Aes128::new(&key.into());
    let mut counter = iv;
    let mut ciphertext = vec![0_u8; sample.len()];
    let mut cursor = 0usize;
    while cursor < sample.len() {
        let mut stream_block = Block::<Aes128>::default();
        stream_block.copy_from_slice(&counter);
        aes.encrypt_block(&mut stream_block);
        let chunk_len = 16.min(sample.len() - cursor);
        for index in 0..chunk_len {
            ciphertext[cursor + index] = sample[cursor + index] ^ stream_block[index];
        }
        cursor += chunk_len;
        for byte in counter.iter_mut().rev() {
            *byte = byte.wrapping_add(1);
            if *byte != 0 {
                break;
            }
        }
    }

    [iv.to_vec(), ciphertext].concat()
}

#[cfg(feature = "decrypt")]
fn encrypt_iaec_movie_sample(sample: &[u8], key: [u8; 16], salt: [u8; 8], iv: [u8; 8]) -> Vec<u8> {
    let aes = Aes128::new(&key.into());
    let mut counter = [0_u8; 16];
    counter[..8].copy_from_slice(&salt);
    counter[8..].copy_from_slice(&iv);
    let mut ciphertext = vec![0_u8; sample.len()];
    let mut cursor = 0usize;
    while cursor < sample.len() {
        let mut stream_block = Block::<Aes128>::default();
        stream_block.copy_from_slice(&counter);
        aes.encrypt_block(&mut stream_block);
        let chunk_len = 16.min(sample.len() - cursor);
        for index in 0..chunk_len {
            ciphertext[cursor + index] = sample[cursor + index] ^ stream_block[index];
        }
        cursor += chunk_len;
        for byte in counter.iter_mut().rev() {
            *byte = byte.wrapping_add(1);
            if *byte != 0 {
                break;
            }
        }
    }

    [iv.to_vec(), ciphertext].concat()
}

pub fn read_text(path: &Path) -> String {
    normalize_text(&fs::read_to_string(path).unwrap())
}

pub fn read_golden(relative_path: &str) -> String {
    read_text(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("golden")
            .join(relative_path),
    )
}

pub fn normalize_text(value: &str) -> String {
    value.replace("\r\n", "\n")
}

pub fn build_encrypted_fragmented_video_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 1,
            compatible_brands: vec![fourcc("iso6"), fourcc("dash"), fourcc("cenc")],
        },
        &[],
    );
    let moov = build_encrypted_fragmented_video_moov();
    let moof = build_encrypted_fragmented_video_moof();
    let mdat = encode_raw_box(fourcc("mdat"), &[0xde, 0xad, 0xbe, 0xef]);
    [ftyp, moov, moof, mdat].concat()
}

pub fn build_visual_sample_entry_box_with_trailing_bytes() -> Vec<u8> {
    let pasp = encode_supported_box(
        &Pasp {
            h_spacing: 1,
            v_spacing: 1,
        },
        &[],
    );
    let mut extensions = pasp;
    extensions.extend_from_slice(&visual_sample_entry_trailing_bytes());
    encode_supported_box(&video_sample_entry_with_type("avc1", 640, 360), &extensions)
}

pub fn visual_sample_entry_trailing_bytes() -> Vec<u8> {
    vec![0xde, 0xad, 0xbe]
}

pub fn build_event_message_movie_file() -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("isom"),
            minor_version: 1,
            compatible_brands: vec![fourcc("isom"), fourcc("iso8")],
        },
        &[],
    );
    let moov = build_event_message_moov();
    let emib = encode_supported_box(&event_message_instance_box(), &[]);
    let emeb = encode_supported_box(&Emeb, &[]);
    let mdat = encode_raw_box(fourcc("mdat"), &[0x01, 0x02, 0x03, 0x04]);
    [ftyp, moov, emib, emeb, mdat].concat()
}

#[cfg(feature = "decrypt")]
pub struct DecryptRewriteFixture {
    pub init_segment: Vec<u8>,
    pub media_segment: Vec<u8>,
    pub single_file: Vec<u8>,
    pub all_keys: Vec<DecryptionKey>,
    pub first_track_only_keys: Vec<DecryptionKey>,
    pub first_track_id: u32,
    pub second_track_id: u32,
    pub first_track_plaintext: Vec<u8>,
    pub second_track_plaintext: Vec<u8>,
}

#[cfg(feature = "decrypt")]
pub struct MultiSampleEntryDecryptFixture {
    pub init_segment: Vec<u8>,
    pub media_segment: Vec<u8>,
    pub single_file: Vec<u8>,
    pub decrypted_init_segment: Vec<u8>,
    pub decrypted_media_segment: Vec<u8>,
    pub decrypted_single_file: Vec<u8>,
    pub all_keys: Vec<DecryptionKey>,
    pub ambiguous_track_id_keys: Vec<DecryptionKey>,
}

#[cfg(feature = "decrypt")]
pub struct ZeroKidMultiSampleEntryDecryptFixture {
    pub init_segment: Vec<u8>,
    pub media_segment: Vec<u8>,
    pub single_file: Vec<u8>,
    pub decrypted_init_segment: Vec<u8>,
    pub decrypted_media_segment: Vec<u8>,
    pub decrypted_single_file: Vec<u8>,
    pub ordered_track_id_keys: Vec<DecryptionKey>,
}

#[cfg(feature = "decrypt")]
pub fn build_decrypt_rewrite_fixture() -> DecryptRewriteFixture {
    build_decrypt_rewrite_fixture_with_mode(DecryptFixtureLayout::CommonEncryption)
}

#[cfg(feature = "decrypt")]
pub fn build_piff_decrypt_rewrite_fixture() -> DecryptRewriteFixture {
    build_decrypt_rewrite_fixture_with_mode(DecryptFixtureLayout::PiffCompatibility)
}

#[cfg(feature = "decrypt")]
pub fn build_multi_sample_entry_decrypt_fixture() -> MultiSampleEntryDecryptFixture {
    let first_spec = DecryptFixtureTrackSpec {
        track_id: 1,
        width: 320,
        height: 180,
        scheme_type: fourcc("cbcs"),
        native_scheme: NativeCommonEncryptionScheme::Cbcs,
        key: [0x31; 16],
        kid: [0xc1; 16],
        initialization_vector: vec![],
        constant_iv: Some(vec![
            0x01, 0x13, 0x25, 0x37, 0x49, 0x5b, 0x6d, 0x7f, 0x80, 0x92, 0xa4, 0xb6, 0xc8, 0xda,
            0xec, 0xfe,
        ]),
        per_sample_iv_size: None,
        crypt_byte_block: 1,
        skip_byte_block: 1,
        subsamples: vec![
            SencSubsample {
                bytes_of_clear_data: 4,
                bytes_of_protected_data: 32,
            },
            SencSubsample {
                bytes_of_clear_data: 2,
                bytes_of_protected_data: 16,
            },
        ],
        plaintext: (0u8..54).map(|value| value ^ 0x41).collect(),
        use_fragment_group: false,
        layout: DecryptFixtureLayout::CommonEncryption,
    };
    let second_spec = DecryptFixtureTrackSpec {
        track_id: 1,
        width: 320,
        height: 180,
        scheme_type: fourcc("cbcs"),
        native_scheme: NativeCommonEncryptionScheme::Cbcs,
        key: [0x42; 16],
        kid: [0xd2; 16],
        initialization_vector: vec![],
        constant_iv: Some(vec![
            0xfe, 0xec, 0xda, 0xc8, 0xb6, 0xa4, 0x92, 0x80, 0x7f, 0x6d, 0x5b, 0x49, 0x37, 0x25,
            0x13, 0x01,
        ]),
        per_sample_iv_size: None,
        crypt_byte_block: 1,
        skip_byte_block: 1,
        subsamples: vec![
            SencSubsample {
                bytes_of_clear_data: 6,
                bytes_of_protected_data: 24,
            },
            SencSubsample {
                bytes_of_clear_data: 0,
                bytes_of_protected_data: 24,
            },
        ],
        plaintext: (0u8..54)
            .map(|value| value.wrapping_mul(5) ^ 0x22)
            .collect(),
        use_fragment_group: false,
        layout: DecryptFixtureLayout::CommonEncryption,
    };

    let encrypted_sample_entries = vec![
        build_fragmented_track_sample_entry(&first_spec, true),
        build_fragmented_track_sample_entry(&second_spec, true),
    ];
    let clear_sample_entries = vec![
        build_fragmented_track_sample_entry(&first_spec, false),
        build_fragmented_track_sample_entry(&second_spec, false),
    ];
    let init_segment = build_multi_sample_entry_init_segment(&encrypted_sample_entries);
    let decrypted_init_segment = build_multi_sample_entry_init_segment(&clear_sample_entries);

    let first_ciphertext = encrypt_fixture_sample(&first_spec);
    let second_ciphertext = encrypt_fixture_sample(&second_spec);
    let media_segment = build_multi_sample_entry_media_segment(
        [
            MultiSampleEntryFragmentSpec {
                track_spec: &first_spec,
                payload: &first_ciphertext,
                sample_description_index: None,
                base_media_decode_time: 0,
                sequence_number: 1,
            },
            MultiSampleEntryFragmentSpec {
                track_spec: &second_spec,
                payload: &second_ciphertext,
                sample_description_index: Some(2),
                base_media_decode_time: 1_000,
                sequence_number: 2,
            },
        ],
        true,
    );
    let decrypted_media_segment = build_multi_sample_entry_media_segment(
        [
            MultiSampleEntryFragmentSpec {
                track_spec: &first_spec,
                payload: &first_spec.plaintext,
                sample_description_index: None,
                base_media_decode_time: 0,
                sequence_number: 1,
            },
            MultiSampleEntryFragmentSpec {
                track_spec: &second_spec,
                payload: &second_spec.plaintext,
                sample_description_index: Some(2),
                base_media_decode_time: 1_000,
                sequence_number: 2,
            },
        ],
        false,
    );
    let single_file = [init_segment.clone(), media_segment.clone()].concat();
    let decrypted_single_file = [
        decrypted_init_segment.clone(),
        decrypted_media_segment.clone(),
    ]
    .concat();

    MultiSampleEntryDecryptFixture {
        init_segment,
        media_segment,
        single_file,
        decrypted_init_segment,
        decrypted_media_segment,
        decrypted_single_file,
        all_keys: vec![
            DecryptionKey::kid(first_spec.kid, first_spec.key),
            DecryptionKey::kid(second_spec.kid, second_spec.key),
        ],
        ambiguous_track_id_keys: vec![
            DecryptionKey::track(first_spec.track_id, first_spec.key),
            DecryptionKey::track(second_spec.track_id, second_spec.key),
        ],
    }
}

#[cfg(feature = "decrypt")]
pub fn build_zero_kid_multi_sample_entry_decrypt_fixture() -> ZeroKidMultiSampleEntryDecryptFixture
{
    let first_spec = DecryptFixtureTrackSpec {
        track_id: 1,
        width: 320,
        height: 180,
        scheme_type: fourcc("cbcs"),
        native_scheme: NativeCommonEncryptionScheme::Cbcs,
        key: [0x31; 16],
        kid: [0; 16],
        initialization_vector: vec![],
        constant_iv: Some(vec![
            0x01, 0x13, 0x25, 0x37, 0x49, 0x5b, 0x6d, 0x7f, 0x80, 0x92, 0xa4, 0xb6, 0xc8, 0xda,
            0xec, 0xfe,
        ]),
        per_sample_iv_size: None,
        crypt_byte_block: 1,
        skip_byte_block: 1,
        subsamples: vec![
            SencSubsample {
                bytes_of_clear_data: 4,
                bytes_of_protected_data: 32,
            },
            SencSubsample {
                bytes_of_clear_data: 2,
                bytes_of_protected_data: 16,
            },
        ],
        plaintext: (0u8..54).map(|value| value ^ 0x41).collect(),
        use_fragment_group: false,
        layout: DecryptFixtureLayout::CommonEncryption,
    };
    let second_spec = DecryptFixtureTrackSpec {
        track_id: 1,
        width: 320,
        height: 180,
        scheme_type: fourcc("cbcs"),
        native_scheme: NativeCommonEncryptionScheme::Cbcs,
        key: [0x42; 16],
        kid: [0; 16],
        initialization_vector: vec![],
        constant_iv: Some(vec![
            0xfe, 0xec, 0xda, 0xc8, 0xb6, 0xa4, 0x92, 0x80, 0x7f, 0x6d, 0x5b, 0x49, 0x37, 0x25,
            0x13, 0x01,
        ]),
        per_sample_iv_size: None,
        crypt_byte_block: 1,
        skip_byte_block: 1,
        subsamples: vec![
            SencSubsample {
                bytes_of_clear_data: 6,
                bytes_of_protected_data: 24,
            },
            SencSubsample {
                bytes_of_clear_data: 0,
                bytes_of_protected_data: 24,
            },
        ],
        plaintext: (0u8..54)
            .map(|value| value.wrapping_mul(5) ^ 0x22)
            .collect(),
        use_fragment_group: false,
        layout: DecryptFixtureLayout::CommonEncryption,
    };

    let encrypted_sample_entries = vec![
        build_fragmented_track_sample_entry(&first_spec, true),
        build_fragmented_track_sample_entry(&second_spec, true),
    ];
    let clear_sample_entries = vec![
        build_fragmented_track_sample_entry(&first_spec, false),
        build_fragmented_track_sample_entry(&second_spec, false),
    ];
    let init_segment = build_multi_sample_entry_init_segment(&encrypted_sample_entries);
    let decrypted_init_segment = build_multi_sample_entry_init_segment(&clear_sample_entries);

    let first_ciphertext = encrypt_fixture_sample(&first_spec);
    let second_ciphertext = encrypt_fixture_sample(&second_spec);
    let media_segment = build_multi_sample_entry_media_segment(
        [
            MultiSampleEntryFragmentSpec {
                track_spec: &first_spec,
                payload: &first_ciphertext,
                sample_description_index: None,
                base_media_decode_time: 0,
                sequence_number: 1,
            },
            MultiSampleEntryFragmentSpec {
                track_spec: &second_spec,
                payload: &second_ciphertext,
                sample_description_index: Some(2),
                base_media_decode_time: 1_000,
                sequence_number: 2,
            },
        ],
        true,
    );
    let decrypted_media_segment = build_multi_sample_entry_media_segment(
        [
            MultiSampleEntryFragmentSpec {
                track_spec: &first_spec,
                payload: &first_spec.plaintext,
                sample_description_index: None,
                base_media_decode_time: 0,
                sequence_number: 1,
            },
            MultiSampleEntryFragmentSpec {
                track_spec: &second_spec,
                payload: &second_spec.plaintext,
                sample_description_index: Some(2),
                base_media_decode_time: 1_000,
                sequence_number: 2,
            },
        ],
        false,
    );
    let single_file = [init_segment.clone(), media_segment.clone()].concat();
    let decrypted_single_file = [
        decrypted_init_segment.clone(),
        decrypted_media_segment.clone(),
    ]
    .concat();

    ZeroKidMultiSampleEntryDecryptFixture {
        init_segment,
        media_segment,
        single_file,
        decrypted_init_segment,
        decrypted_media_segment,
        decrypted_single_file,
        ordered_track_id_keys: vec![
            DecryptionKey::track(first_spec.track_id, first_spec.key),
            DecryptionKey::track(second_spec.track_id, second_spec.key),
        ],
    }
}

#[cfg(feature = "decrypt")]
fn build_decrypt_rewrite_fixture_with_mode(layout: DecryptFixtureLayout) -> DecryptRewriteFixture {
    let first_spec = DecryptFixtureTrackSpec {
        track_id: 1,
        width: 320,
        height: 180,
        scheme_type: match layout {
            DecryptFixtureLayout::CommonEncryption => fourcc("cenc"),
            DecryptFixtureLayout::PiffCompatibility => fourcc("piff"),
        },
        native_scheme: NativeCommonEncryptionScheme::Cenc,
        key: [0x11; 16],
        kid: [0xa1; 16],
        initialization_vector: vec![1, 2, 3, 4, 5, 6, 7, 8],
        constant_iv: None,
        per_sample_iv_size: Some(8),
        crypt_byte_block: 0,
        skip_byte_block: 0,
        subsamples: match layout {
            DecryptFixtureLayout::CommonEncryption => vec![],
            DecryptFixtureLayout::PiffCompatibility => vec![SencSubsample {
                bytes_of_clear_data: 4,
                bytes_of_protected_data: 32,
            }],
        },
        plaintext: (0u8..48).map(|value| value ^ 0x35).collect(),
        use_fragment_group: false,
        layout,
    };
    let second_spec = DecryptFixtureTrackSpec {
        track_id: 2,
        width: 640,
        height: 360,
        scheme_type: match layout {
            DecryptFixtureLayout::CommonEncryption => fourcc("cbcs"),
            DecryptFixtureLayout::PiffCompatibility => fourcc("piff"),
        },
        native_scheme: match layout {
            DecryptFixtureLayout::CommonEncryption => NativeCommonEncryptionScheme::Cbcs,
            DecryptFixtureLayout::PiffCompatibility => NativeCommonEncryptionScheme::Cbc1,
        },
        key: [0x22; 16],
        kid: [0xb2; 16],
        initialization_vector: match layout {
            DecryptFixtureLayout::CommonEncryption => vec![],
            DecryptFixtureLayout::PiffCompatibility => {
                vec![
                    0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe, 0x01, 0x23, 0x45, 0x67, 0x89,
                    0xab, 0xcd, 0xef,
                ]
            }
        },
        constant_iv: match layout {
            DecryptFixtureLayout::CommonEncryption => Some(vec![
                0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef,
            ]),
            DecryptFixtureLayout::PiffCompatibility => None,
        },
        per_sample_iv_size: match layout {
            DecryptFixtureLayout::CommonEncryption => None,
            DecryptFixtureLayout::PiffCompatibility => Some(16),
        },
        crypt_byte_block: match layout {
            DecryptFixtureLayout::CommonEncryption => 1,
            DecryptFixtureLayout::PiffCompatibility => 0,
        },
        skip_byte_block: match layout {
            DecryptFixtureLayout::CommonEncryption => 1,
            DecryptFixtureLayout::PiffCompatibility => 0,
        },
        subsamples: match layout {
            DecryptFixtureLayout::CommonEncryption => vec![
                SencSubsample {
                    bytes_of_clear_data: 4,
                    bytes_of_protected_data: 48,
                },
                SencSubsample {
                    bytes_of_clear_data: 2,
                    bytes_of_protected_data: 32,
                },
            ],
            DecryptFixtureLayout::PiffCompatibility => vec![SencSubsample {
                bytes_of_clear_data: 0,
                bytes_of_protected_data: 32,
            }],
        },
        plaintext: match layout {
            DecryptFixtureLayout::CommonEncryption => {
                (0u8..86).map(|value| value.wrapping_mul(7)).collect()
            }
            DecryptFixtureLayout::PiffCompatibility => {
                (0u8..48).map(|value| value.wrapping_mul(7)).collect()
            }
        },
        use_fragment_group: matches!(layout, DecryptFixtureLayout::CommonEncryption),
        layout,
    };

    let first_ciphertext = encrypt_fixture_sample(&first_spec);
    let second_ciphertext = encrypt_fixture_sample(&second_spec);
    let init_segment = build_decrypt_fixture_init_segment(&first_spec, &second_spec);
    let media_segment = build_decrypt_fixture_media_segment(
        &first_spec,
        &second_spec,
        &first_ciphertext,
        &second_ciphertext,
    );
    let single_file = [init_segment.clone(), media_segment.clone()].concat();

    DecryptRewriteFixture {
        init_segment,
        media_segment,
        single_file,
        all_keys: vec![
            DecryptionKey::track(first_spec.track_id, first_spec.key),
            DecryptionKey::kid(second_spec.kid, second_spec.key),
        ],
        first_track_only_keys: vec![DecryptionKey::track(first_spec.track_id, first_spec.key)],
        first_track_id: first_spec.track_id,
        second_track_id: second_spec.track_id,
        first_track_plaintext: first_spec.plaintext,
        second_track_plaintext: second_spec.plaintext,
    }
}

fn build_encrypted_fragmented_video_moov() -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 1_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 2;
    let mvhd = encode_supported_box(&mvhd, &[]);

    let mut trex = Trex::default();
    trex.track_id = 1;
    trex.default_sample_description_index = 1;
    let trex = encode_supported_box(&trex, &[]);
    let mvex = encode_supported_box(&Mvex, &trex);

    encode_supported_box(
        &Moov,
        &[mvhd, build_encrypted_fragmented_video_trak(), mvex].concat(),
    )
}

fn build_encrypted_fragmented_video_trak() -> Vec<u8> {
    let mut tkhd = mp4forge::boxes::iso14496_12::Tkhd::default();
    tkhd.track_id = 1;
    tkhd.width = u32::from(320_u16) << 16;
    tkhd.height = u32::from(180_u16) << 16;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(
        &stsd,
        &encode_supported_box(
            &video_sample_entry_with_type("encv", 320, 180),
            &[
                encode_supported_box(&avc_config(), &[]),
                build_encrypted_fragmented_video_sinf(),
            ]
            .concat(),
        ),
    );

    let mut stco = Stco::default();
    stco.entry_count = 0;
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 0;
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 0;
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 0;
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(
        &Mdia,
        &[mdhd, handler_box("vide", "VideoHandler"), minf].concat(),
    );
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_init_segment(
    first_spec: &DecryptFixtureTrackSpec,
    second_spec: &DecryptFixtureTrackSpec,
) -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: match first_spec.layout {
                DecryptFixtureLayout::CommonEncryption => fourcc("iso6"),
                DecryptFixtureLayout::PiffCompatibility => fourcc("piff"),
            },
            minor_version: 1,
            compatible_brands: match first_spec.layout {
                DecryptFixtureLayout::CommonEncryption => {
                    vec![fourcc("iso6"), fourcc("dash"), fourcc("cenc")]
                }
                DecryptFixtureLayout::PiffCompatibility => {
                    vec![fourcc("piff"), fourcc("iso6"), fourcc("dash")]
                }
            },
        },
        &[],
    );

    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 1_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 3;
    let mvhd = encode_supported_box(&mvhd, &[]);

    let first_trex = build_decrypt_fixture_trex(first_spec);
    let second_trex = build_decrypt_fixture_trex(second_spec);
    let mvex = encode_supported_box(&Mvex, &[first_trex, second_trex].concat());

    let moov = encode_supported_box(
        &Moov,
        &[
            mvhd,
            build_decrypt_fixture_trak(first_spec),
            build_decrypt_fixture_trak(second_spec),
            mvex,
        ]
        .concat(),
    );

    [ftyp, moov].concat()
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_trak(spec: &DecryptFixtureTrackSpec) -> Vec<u8> {
    let mut tkhd = mp4forge::boxes::iso14496_12::Tkhd::default();
    tkhd.track_id = spec.track_id;
    tkhd.width = u32::from(spec.width) << 16;
    tkhd.height = u32::from(spec.height) << 16;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(
        &stsd,
        &encode_supported_box(
            &video_sample_entry_with_type("encv", spec.width, spec.height),
            &[
                encode_supported_box(&avc_config(), &[]),
                build_decrypt_fixture_sinf(spec),
            ]
            .concat(),
        ),
    );

    let mut stco = Stco::default();
    stco.entry_count = 0;
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 0;
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 0;
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 0;
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(
        &Mdia,
        &[mdhd, handler_box("vide", "VideoHandler"), minf].concat(),
    );
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_sinf(spec: &DecryptFixtureTrackSpec) -> Vec<u8> {
    let mut schm = Schm::default();
    schm.set_version(0);
    schm.scheme_type = spec.scheme_type;
    schm.scheme_version = 0x0001_0000;

    let mut tenc = Tenc::default();
    tenc.set_version(match spec.layout {
        DecryptFixtureLayout::CommonEncryption => 1,
        DecryptFixtureLayout::PiffCompatibility => 0,
    });
    tenc.default_crypt_byte_block = spec.crypt_byte_block;
    tenc.default_skip_byte_block = spec.skip_byte_block;
    tenc.default_is_protected = match (spec.layout, spec.native_scheme) {
        (DecryptFixtureLayout::CommonEncryption, _) => 1,
        (DecryptFixtureLayout::PiffCompatibility, NativeCommonEncryptionScheme::Cenc) => 1,
        (DecryptFixtureLayout::PiffCompatibility, NativeCommonEncryptionScheme::Cbc1) => 2,
        (DecryptFixtureLayout::PiffCompatibility, _) => {
            panic!("PIFF fixture layout only supports CTR and full-block CBC tracks")
        }
    };
    tenc.default_per_sample_iv_size = spec.per_sample_iv_size.unwrap_or(0);
    tenc.default_kid = spec.kid;
    if let Some(constant_iv) = &spec.constant_iv {
        tenc.default_constant_iv_size = u8::try_from(constant_iv.len()).unwrap();
        tenc.default_constant_iv = constant_iv.clone();
    }

    let schi_child = match spec.layout {
        DecryptFixtureLayout::CommonEncryption => encode_supported_box(&tenc, &[]),
        DecryptFixtureLayout::PiffCompatibility => build_piff_track_encryption_uuid_box(&tenc),
    };
    let schi = encode_supported_box(&Schi, &schi_child);
    encode_supported_box(
        &Sinf,
        &[
            encode_supported_box(
                &Frma {
                    data_format: fourcc("avc1"),
                },
                &[],
            ),
            encode_supported_box(&schm, &[]),
            schi,
        ]
        .concat(),
    )
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_trex(spec: &DecryptFixtureTrackSpec) -> Vec<u8> {
    let mut trex = Trex::default();
    trex.track_id = spec.track_id;
    trex.default_sample_description_index = 1;
    trex.default_sample_duration = 1_000;
    trex.default_sample_size = u32::try_from(spec.plaintext.len()).unwrap();
    encode_supported_box(&trex, &[])
}

#[cfg(feature = "decrypt")]
struct MultiSampleEntryFragmentSpec<'a> {
    track_spec: &'a DecryptFixtureTrackSpec,
    payload: &'a [u8],
    sample_description_index: Option<u32>,
    base_media_decode_time: u64,
    sequence_number: u32,
}

#[cfg(feature = "decrypt")]
fn build_fragmented_track_sample_entry(spec: &DecryptFixtureTrackSpec, protected: bool) -> Vec<u8> {
    if protected {
        return encode_supported_box(
            &video_sample_entry_with_type("encv", spec.width, spec.height),
            &[
                encode_supported_box(&avc_config(), &[]),
                build_decrypt_fixture_sinf(spec),
            ]
            .concat(),
        );
    }

    encode_supported_box(
        &video_sample_entry_with_type("avc1", spec.width, spec.height),
        &encode_supported_box(&avc_config(), &[]),
    )
}

#[cfg(feature = "decrypt")]
fn build_multi_sample_entry_init_segment(sample_entries: &[Vec<u8>]) -> Vec<u8> {
    let ftyp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("iso6"),
            minor_version: 0,
            compatible_brands: vec![fourcc("iso6"), fourcc("isom"), fourcc("dash")],
        },
        &[],
    );

    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 2_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 2;
    let mvhd = encode_supported_box(&mvhd, &[]);

    let mut trex = Trex::default();
    trex.track_id = 1;
    trex.default_sample_description_index = 1;
    trex.default_sample_duration = 1_000;
    trex.default_sample_size = 54;
    let mvex = encode_supported_box(&Mvex, &encode_supported_box(&trex, &[]));
    let moov = encode_supported_box(
        &Moov,
        &[
            mvhd,
            build_fragmented_track_with_sample_entries(1, 320, 180, sample_entries),
            mvex,
        ]
        .concat(),
    );

    [ftyp, moov].concat()
}

#[cfg(feature = "decrypt")]
fn build_fragmented_track_with_sample_entries(
    track_id: u32,
    width: u16,
    height: u16,
    sample_entries: &[Vec<u8>],
) -> Vec<u8> {
    let mut tkhd = mp4forge::boxes::iso14496_12::Tkhd::default();
    tkhd.track_id = track_id;
    tkhd.width = u32::from(width) << 16;
    tkhd.height = u32::from(height) << 16;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);

    let mut stsd = Stsd::default();
    stsd.entry_count = u32::try_from(sample_entries.len()).unwrap();
    let stsd = encode_supported_box(&stsd, &sample_entries.concat());

    let mut stco = Stco::default();
    stco.entry_count = 0;
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 0;
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 0;
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 0;
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(
        &Mdia,
        &[mdhd, handler_box("vide", "VideoHandler"), minf].concat(),
    );
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

#[cfg(feature = "decrypt")]
fn build_multi_sample_entry_media_segment(
    fragments: [MultiSampleEntryFragmentSpec<'_>; 2],
    encrypted: bool,
) -> Vec<u8> {
    let styp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("msdh"),
            minor_version: 0,
            compatible_brands: vec![fourcc("msdh"), fourcc("msix")],
        },
        &[],
    );
    let mut output = styp;
    for fragment in fragments {
        let moof_placeholder = build_multi_sample_entry_fragment_moof(&fragment, 0, encrypted);
        let data_offset = i32::try_from(moof_placeholder.len() + 8).unwrap();
        let moof = build_multi_sample_entry_fragment_moof(&fragment, data_offset, encrypted);
        let mdat = encode_raw_box(fourcc("mdat"), fragment.payload);
        output.extend_from_slice(&moof);
        output.extend_from_slice(&mdat);
    }
    output
}

#[cfg(feature = "decrypt")]
fn build_multi_sample_entry_fragment_moof(
    fragment: &MultiSampleEntryFragmentSpec<'_>,
    data_offset: i32,
    encrypted: bool,
) -> Vec<u8> {
    let mut mfhd = Mfhd::default();
    mfhd.sequence_number = fragment.sequence_number;
    let mfhd = encode_supported_box(&mfhd, &[]);
    let traf = if encrypted {
        build_decrypt_fixture_traf_with_options(
            fragment.track_spec,
            data_offset,
            fragment.sample_description_index,
            fragment.base_media_decode_time,
        )
    } else {
        build_clear_fragment_traf(
            fragment.track_spec.track_id,
            u32::try_from(fragment.payload.len()).unwrap(),
            data_offset,
            fragment.sample_description_index,
            fragment.base_media_decode_time,
        )
    };
    encode_supported_box(&Moof, &[mfhd, traf].concat())
}

#[cfg(feature = "decrypt")]
fn build_clear_fragment_traf(
    track_id: u32,
    sample_size: u32,
    data_offset: i32,
    sample_description_index: Option<u32>,
    base_media_decode_time: u64,
) -> Vec<u8> {
    let mut tfhd = Tfhd::default();
    let mut tfhd_flags = TFHD_DEFAULT_BASE_IS_MOOF
        | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
        | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT;
    if sample_description_index.is_some() {
        tfhd_flags |= TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT;
    }
    tfhd.set_flags(tfhd_flags);
    tfhd.track_id = track_id;
    tfhd.default_sample_duration = 1_000;
    tfhd.default_sample_size = sample_size;
    if let Some(sample_description_index) = sample_description_index {
        tfhd.sample_description_index = sample_description_index;
    }
    let tfhd = encode_supported_box(&tfhd, &[]);

    let mut tfdt = Tfdt::default();
    tfdt.set_version(1);
    tfdt.base_media_decode_time_v1 = base_media_decode_time;
    let tfdt = encode_supported_box(&tfdt, &[]);

    let mut trun = Trun::default();
    trun.set_flags(TRUN_DATA_OFFSET_PRESENT);
    trun.sample_count = 1;
    trun.data_offset = data_offset;
    let trun = encode_supported_box(&trun, &[]);

    encode_supported_box(&Traf, &[tfhd, tfdt, trun].concat())
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_media_segment(
    first_spec: &DecryptFixtureTrackSpec,
    second_spec: &DecryptFixtureTrackSpec,
    first_ciphertext: &[u8],
    second_ciphertext: &[u8],
) -> Vec<u8> {
    let styp = encode_supported_box(
        &Ftyp {
            major_brand: fourcc("msdh"),
            minor_version: 0,
            compatible_brands: vec![fourcc("msdh"), fourcc("msix")],
        },
        &[],
    );

    let moof_placeholder = build_decrypt_fixture_moof(first_spec, second_spec, 0, 0);
    let first_data_offset = i32::try_from(moof_placeholder.len() + 8).unwrap();
    let second_data_offset = first_data_offset + i32::try_from(first_ciphertext.len()).unwrap();
    let moof = build_decrypt_fixture_moof(
        first_spec,
        second_spec,
        first_data_offset,
        second_data_offset,
    );
    let mdat = encode_raw_box(
        fourcc("mdat"),
        &[first_ciphertext, second_ciphertext].concat(),
    );
    [styp, moof, mdat].concat()
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_moof(
    first_spec: &DecryptFixtureTrackSpec,
    second_spec: &DecryptFixtureTrackSpec,
    first_data_offset: i32,
    second_data_offset: i32,
) -> Vec<u8> {
    let mut mfhd = Mfhd::default();
    mfhd.sequence_number = 1;
    let mfhd = encode_supported_box(&mfhd, &[]);
    let first_traf = build_decrypt_fixture_traf(first_spec, first_data_offset);
    let second_traf = build_decrypt_fixture_traf(second_spec, second_data_offset);
    encode_supported_box(&Moof, &[mfhd, first_traf, second_traf].concat())
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_traf(spec: &DecryptFixtureTrackSpec, data_offset: i32) -> Vec<u8> {
    build_decrypt_fixture_traf_with_options(spec, data_offset, None, 0)
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_traf_with_options(
    spec: &DecryptFixtureTrackSpec,
    data_offset: i32,
    sample_description_index: Option<u32>,
    base_media_decode_time: u64,
) -> Vec<u8> {
    let mut tfhd = Tfhd::default();
    let mut tfhd_flags = TFHD_DEFAULT_BASE_IS_MOOF
        | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
        | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT;
    if sample_description_index.is_some() {
        tfhd_flags |= TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT;
    }
    tfhd.set_flags(tfhd_flags);
    tfhd.track_id = spec.track_id;
    tfhd.default_sample_duration = 1_000;
    tfhd.default_sample_size = u32::try_from(spec.plaintext.len()).unwrap();
    if let Some(sample_description_index) = sample_description_index {
        tfhd.sample_description_index = sample_description_index;
    }
    let tfhd = encode_supported_box(&tfhd, &[]);

    let mut tfdt = Tfdt::default();
    tfdt.set_version(1);
    tfdt.base_media_decode_time_v1 = base_media_decode_time;
    let tfdt = encode_supported_box(&tfdt, &[]);

    let mut trun = Trun::default();
    trun.set_flags(TRUN_DATA_OFFSET_PRESENT);
    trun.sample_count = 1;
    trun.data_offset = data_offset;
    let trun = encode_supported_box(&trun, &[]);

    let mut saiz = Saiz::default();
    saiz.sample_count = 1;
    saiz.sample_info_size = vec![decrypt_fixture_aux_info_size(spec)];
    let saiz = encode_supported_box(&saiz, &[]);

    let mut saio = Saio::default();
    saio.entry_count = 1;
    saio.offset_v0 = vec![0];
    let saio = encode_supported_box(&saio, &[]);

    let senc = match spec.layout {
        DecryptFixtureLayout::CommonEncryption => {
            encode_supported_box(&build_decrypt_fixture_senc(spec), &[])
        }
        DecryptFixtureLayout::PiffCompatibility => {
            let mut uuid = Uuid::default();
            uuid.user_type = UUID_SAMPLE_ENCRYPTION;
            uuid.payload = UuidPayload::SampleEncryption(build_decrypt_fixture_senc(spec));
            encode_supported_box(&uuid, &[])
        }
    };
    let sgpd = if spec.use_fragment_group {
        build_decrypt_fixture_sgpd(spec)
    } else {
        Vec::new()
    };
    let sbgp = if spec.use_fragment_group {
        build_decrypt_fixture_sbgp()
    } else {
        Vec::new()
    };

    encode_supported_box(
        &Traf,
        &[tfhd, tfdt, trun, saiz, saio, senc, sgpd, sbgp].concat(),
    )
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_senc(spec: &DecryptFixtureTrackSpec) -> Senc {
    let mut senc = Senc::default();
    senc.set_version(0);
    if !spec.subsamples.is_empty() {
        senc.set_flags(SENC_USE_SUBSAMPLE_ENCRYPTION);
    }
    senc.sample_count = 1;
    senc.samples = vec![SencSample {
        initialization_vector: spec.initialization_vector.clone(),
        subsamples: spec.subsamples.clone(),
    }];
    senc
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_sgpd(spec: &DecryptFixtureTrackSpec) -> Vec<u8> {
    let mut sgpd = Sgpd::default();
    sgpd.set_version(1);
    sgpd.grouping_type = fourcc("seig");
    sgpd.default_length = 0;
    sgpd.entry_count = 1;
    let mut seig = SeigEntry {
        crypt_byte_block: spec.crypt_byte_block,
        skip_byte_block: spec.skip_byte_block,
        is_protected: 1,
        per_sample_iv_size: spec.per_sample_iv_size.unwrap_or(0),
        kid: spec.kid,
        ..SeigEntry::default()
    };
    if let Some(constant_iv) = &spec.constant_iv {
        seig.constant_iv_size = u8::try_from(constant_iv.len()).unwrap();
        seig.constant_iv = constant_iv.clone();
    }
    sgpd.seig_entries_l = vec![SeigEntryL {
        description_length: decrypt_fixture_seig_description_length(&seig),
        seig_entry: seig,
    }];
    encode_supported_box(&sgpd, &[])
}

#[cfg(feature = "decrypt")]
fn decrypt_fixture_seig_description_length(entry: &SeigEntry) -> u32 {
    let mut length = 20u32;
    if entry.is_protected == 1 && entry.per_sample_iv_size == 0 {
        length += 1 + u32::from(entry.constant_iv_size);
    }
    length
}

#[cfg(feature = "decrypt")]
fn build_decrypt_fixture_sbgp() -> Vec<u8> {
    let mut sbgp = Sbgp::default();
    sbgp.grouping_type = u32::from_be_bytes(*b"seig");
    sbgp.entry_count = 1;
    sbgp.entries = vec![SbgpEntry {
        sample_count: 1,
        group_description_index: 65_537,
    }];
    encode_supported_box(&sbgp, &[])
}

#[cfg(feature = "decrypt")]
fn decrypt_fixture_aux_info_size(spec: &DecryptFixtureTrackSpec) -> u8 {
    let iv_size = spec.per_sample_iv_size.unwrap_or(0);
    let subsample_bytes = if spec.subsamples.is_empty() {
        0
    } else {
        2 + (6 * u32::try_from(spec.subsamples.len()).unwrap())
    };
    u8::try_from(u32::from(iv_size) + subsample_bytes).unwrap()
}

fn build_event_message_moov() -> Vec<u8> {
    let mut mvhd = Mvhd::default();
    mvhd.timescale = 1_000;
    mvhd.duration_v0 = 1_000;
    mvhd.rate = 1 << 16;
    mvhd.volume = 1 << 8;
    mvhd.next_track_id = 2;
    let mvhd = encode_supported_box(&mvhd, &[]);

    encode_supported_box(&Moov, &[mvhd, build_event_message_trak()].concat())
}

fn build_event_message_trak() -> Vec<u8> {
    let mut tkhd = mp4forge::boxes::iso14496_12::Tkhd::default();
    tkhd.track_id = 1;
    tkhd.duration_v0 = 1_000;
    let tkhd = encode_supported_box(&tkhd, &[]);

    let mut mdhd = Mdhd::default();
    mdhd.timescale = 1_000;
    mdhd.duration_v0 = 1_000;
    mdhd.language = [5, 14, 7];
    let mdhd = encode_supported_box(&mdhd, &[]);

    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let stsd = encode_supported_box(&stsd, &event_message_sample_entry_box());

    let mut stco = Stco::default();
    stco.entry_count = 1;
    stco.chunk_offset = vec![0x40];
    let stco = encode_supported_box(&stco, &[]);

    let mut stts = Stts::default();
    stts.entry_count = 1;
    stts.entries = vec![mp4forge::boxes::iso14496_12::SttsEntry {
        sample_count: 1,
        sample_delta: 1_000,
    }];
    let stts = encode_supported_box(&stts, &[]);

    let mut stsc = Stsc::default();
    stsc.entry_count = 1;
    stsc.entries = vec![mp4forge::boxes::iso14496_12::StscEntry {
        first_chunk: 1,
        samples_per_chunk: 1,
        sample_description_index: 1,
    }];
    let stsc = encode_supported_box(&stsc, &[]);

    let mut stsz = Stsz::default();
    stsz.sample_count = 1;
    stsz.entry_size = vec![4];
    let stsz = encode_supported_box(&stsz, &[]);

    let stbl = encode_supported_box(&Stbl, &[stsd, stco, stts, stsc, stsz].concat());
    let minf = encode_supported_box(&Minf, &stbl);
    let mdia = encode_supported_box(
        &Mdia,
        &[mdhd, handler_box("subt", "SubtitleHandler"), minf].concat(),
    );
    encode_supported_box(&Trak, &[tkhd, mdia].concat())
}

fn event_message_sample_entry_box() -> Vec<u8> {
    let entry = EventMessageSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc("evte"),
            data_reference_index: 1,
        },
    };
    let children = [
        encode_supported_box(
            &Btrt {
                buffer_size_db: 32_768,
                max_bitrate: 4_000_000,
                avg_bitrate: 2_500_000,
            },
            &[],
        ),
        encode_supported_box(&event_message_scheme_box(), &[]),
    ]
    .concat();
    encode_supported_box(&entry, &children)
}

pub fn event_message_scheme_box() -> Silb {
    let mut silb = Silb::default();
    silb.set_version(0);
    silb.scheme_count = 2;
    silb.schemes = vec![
        SilbEntry {
            scheme_id_uri: "urn:mpeg:dash:event:2012".to_string(),
            value: "event-1".to_string(),
            at_least_one_flag: false,
        },
        SilbEntry {
            scheme_id_uri: "urn:scte:scte35:2013:bin".to_string(),
            value: "splice".to_string(),
            at_least_one_flag: true,
        },
    ];
    silb.other_schemes_flag = true;
    silb
}

pub fn event_message_instance_box() -> Emib {
    let mut emib = Emib::default();
    emib.set_version(0);
    emib.presentation_time_delta = -1_000;
    emib.event_duration = 2_000;
    emib.id = 1_234;
    emib.scheme_id_uri = "urn:scte:scte35:2013:bin".to_string();
    emib.value = "2".to_string();
    emib.message_data = vec![0x01, 0x02, 0x03];
    emib
}

fn build_encrypted_fragmented_video_sinf() -> Vec<u8> {
    let mut schm = Schm::default();
    schm.set_version(0);
    schm.scheme_type = fourcc("cenc");
    schm.scheme_version = 0x0001_0000;

    let mut tenc = Tenc::default();
    tenc.set_version(1);
    tenc.default_crypt_byte_block = 1;
    tenc.default_skip_byte_block = 9;
    tenc.default_is_protected = 1;
    tenc.default_per_sample_iv_size = 8;
    tenc.default_kid = encrypted_fragment_default_kid();

    let schi = encode_supported_box(&Schi, &encode_supported_box(&tenc, &[]));
    encode_supported_box(
        &Sinf,
        &[
            encode_supported_box(
                &Frma {
                    data_format: fourcc("avc1"),
                },
                &[],
            ),
            encode_supported_box(&schm, &[]),
            schi,
        ]
        .concat(),
    )
}

fn build_encrypted_fragmented_video_moof() -> Vec<u8> {
    let mut mfhd = Mfhd::default();
    mfhd.sequence_number = 1;
    let mfhd = encode_supported_box(&mfhd, &[]);

    let mut tfhd = Tfhd::default();
    tfhd.set_flags(TFHD_DEFAULT_SAMPLE_DURATION_PRESENT | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT);
    tfhd.track_id = 1;
    tfhd.default_sample_duration = 1_000;
    tfhd.default_sample_size = 4;
    let tfhd = encode_supported_box(&tfhd, &[]);

    let mut tfdt = Tfdt::default();
    tfdt.set_version(1);
    tfdt.base_media_decode_time_v1 = 0;
    let tfdt = encode_supported_box(&tfdt, &[]);

    let mut trun = Trun::default();
    trun.sample_count = 1;
    let trun = encode_supported_box(&trun, &[]);

    let mut saiz = Saiz::default();
    saiz.sample_count = 1;
    saiz.sample_info_size = vec![16];
    let saiz = encode_supported_box(&saiz, &[]);

    let mut saio = Saio::default();
    saio.entry_count = 1;
    saio.offset_v0 = vec![0];
    let saio = encode_supported_box(&saio, &[]);

    let mut senc = Senc::default();
    senc.set_version(0);
    senc.set_flags(SENC_USE_SUBSAMPLE_ENCRYPTION);
    senc.sample_count = 1;
    senc.samples = vec![SencSample {
        initialization_vector: vec![1, 2, 3, 4, 5, 6, 7, 8],
        subsamples: vec![SencSubsample {
            bytes_of_clear_data: 32,
            bytes_of_protected_data: 480,
        }],
    }];
    let senc = encode_supported_box(&senc, &[]);

    let mut sgpd = Sgpd::default();
    sgpd.set_version(1);
    sgpd.grouping_type = fourcc("seig");
    sgpd.default_length = 0;
    sgpd.entry_count = 1;
    sgpd.seig_entries_l = vec![SeigEntryL {
        description_length: 20,
        seig_entry: SeigEntry {
            crypt_byte_block: 1,
            skip_byte_block: 9,
            is_protected: 1,
            per_sample_iv_size: 8,
            kid: encrypted_fragment_default_kid(),
            ..SeigEntry::default()
        },
    }];
    let sgpd = encode_supported_box(&sgpd, &[]);

    let mut sbgp = Sbgp::default();
    sbgp.grouping_type = u32::from_be_bytes(*b"seig");
    sbgp.entry_count = 1;
    sbgp.entries = vec![SbgpEntry {
        sample_count: 1,
        group_description_index: 65_537,
    }];
    let sbgp = encode_supported_box(&sbgp, &[]);

    let traf = encode_supported_box(
        &Traf,
        &[tfhd, tfdt, trun, saiz, saio, senc, sgpd, sbgp].concat(),
    );
    encode_supported_box(&Moof, &[mfhd, traf].concat())
}

#[cfg(feature = "decrypt")]
struct DecryptFixtureTrackSpec {
    track_id: u32,
    width: u16,
    height: u16,
    scheme_type: FourCc,
    native_scheme: NativeCommonEncryptionScheme,
    key: [u8; 16],
    kid: [u8; 16],
    initialization_vector: Vec<u8>,
    constant_iv: Option<Vec<u8>>,
    per_sample_iv_size: Option<u8>,
    crypt_byte_block: u8,
    skip_byte_block: u8,
    subsamples: Vec<SencSubsample>,
    plaintext: Vec<u8>,
    use_fragment_group: bool,
    layout: DecryptFixtureLayout,
}

#[cfg(feature = "decrypt")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecryptFixtureLayout {
    CommonEncryption,
    PiffCompatibility,
}

#[cfg(feature = "decrypt")]
fn encrypt_fixture_sample(spec: &DecryptFixtureTrackSpec) -> Vec<u8> {
    let sample = resolved_decrypt_fixture_sample(spec);
    let iv = sample.effective_initialization_vector();
    let pattern = DecryptFixturePattern {
        crypt_byte_block: spec.crypt_byte_block,
        skip_byte_block: spec.skip_byte_block,
    };
    let iv_block = if iv.len() == 16 {
        iv.try_into().unwrap()
    } else {
        let mut padded = [0u8; 16];
        padded[..iv.len()].copy_from_slice(iv);
        padded
    };
    let scheme = spec.native_scheme;
    let mut output = spec.plaintext.clone();

    if sample.subsamples.is_empty() {
        encrypt_fixture_region(
            scheme,
            spec.key,
            iv_block,
            pattern,
            &spec.plaintext,
            &mut output,
        );
        return output;
    }

    let mut cursor = 0usize;
    let mut state = DecryptFixtureEncryptState {
        ctr_offset: 0,
        pattern_offset: 0,
        chain_block: iv_block,
    };
    for subsample in sample.subsamples {
        cursor += usize::from(subsample.bytes_of_clear_data);
        let protected = usize::try_from(subsample.bytes_of_protected_data).unwrap();
        if scheme == NativeCommonEncryptionScheme::Cbcs {
            state.ctr_offset = 0;
            state.pattern_offset = 0;
            state.chain_block = iv_block;
        }
        encrypt_fixture_region_with_state(
            scheme,
            spec.key,
            iv_block,
            pattern,
            &mut state,
            &spec.plaintext[cursor..cursor + protected],
            &mut output[cursor..cursor + protected],
        );
        cursor += protected;
    }

    output
}

#[cfg(feature = "decrypt")]
fn build_piff_track_encryption_uuid_box(tenc: &Tenc) -> Vec<u8> {
    let mut payload = vec![0, 0, 0, 0];
    payload.push(tenc.reserved);
    payload.push(0);
    payload.push(tenc.default_is_protected);
    payload.push(tenc.default_per_sample_iv_size);
    payload.extend_from_slice(&tenc.default_kid);
    if tenc.default_per_sample_iv_size == 0 {
        payload.push(tenc.default_constant_iv_size);
        payload.extend_from_slice(&tenc.default_constant_iv);
    }
    encode_uuid_box(
        [
            0x89, 0x74, 0xdb, 0xce, 0x7b, 0xe7, 0x4c, 0x51, 0x84, 0xf9, 0x71, 0x48, 0xf9, 0x88,
            0x25, 0x54,
        ],
        &payload,
    )
}

#[cfg(feature = "decrypt")]
fn encode_uuid_box(user_type: [u8; 16], payload: &[u8]) -> Vec<u8> {
    let info = BoxInfo::new(fourcc("uuid"), 8 + 16 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(&user_type);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "decrypt")]
fn resolved_decrypt_fixture_sample(
    spec: &DecryptFixtureTrackSpec,
) -> ResolvedSampleEncryptionSample<'static> {
    let initialization_vector = Box::leak(spec.initialization_vector.clone().into_boxed_slice());
    let constant_iv = spec
        .constant_iv
        .clone()
        .map(|bytes| Box::leak(bytes.into_boxed_slice()) as &'static [u8]);
    let subsamples = Box::leak(spec.subsamples.clone().into_boxed_slice());
    ResolvedSampleEncryptionSample {
        sample_index: 1,
        metadata_source: ResolvedSampleEncryptionSource::TrackEncryptionBox,
        is_protected: true,
        crypt_byte_block: spec.crypt_byte_block,
        skip_byte_block: spec.skip_byte_block,
        per_sample_iv_size: spec.per_sample_iv_size,
        initialization_vector,
        constant_iv,
        kid: spec.kid,
        subsamples,
        auxiliary_info_size: 0,
    }
}

#[cfg(feature = "decrypt")]
struct DecryptFixtureEncryptState {
    ctr_offset: u64,
    pattern_offset: u64,
    chain_block: [u8; 16],
}

#[cfg(feature = "decrypt")]
#[derive(Clone, Copy)]
struct DecryptFixturePattern {
    crypt_byte_block: u8,
    skip_byte_block: u8,
}

#[cfg(feature = "decrypt")]
fn encrypt_fixture_region(
    scheme: NativeCommonEncryptionScheme,
    key: [u8; 16],
    iv: [u8; 16],
    pattern: DecryptFixturePattern,
    plaintext: &[u8],
    output: &mut [u8],
) {
    let mut state = DecryptFixtureEncryptState {
        ctr_offset: 0,
        pattern_offset: 0,
        chain_block: iv,
    };
    encrypt_fixture_region_with_state(scheme, key, iv, pattern, &mut state, plaintext, output);
}

#[cfg(feature = "decrypt")]
fn encrypt_fixture_region_with_state(
    scheme: NativeCommonEncryptionScheme,
    key: [u8; 16],
    iv: [u8; 16],
    pattern: DecryptFixturePattern,
    state: &mut DecryptFixtureEncryptState,
    plaintext: &[u8],
    output: &mut [u8],
) {
    if pattern.crypt_byte_block != 0 && pattern.skip_byte_block != 0 {
        let pattern_span =
            usize::from(pattern.crypt_byte_block) + usize::from(pattern.skip_byte_block);
        let mut cursor = 0usize;
        while cursor < plaintext.len() {
            let block_position = usize::try_from(state.pattern_offset / 16).unwrap();
            let pattern_position = block_position % pattern_span;
            let mut crypt_size = 0usize;
            let mut skip_size = usize::from(pattern.skip_byte_block) * 16;
            if pattern_position < usize::from(pattern.crypt_byte_block) {
                crypt_size = (usize::from(pattern.crypt_byte_block) - pattern_position) * 16;
            } else {
                skip_size = (pattern_span - pattern_position) * 16;
            }

            let remain = plaintext.len() - cursor;
            if crypt_size > remain {
                crypt_size = 16 * (remain / 16);
                skip_size = remain - crypt_size;
            }
            if crypt_size + skip_size > remain {
                skip_size = remain - crypt_size;
            }

            if crypt_size != 0 {
                encrypt_fixture_chunk(
                    scheme,
                    key,
                    iv,
                    &mut state.ctr_offset,
                    &mut state.chain_block,
                    &plaintext[cursor..cursor + crypt_size],
                    &mut output[cursor..cursor + crypt_size],
                );
                cursor += crypt_size;
                state.pattern_offset += crypt_size as u64;
            }

            if skip_size != 0 {
                output[cursor..cursor + skip_size]
                    .copy_from_slice(&plaintext[cursor..cursor + skip_size]);
                cursor += skip_size;
                state.pattern_offset += skip_size as u64;
            }
        }
    } else {
        encrypt_fixture_chunk(
            scheme,
            key,
            iv,
            &mut state.ctr_offset,
            &mut state.chain_block,
            plaintext,
            output,
        );
    }
}

#[cfg(feature = "decrypt")]
fn encrypt_fixture_chunk(
    scheme: NativeCommonEncryptionScheme,
    key: [u8; 16],
    iv: [u8; 16],
    ctr_offset: &mut u64,
    chain_block: &mut [u8; 16],
    plaintext: &[u8],
    output: &mut [u8],
) {
    match scheme {
        NativeCommonEncryptionScheme::Cenc | NativeCommonEncryptionScheme::Cens => {
            let aes = Aes128::new(&key.into());
            let mut cursor = 0usize;
            while cursor < plaintext.len() {
                let block_offset = usize::try_from(*ctr_offset % 16).unwrap();
                let chunk_len = (16 - block_offset).min(plaintext.len() - cursor);
                let mut counter_block = compute_fixture_ctr_counter_block(iv, *ctr_offset);
                aes.encrypt_block(&mut counter_block);
                for index in 0..chunk_len {
                    output[cursor + index] =
                        plaintext[cursor + index] ^ counter_block[block_offset + index];
                }
                cursor += chunk_len;
                *ctr_offset += chunk_len as u64;
            }
        }
        NativeCommonEncryptionScheme::Cbc1 | NativeCommonEncryptionScheme::Cbcs => {
            let aes = Aes128::new(&key.into());
            let full_blocks_len = plaintext.len() - (plaintext.len() % 16);
            let mut cursor = 0usize;
            while cursor < full_blocks_len {
                let mut block = Block::<Aes128>::clone_from_slice(&plaintext[cursor..cursor + 16]);
                for index in 0..16 {
                    block[index] ^= chain_block[index];
                }
                aes.encrypt_block(&mut block);
                output[cursor..cursor + 16].copy_from_slice(&block);
                chain_block.copy_from_slice(&block);
                cursor += 16;
            }
            output[full_blocks_len..].copy_from_slice(&plaintext[full_blocks_len..]);
        }
    }
}

#[cfg(feature = "decrypt")]
fn compute_fixture_ctr_counter_block(iv: [u8; 16], stream_offset: u64) -> Block<Aes128> {
    let counter_offset = stream_offset / 16;
    let counter_offset_bytes = counter_offset.to_be_bytes();
    let mut counter_block = Block::<Aes128>::default();

    let mut carry = 0u16;
    for index in 0..8 {
        let offset = 15 - index;
        let sum = u16::from(iv[offset]) + u16::from(counter_offset_bytes[7 - index]) + carry;
        counter_block[offset] = (sum & 0xff) as u8;
        carry = if sum >= 0x100 { 1 } else { 0 };
    }
    for index in 8..16 {
        let offset = 15 - index;
        counter_block[offset] = iv[offset];
    }

    counter_block
}

#[cfg(feature = "mux")]
fn build_test_avi_avih_payload(stream_count: usize, max_chunk_size: usize) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&21_333_u32.to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&u32::try_from(stream_count).unwrap().to_le_bytes());
    payload.extend_from_slice(&u32::try_from(max_chunk_size).unwrap().to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload
}

#[cfg(feature = "mux")]
fn build_test_avi_pcm_stream_list(index: usize, stream: &TestAviPcmStream<'_>) -> Vec<u8> {
    build_test_avi_pcm_stream_list_with_format_tag(index, stream, 0x0001)
}

#[cfg(feature = "mux")]
fn build_test_avi_pcm_stream_list_with_format_tag(
    index: usize,
    stream: &TestAviPcmStream<'_>,
    format_tag: u16,
) -> Vec<u8> {
    let block_align = stream.channel_count * (stream.bits_per_sample / 8);
    let byte_rate = stream.sample_rate * u32::from(block_align);
    let total_samples = stream
        .chunks
        .iter()
        .map(|chunk| u32::try_from(chunk.len()).unwrap() / u32::from(block_align))
        .sum::<u32>();

    let mut strh = Vec::new();
    strh.extend_from_slice(b"auds");
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&0_u16.to_le_bytes());
    strh.extend_from_slice(&0_u16.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&u32::from(block_align).to_le_bytes());
    strh.extend_from_slice(&byte_rate.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&total_samples.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&u32::from(block_align).to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());

    let mut strf = Vec::new();
    strf.extend_from_slice(&format_tag.to_le_bytes());
    strf.extend_from_slice(&stream.channel_count.to_le_bytes());
    strf.extend_from_slice(&stream.sample_rate.to_le_bytes());
    strf.extend_from_slice(&byte_rate.to_le_bytes());
    strf.extend_from_slice(&block_align.to_le_bytes());
    strf.extend_from_slice(&stream.bits_per_sample.to_le_bytes());

    let mut bytes = Vec::new();
    let _ = index;
    bytes.extend_from_slice(&encode_riff_chunk(*b"strh", &strh));
    bytes.extend_from_slice(&encode_riff_chunk(*b"strf", &strf));
    bytes
}

#[cfg(feature = "mux")]
fn build_test_avi_pcm_stream_list_with_extensible_subtype(
    index: usize,
    stream: &TestAviPcmStream<'_>,
    subtype_guid: &[u8; 16],
) -> Vec<u8> {
    let block_align = stream.channel_count * (stream.bits_per_sample / 8);
    let byte_rate = stream.sample_rate * u32::from(block_align);
    let total_samples = stream
        .chunks
        .iter()
        .map(|chunk| u32::try_from(chunk.len()).unwrap() / u32::from(block_align))
        .sum::<u32>();

    let mut strh = Vec::new();
    strh.extend_from_slice(b"auds");
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&0_u16.to_le_bytes());
    strh.extend_from_slice(&0_u16.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&u32::from(block_align).to_le_bytes());
    strh.extend_from_slice(&byte_rate.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&total_samples.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&u32::from(block_align).to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());

    let mut strf = Vec::new();
    strf.extend_from_slice(&0xFFFE_u16.to_le_bytes());
    strf.extend_from_slice(&stream.channel_count.to_le_bytes());
    strf.extend_from_slice(&stream.sample_rate.to_le_bytes());
    strf.extend_from_slice(&byte_rate.to_le_bytes());
    strf.extend_from_slice(&block_align.to_le_bytes());
    strf.extend_from_slice(&stream.bits_per_sample.to_le_bytes());
    strf.extend_from_slice(&22_u16.to_le_bytes());
    strf.extend_from_slice(&stream.bits_per_sample.to_le_bytes());
    strf.extend_from_slice(&0_u32.to_le_bytes());
    strf.extend_from_slice(subtype_guid);

    let mut bytes = Vec::new();
    let _ = index;
    bytes.extend_from_slice(&encode_riff_chunk(*b"strh", &strh));
    bytes.extend_from_slice(&encode_riff_chunk(*b"strf", &strf));
    bytes
}

#[cfg(feature = "mux")]
fn build_test_avi_framed_audio_stream_list(
    format_tag: u16,
    sample_rate: u32,
    channel_count: u16,
    bits_per_sample: u16,
    frames: &[&[u8]],
) -> Vec<u8> {
    let max_chunk_size = frames.iter().map(|frame| frame.len()).max().unwrap_or(0);
    let block_align = u16::try_from(max_chunk_size).unwrap_or(u16::MAX).max(1);
    let sample_duration = match format_tag {
        0x0055 => 1_152,
        0x2000 => 1_536,
        _ => 1,
    };
    let byte_rate = u32::try_from(max_chunk_size)
        .unwrap_or(u32::MAX)
        .saturating_mul(sample_rate)
        / sample_duration.max(1);
    let total_samples = u32::try_from(frames.len()).unwrap();

    let mut strh = Vec::new();
    strh.extend_from_slice(b"auds");
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&0_u16.to_le_bytes());
    strh.extend_from_slice(&0_u16.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&sample_duration.to_le_bytes());
    strh.extend_from_slice(&sample_rate.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&total_samples.to_le_bytes());
    strh.extend_from_slice(
        &u32::try_from(max_chunk_size)
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    strh.extend_from_slice(&u32::MAX.to_le_bytes());
    strh.extend_from_slice(&u32::from(block_align).to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());

    let mut strf = Vec::new();
    strf.extend_from_slice(&format_tag.to_le_bytes());
    strf.extend_from_slice(&channel_count.to_le_bytes());
    strf.extend_from_slice(&sample_rate.to_le_bytes());
    strf.extend_from_slice(&byte_rate.to_le_bytes());
    strf.extend_from_slice(&block_align.to_le_bytes());
    strf.extend_from_slice(&bits_per_sample.to_le_bytes());

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&encode_riff_chunk(*b"strh", &strh));
    bytes.extend_from_slice(&encode_riff_chunk(*b"strf", &strf));
    bytes
}

#[cfg(feature = "mux")]
fn build_test_avi_mp4v_stream_list(stream: &TestAviMp4vStream<'_>) -> Vec<u8> {
    build_test_avi_video_stream_list(
        stream.width,
        stream.height,
        stream.frame_scale,
        stream.frame_rate,
        stream.compression,
        stream.decoder_specific_info,
        stream.frames,
    )
}

#[cfg(feature = "mux")]
fn build_test_avi_movi_payload(streams: &[TestAviPcmStream<'_>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let max_chunk_count = streams
        .iter()
        .map(|stream| stream.chunks.len())
        .max()
        .unwrap_or(0);
    for chunk_index in 0..max_chunk_count {
        for (stream_index, stream) in streams.iter().enumerate() {
            if let Some(chunk) = stream.chunks.get(chunk_index) {
                let chunk_id = format!("{stream_index:02}wb");
                bytes.extend_from_slice(&encode_riff_chunk(
                    chunk_id.as_bytes().try_into().unwrap(),
                    chunk,
                ));
            }
        }
    }
    bytes
}

#[cfg(feature = "mux")]
fn build_test_avi_audio_movi_payload(frames: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for frame in frames {
        bytes.extend_from_slice(&encode_riff_chunk(*b"00wb", frame));
    }
    bytes
}

#[cfg(feature = "mux")]
fn build_test_avi_mp4v_movi_payload(stream: &TestAviMp4vStream<'_>) -> Vec<u8> {
    build_test_avi_video_movi_payload(stream.frames)
}

#[cfg(feature = "mux")]
fn build_test_avi_video_stream_list(
    width: u16,
    height: u16,
    frame_scale: u32,
    frame_rate: u32,
    compression: [u8; 4],
    decoder_specific_info: &[u8],
    frames: &[&[u8]],
) -> Vec<u8> {
    let total_frames = u32::try_from(frames.len()).unwrap();
    let max_chunk_size = frames.iter().map(|frame| frame.len()).max().unwrap_or(0);

    let mut strh = Vec::new();
    strh.extend_from_slice(b"vids");
    strh.extend_from_slice(&compression);
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&0_u16.to_le_bytes());
    strh.extend_from_slice(&0_u16.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&frame_scale.to_le_bytes());
    strh.extend_from_slice(&frame_rate.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&total_frames.to_le_bytes());
    strh.extend_from_slice(&u32::try_from(max_chunk_size).unwrap().to_le_bytes());
    strh.extend_from_slice(&u32::MAX.to_le_bytes());
    strh.extend_from_slice(&0_u32.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&0_i16.to_le_bytes());
    strh.extend_from_slice(&i16::try_from(width).unwrap().to_le_bytes());
    strh.extend_from_slice(&i16::try_from(height).unwrap().to_le_bytes());

    let mut strf = Vec::new();
    strf.extend_from_slice(&40_u32.to_le_bytes());
    strf.extend_from_slice(&i32::from(width).to_le_bytes());
    strf.extend_from_slice(&i32::from(height).to_le_bytes());
    strf.extend_from_slice(&1_u16.to_le_bytes());
    strf.extend_from_slice(&24_u16.to_le_bytes());
    strf.extend_from_slice(&compression);
    strf.extend_from_slice(&u32::try_from(max_chunk_size).unwrap().to_le_bytes());
    strf.extend_from_slice(&0_i32.to_le_bytes());
    strf.extend_from_slice(&0_i32.to_le_bytes());
    strf.extend_from_slice(&0_u32.to_le_bytes());
    strf.extend_from_slice(&0_u32.to_le_bytes());
    strf.extend_from_slice(decoder_specific_info);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&encode_riff_chunk(*b"strh", &strh));
    bytes.extend_from_slice(&encode_riff_chunk(*b"strf", &strf));
    bytes
}

#[cfg(feature = "mux")]
fn build_test_avi_video_movi_payload(frames: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for frame in frames {
        bytes.extend_from_slice(&encode_riff_chunk(*b"00dc", frame));
    }
    bytes
}

#[cfg(feature = "mux")]
fn encode_riff_chunk(chunk_type: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&chunk_type);
    bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(payload);
    if !payload.len().is_multiple_of(2) {
        bytes.push(0);
    }
    bytes
}

#[cfg(feature = "mux")]
fn encode_riff_list(list_type: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut list_payload = Vec::new();
    list_payload.extend_from_slice(&list_type);
    list_payload.extend_from_slice(payload);
    encode_riff_chunk(*b"LIST", &list_payload)
}

#[cfg(feature = "mux")]
fn build_test_program_stream_pack_header() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xBA]);
    bytes.extend_from_slice(&[0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x89, 0xC3, 0xF8, 0x00]);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_program_stream_mp3_pes_packet(payload: &[u8]) -> Vec<u8> {
    build_test_program_stream_mpeg_audio_pes_packet(&build_mp3_frame(payload))
}

#[cfg(feature = "mux")]
fn build_test_program_stream_mp2_pes_packet(payload: &[u8]) -> Vec<u8> {
    build_test_program_stream_mpeg_audio_pes_packet(&build_mp2_frame(payload))
}

#[cfg(feature = "mux")]
fn build_test_program_stream_mpeg_audio_pes_packet(frame: &[u8]) -> Vec<u8> {
    let pes_packet_length = u16::try_from(frame.len() + 3).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xC0]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x00, 0x00]);
    bytes.extend_from_slice(frame);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_program_stream_ac3_pes_packet(payload: &[u8]) -> Vec<u8> {
    let frame = build_ac3_frame(payload);
    let pes_packet_length = u16::try_from(frame.len() + 7).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xBD]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x00, 0x00]);
    bytes.extend_from_slice(&[0x80, 0x00, 0x00, 0x00]);
    bytes.extend_from_slice(&frame);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_program_stream_lpcm_pes_packet(payload: &[u8]) -> Vec<u8> {
    let pes_packet_length = u16::try_from(payload.len() + 7).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xBD]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x00, 0x00]);
    bytes.extend_from_slice(&[0xA0, 0x00, 0x00, 0x01]);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_program_stream_vobsub_pes_packet(
    pts: u64,
    substream_id: u8,
    packet: &[u8],
) -> Vec<u8> {
    let pts_bytes = [
        (((pts >> 29) & 0x0E) as u8) | 0x21,
        ((pts >> 22) & 0xFF) as u8,
        (((pts >> 14) & 0xFE) as u8) | 0x01,
        ((pts >> 7) & 0xFF) as u8,
        (((pts << 1) & 0xFE) as u8) | 0x01,
    ];
    let pes_packet_length = u16::try_from(packet.len() + 12).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xBD]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x80, 0x05]);
    bytes.extend_from_slice(&pts_bytes);
    bytes.extend_from_slice(&[substream_id, 0x00, 0x00, 0x00]);
    bytes.extend_from_slice(packet);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_program_stream_mp4v_pes_packet(payload: &[u8]) -> Vec<u8> {
    build_test_program_stream_video_pes_packet(payload)
}

#[cfg(feature = "mux")]
fn build_test_program_stream_video_pes_packet(payload: &[u8]) -> Vec<u8> {
    let pes_packet_length = u16::try_from(payload.len() + 3).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x00, 0x00]);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_program_stream_video_pes_packet_with_pts(pts: u64, payload: &[u8]) -> Vec<u8> {
    let pts_bytes = [
        (((pts >> 29) & 0x0E) as u8) | 0x21,
        ((pts >> 22) & 0xFF) as u8,
        (((pts >> 14) & 0xFE) as u8) | 0x01,
        ((pts >> 7) & 0xFF) as u8,
        (((pts << 1) & 0xFE) as u8) | 0x01,
    ];
    let pes_packet_length = u16::try_from(payload.len() + 8).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x80, 0x05]);
    bytes.extend_from_slice(&pts_bytes);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_program_stream_video_pes_packet_with_pts_and_dts(
    pts: u64,
    dts: u64,
    payload: &[u8],
) -> Vec<u8> {
    let pts_bytes = [
        (((pts >> 29) & 0x0E) as u8) | 0x31,
        ((pts >> 22) & 0xFF) as u8,
        (((pts >> 14) & 0xFE) as u8) | 0x01,
        ((pts >> 7) & 0xFF) as u8,
        (((pts << 1) & 0xFE) as u8) | 0x01,
    ];
    let dts_bytes = [
        (((dts >> 29) & 0x0E) as u8) | 0x11,
        ((dts >> 22) & 0xFF) as u8,
        (((dts >> 14) & 0xFE) as u8) | 0x01,
        ((dts >> 7) & 0xFF) as u8,
        (((dts << 1) & 0xFE) as u8) | 0x01,
    ];
    let pes_packet_length = u16::try_from(payload.len() + 13).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0xC0, 0x0A]);
    bytes.extend_from_slice(&pts_bytes);
    bytes.extend_from_slice(&dts_bytes);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_program_stream_open_ended_video_pes_packet(payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]);
    bytes.extend_from_slice(&0_u16.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x00, 0x00]);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_pat_packet(continuity_counter: u8) -> Vec<u8> {
    let mut section = Vec::new();
    section.push(0x00);
    section.extend_from_slice(&0xB00D_u16.to_be_bytes());
    section.extend_from_slice(&1_u16.to_be_bytes());
    section.extend_from_slice(&[0xC1, 0x00, 0x00]);
    section.extend_from_slice(&1_u16.to_be_bytes());
    section.extend_from_slice(&0xE100_u16.to_be_bytes());
    section.extend_from_slice(&mpeg2ts_crc32_for_test(&section).to_be_bytes());
    build_test_transport_stream_section_packet(0x0000, continuity_counter, &section)
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_pmt_packet(continuity_counter: u8) -> Vec<u8> {
    build_test_transport_stream_pmt_packet_for_stream_type(continuity_counter, 0x03)
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_pmt_packet_for_stream_type(
    continuity_counter: u8,
    stream_type: u8,
) -> Vec<u8> {
    build_test_transport_stream_pmt_packet_for_stream_type_with_descriptors(
        continuity_counter,
        stream_type,
        &[],
    )
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_pmt_packet_for_private_data(
    continuity_counter: u8,
    descriptors: &[u8],
) -> Vec<u8> {
    build_test_transport_stream_pmt_packet_for_stream_type_with_descriptors(
        continuity_counter,
        0x06,
        descriptors,
    )
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_pmt_packet_for_stream_type_with_descriptors(
    continuity_counter: u8,
    stream_type: u8,
    descriptors: &[u8],
) -> Vec<u8> {
    let mut section = Vec::new();
    section.push(0x02);
    let section_length =
        u16::try_from(18 + descriptors.len()).expect("PMT descriptor payload should fit");
    section.extend_from_slice(&(0xB000_u16 | section_length).to_be_bytes());
    section.extend_from_slice(&1_u16.to_be_bytes());
    section.extend_from_slice(&[0xC1, 0x00, 0x00]);
    section.extend_from_slice(&0xE101_u16.to_be_bytes());
    section.extend_from_slice(&0xF000_u16.to_be_bytes());
    section.push(stream_type);
    section.extend_from_slice(&0xE101_u16.to_be_bytes());
    let es_info_length =
        u16::try_from(descriptors.len()).expect("PMT descriptor payload should fit");
    section.extend_from_slice(&(0xF000_u16 | es_info_length).to_be_bytes());
    section.extend_from_slice(descriptors);
    section.extend_from_slice(&mpeg2ts_crc32_for_test(&section).to_be_bytes());
    build_test_transport_stream_section_packet(0x0100, continuity_counter, &section)
}

#[cfg(feature = "mux")]
fn mpeg2ts_crc32_for_test(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in data {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_section_packet(
    pid: u16,
    continuity_counter: u8,
    section: &[u8],
) -> Vec<u8> {
    let mut packet = vec![0xFF; TS_PACKET_SIZE];
    packet[0] = 0x47;
    packet[1] = 0x40 | u8::try_from((pid >> 8) & 0x1F).unwrap();
    packet[2] = u8::try_from(pid & 0xFF).unwrap();
    packet[3] = 0x10 | (continuity_counter & 0x0F);
    packet[4] = 0x00;
    let payload_end = 5 + section.len();
    packet[5..payload_end].copy_from_slice(section);
    packet
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_mp3_pes_packet(payload: &[u8]) -> Vec<u8> {
    let frame = build_mp3_frame(payload);
    build_test_transport_stream_mpeg_audio_pes_packet(&frame)
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_mpeg_audio_pes_packet(payload: &[u8]) -> Vec<u8> {
    let pes_packet_length = u16::try_from(payload.len() + 3).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xC0]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x00, 0x00]);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_mpeg_audio_pes_packet_with_pts(pts: u64, payload: &[u8]) -> Vec<u8> {
    let pts_bytes = [
        (((pts >> 29) & 0x0E) as u8) | 0x21,
        ((pts >> 22) & 0xFF) as u8,
        (((pts >> 14) & 0xFE) as u8) | 0x01,
        ((pts >> 7) & 0xFF) as u8,
        (((pts << 1) & 0xFE) as u8) | 0x01,
    ];
    let pes_packet_length = u16::try_from(payload.len() + 8).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xC0]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x80, 0x05]);
    bytes.extend_from_slice(&pts_bytes);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_mp4v_pes_packet(payload: &[u8]) -> Vec<u8> {
    build_test_transport_stream_video_pes_packet(payload)
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_mpeg2v_pes_packet_with_pts(pts: u64, payload: &[u8]) -> Vec<u8> {
    build_test_transport_stream_video_pes_packet_with_pts(pts, payload)
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_private_data_pes_packet(payload: &[u8]) -> Vec<u8> {
    let pes_packet_length = u16::try_from(payload.len() + 3).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xBD]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x00, 0x00]);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_video_pes_packet(payload: &[u8]) -> Vec<u8> {
    let pes_packet_length = u16::try_from(payload.len() + 3).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x00, 0x00]);
    bytes.extend_from_slice(payload);
    bytes
}

fn build_test_transport_stream_video_pes_packet_with_pts(pts: u64, payload: &[u8]) -> Vec<u8> {
    let pts_bytes = [
        (((pts >> 29) & 0x0E) as u8) | 0x21,
        ((pts >> 22) & 0xFF) as u8,
        (((pts >> 14) & 0xFE) as u8) | 0x01,
        ((pts >> 7) & 0xFF) as u8,
        (((pts << 1) & 0xFE) as u8) | 0x01,
    ];
    let pes_packet_length = u16::try_from(payload.len() + 8).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]);
    bytes.extend_from_slice(&pes_packet_length.to_be_bytes());
    bytes.extend_from_slice(&[0x80, 0x80, 0x05]);
    bytes.extend_from_slice(&pts_bytes);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_dvb_subtitle_descriptor(
    language: [u8; 3],
    subtitle_type: u8,
    composition_page_id: u16,
    ancillary_page_id: u16,
) -> Vec<u8> {
    let mut bytes = vec![0x59, 8];
    bytes.extend_from_slice(&language);
    bytes.push(subtitle_type);
    bytes.extend_from_slice(&composition_page_id.to_be_bytes());
    bytes.extend_from_slice(&ancillary_page_id.to_be_bytes());
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_dvb_teletext_descriptor(
    language: [u8; 3],
    teletext_type: u8,
    page_byte: u8,
) -> Vec<u8> {
    let mut bytes = vec![0x56, 5];
    bytes.extend_from_slice(&language);
    bytes.push(teletext_type);
    bytes.push(page_byte);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_registration_descriptor(registration: [u8; 4]) -> Vec<u8> {
    let mut bytes = vec![0x05, 4];
    bytes.extend_from_slice(&registration);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_private_data_specifier_descriptor(specifier: [u8; 4]) -> Vec<u8> {
    let mut bytes = vec![0x5F, 4];
    bytes.extend_from_slice(&specifier);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_av1_video_descriptor() -> Vec<u8> {
    vec![0x80, 4, 0x81, 0x00, 0x0C, 0xC0]
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_avs3_registration_descriptor(decoder_config: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0x05,
        u8::try_from(4 + decoder_config.len()).expect("AVS3 registration descriptor should fit"),
    ];
    bytes.extend_from_slice(b"AVSV");
    bytes.extend_from_slice(decoder_config);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_av1_sample_bytes(frame_payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&build_test_transport_stream_av1_framed_obu(
        &build_test_av1_temporal_delimiter_obu(),
    ));
    for obu in split_test_av1_obu_units(frame_payload) {
        bytes.extend_from_slice(&build_test_transport_stream_av1_framed_obu(&obu));
    }
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_av1_framed_obu(obu: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(3 + obu.len());
    bytes.extend_from_slice(&[0x00, 0x00, 0x01]);
    let mut zero_run = 0usize;
    for &byte in obu {
        if zero_run >= 2 && byte <= 0x03 {
            bytes.push(0x03);
            zero_run = 0;
        }
        bytes.push(byte);
        if byte == 0x00 {
            zero_run += 1;
        } else {
            zero_run = 0;
        }
    }
    bytes
}

#[cfg(feature = "mux")]
fn packetize_test_transport_stream_pes(
    pid: u16,
    continuity_counter: &mut u8,
    pes_packet: &[u8],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut offset = 0usize;
    let mut first = true;
    while offset < pes_packet.len() {
        let mut packet = vec![0xFF; TS_PACKET_SIZE];
        packet[0] = 0x47;
        packet[1] = (if first { 0x40 } else { 0x00 }) | u8::try_from((pid >> 8) & 0x1F).unwrap();
        packet[2] = u8::try_from(pid & 0xFF).unwrap();

        let remaining = pes_packet.len() - offset;
        if remaining >= 184 {
            packet[3] = 0x10 | (*continuity_counter & 0x0F);
            let payload_end = offset + 184;
            packet[4..188].copy_from_slice(&pes_packet[offset..payload_end]);
            offset = payload_end;
        } else {
            let adaptation_length = 183 - remaining;
            packet[3] = 0x30 | (*continuity_counter & 0x0F);
            packet[4] = u8::try_from(adaptation_length).unwrap();
            if adaptation_length > 0 {
                packet[5] = 0x00;
                for byte in &mut packet[6..(5 + adaptation_length)] {
                    *byte = 0xFF;
                }
            }
            let payload_start = 5 + adaptation_length;
            packet[payload_start..payload_start + remaining]
                .copy_from_slice(&pes_packet[offset..offset + remaining]);
            offset = pes_packet.len();
        }
        *continuity_counter = (*continuity_counter + 1) & 0x0F;
        first = false;
        bytes.extend_from_slice(&packet);
    }
    bytes
}

#[cfg(feature = "mux")]
fn build_test_transport_stream_avs3_decoder_config(sequence_header: &[u8]) -> Vec<u8> {
    assert!(sequence_header.len() >= 6);
    let mut bytes = Vec::with_capacity(10);
    bytes.push(1);
    bytes.extend_from_slice(&6_u16.to_be_bytes());
    bytes.extend_from_slice(&sequence_header[..6]);
    bytes.push(0xFC);
    bytes
}

#[cfg(feature = "mux")]
fn build_test_avs3_sequence_header_bytes(width: u16, height: u16, frame_rate_code: u8) -> Vec<u8> {
    let mut writer = BitWriter::new(Vec::new());
    write_test_bits_u64(&mut writer, 0x20, 8);
    write_test_bits_u64(&mut writer, 0x10, 8);
    write_test_bits_u64(&mut writer, 1, 1);
    write_test_bits_u64(&mut writer, 0, 1);
    write_test_bits_u64(&mut writer, 0, 2);
    write_test_bits_u64(&mut writer, 1, 1);
    write_test_bits_u64(&mut writer, u64::from(width), 14);
    write_test_bits_u64(&mut writer, 1, 1);
    write_test_bits_u64(&mut writer, u64::from(height), 14);
    write_test_bits_u64(&mut writer, 1, 2);
    write_test_bits_u64(&mut writer, 1, 3);
    write_test_bits_u64(&mut writer, 1, 1);
    write_test_bits_u64(&mut writer, 1, 4);
    write_test_bits_u64(&mut writer, u64::from(frame_rate_code), 4);
    write_test_bits_u64(&mut writer, 1, 1);
    write_test_bits_u64(&mut writer, 0, 18);
    write_test_bits_u64(&mut writer, 1, 1);
    write_test_bits_u64(&mut writer, 0, 12);
    write_test_bits_u64(&mut writer, 1, 1);
    align_test_bit_writer(&mut writer);

    let mut bytes = vec![0x00, 0x00, 0x01, 0xB0];
    bytes.extend_from_slice(&writer.into_inner().unwrap());
    bytes
}

#[cfg(feature = "mux")]
fn build_test_avs3_picture_bytes(is_sync_sample: bool, payload: &[u8]) -> Vec<u8> {
    let start_code = if is_sync_sample { 0xB3 } else { 0xB6 };
    let picture_type = if is_sync_sample { 0x00 } else { 0x01 };
    let mut bytes = vec![
        0x00,
        0x00,
        0x01,
        start_code,
        0x00,
        0x00,
        0x00,
        0x00,
        picture_type,
        0x00,
        0x00,
        0x01,
        0x00,
    ];
    bytes.extend_from_slice(payload);
    bytes
}

fn encrypted_fragment_default_kid() -> [u8; 16] {
    [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc,
        0xfe,
    ]
}

fn avc_config() -> AVCDecoderConfiguration {
    AVCDecoderConfiguration {
        configuration_version: 1,
        profile: 0x64,
        profile_compatibility: 0,
        level: 0x1f,
        length_size_minus_one: 3,
        ..AVCDecoderConfiguration::default()
    }
}

fn handler_box(handler_type: &str, name: &str) -> Vec<u8> {
    let mut hdlr = Hdlr::default();
    hdlr.handler_type = fourcc(handler_type);
    hdlr.name = name.to_string();
    encode_supported_box(&hdlr, &[])
}

fn video_sample_entry_with_type(box_type: &str, width: u16, height: u16) -> VisualSampleEntry {
    let mut entry = VisualSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc(box_type),
            data_reference_index: 1,
        },
        width,
        height,
        frame_count: 1,
        ..VisualSampleEntry::default()
    };
    entry.set_box_type(fourcc(box_type));
    entry
}
