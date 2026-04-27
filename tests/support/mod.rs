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
use mp4forge::boxes::iso14496_12::{TFHD_DEFAULT_BASE_IS_MOOF, TRUN_DATA_OFFSET_PRESENT};
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
#[cfg(feature = "decrypt")]
use mp4forge::walk::BoxPath;
use mp4forge::{BoxInfo, FourCc};

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
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "mp4forge-{prefix}-{}-{unique}.mp4",
        std::process::id()
    ));
    fs::write(&path, data).unwrap();
    path
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
) -> Vec<u8> {
    let chunk_offset_info = match track.chunk_offsets {
        RetainedTrackChunkOffsetState::Stco { info, .. }
        | RetainedTrackChunkOffsetState::Co64 { info, .. } => info,
    };
    let stbl = rebuild_container_box_with_replacements(
        input,
        track.stbl_info,
        &Stbl,
        &BTreeMap::from([
            (track.stsz_info.offset(), encode_supported_box(&stsz, &[])),
            (chunk_offset_info.offset(), chunk_offset_box),
        ]),
    );
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
pub fn build_decrypt_rewrite_fixture() -> DecryptRewriteFixture {
    build_decrypt_rewrite_fixture_with_mode(DecryptFixtureLayout::CommonEncryption)
}

#[cfg(feature = "decrypt")]
pub fn build_piff_decrypt_rewrite_fixture() -> DecryptRewriteFixture {
    build_decrypt_rewrite_fixture_with_mode(DecryptFixtureLayout::PiffCompatibility)
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
    let mut tfhd = Tfhd::default();
    tfhd.set_flags(
        TFHD_DEFAULT_BASE_IS_MOOF
            | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
            | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT,
    );
    tfhd.track_id = spec.track_id;
    tfhd.default_sample_duration = 1_000;
    tfhd.default_sample_size = u32::try_from(spec.plaintext.len()).unwrap();
    let tfhd = encode_supported_box(&tfhd, &[]);

    let mut tfdt = Tfdt::default();
    tfdt.set_version(1);
    tfdt.base_media_decode_time_v1 = 0;
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
