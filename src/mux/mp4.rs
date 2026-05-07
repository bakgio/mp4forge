use std::collections::{BTreeMap, btree_map::Entry};
use std::fs::File;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufWriter};

use crate::FourCc;
#[cfg(feature = "async")]
use crate::async_io::{AsyncReadSeek, AsyncWrite};
use crate::boxes::AnyTypeBox;
use crate::boxes::iso14496_12::{
    AudioSampleEntry, Co64, Ctts, CttsEntry, Dinf, Dref, Edts, Elst, ElstEntry, Ftyp, Hdlr, Mdhd,
    Mdia, Mehd, Meta, Mfhd, Minf, Moof, Moov, Mvex, Mvhd, Nmhd, Sbgp, SbgpEntry, Sgpd, Sidx,
    SidxReference, Smhd, Stbl, Stco, Sthd, Stsc, StscEntry, Stsd, Stss, Stsz, Stts, SttsEntry,
    TFHD_DEFAULT_BASE_IS_MOOF, TFHD_DEFAULT_SAMPLE_DURATION_PRESENT,
    TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT, TFHD_DEFAULT_SAMPLE_SIZE_PRESENT,
    TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT, TRUN_DATA_OFFSET_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_FLAGS_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, Tfdt, Tfhd, Tkhd, Traf, Trak, Trex, Trun,
    TrunEntry, Udta, Url, VisualSampleEntry, Vmhd, split_box_children_with_optional_trailing_bytes,
};
use crate::boxes::iso14496_14::{
    Descriptor, ES_DESCRIPTOR_TAG, Esds, InitialObjectDescriptor, Iods,
};
use crate::boxes::metadata::{DATA_TYPE_STRING_UTF8, Data, Id32, Ilst, IlstMetaContainer};
use crate::codec::{CodecBox, ImmutableBox, MutableBox, marshal, unmarshal};
use crate::header::BoxInfo;

#[cfg(feature = "async")]
use super::copy_planned_payloads_async;
use super::{
    MuxError, MuxFileConfig, MuxPlan, MuxTrackConfig, MuxTrackKind, copy_planned_payloads,
};

const IDENTITY_MATRIX: [i32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
const TKHD_FLAGS_TRACK_ENABLED: u32 = 0x0000_0001;
const TKHD_FLAGS_TRACK_IN_MOVIE: u32 = 0x0000_0002;
const TKHD_FLAGS_TRACK_IN_PREVIEW: u32 = 0x0000_0004;
const VMHD_DEFAULT_FLAGS: u32 = 0x0000_0001;
const NON_KEY_SAMPLE_FLAGS: u32 = 0x0001_0000;
const ID3_OWNER: &str = env!("CARGO_PKG_REPOSITORY");
const ID3_VERSION: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
const ISOM_UNIX_EPOCH_OFFSET: u64 = 2_082_844_800;
const AUTO_FLAT_MOVIE_TIMESCALE: u32 = 600;
const DEFAULT_FREE_PADDING_SIZE: usize = 67;
const TOOL_METADATA_ITEM_TYPE: FourCc = FourCc::from_bytes([0xa9, b't', b'o', b'o']);

pub(super) fn write_mp4_mux<R, W>(
    sources: &mut [R],
    writer: &mut W,
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    R: Read + Seek,
    W: Write,
{
    let layout = build_container_layout(file_config, track_configs, plan)?;
    writer.write_all(&layout.ftyp_bytes)?;
    writer.write_all(&layout.moov_bytes)?;
    writer.write_all(&layout.mdat_header)?;
    copy_planned_payloads(sources, writer, plan)?;
    if !layout.trailing_bytes.is_empty() {
        writer.write_all(&layout.trailing_bytes)?;
    }
    writer.flush()?;
    Ok(())
}

pub(super) fn write_mp4_mux_to_path<P, Q>(
    source_paths: &[P],
    output_path: Q,
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let mut sources = source_paths
        .iter()
        .map(File::open)
        .collect::<Result<Vec<_>, _>>()?;
    let mut writer = File::create(output_path)?;
    write_mp4_mux(&mut sources, &mut writer, file_config, track_configs, plan)
}

#[cfg(feature = "async")]
pub(super) async fn write_mp4_mux_async<R, W>(
    sources: &mut [R],
    writer: &mut W,
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    R: AsyncReadSeek,
    W: AsyncWrite + Unpin,
{
    let layout = build_container_layout(file_config, track_configs, plan)?;
    writer.write_all(&layout.ftyp_bytes).await?;
    writer.write_all(&layout.moov_bytes).await?;
    writer.write_all(&layout.mdat_header).await?;
    copy_planned_payloads_async(sources, writer, plan).await?;
    if !layout.trailing_bytes.is_empty() {
        writer.write_all(&layout.trailing_bytes).await?;
    }
    writer.flush().await?;
    Ok(())
}

#[cfg(feature = "async")]
pub(super) async fn write_mp4_mux_to_path_async<P, Q>(
    source_paths: &[P],
    output_path: Q,
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let mut sources = Vec::with_capacity(source_paths.len());
    for path in source_paths {
        sources.push(TokioFile::open(path).await?);
    }
    let output = TokioFile::create(output_path).await?;
    let mut writer = BufWriter::new(output);
    write_mp4_mux_async(&mut sources, &mut writer, file_config, track_configs, plan).await
}

pub(super) fn write_fragmented_mp4_mux<R, W>(
    sources: &mut [R],
    writer: &mut W,
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    single_sidx_reference: bool,
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    R: Read + Seek,
    W: Write,
{
    let layout = build_fragmented_layout(file_config, track_configs, single_sidx_reference, plan)?;
    writer.write_all(&layout.ftyp_bytes)?;
    writer.write_all(&layout.moov_bytes)?;
    writer.write_all(&layout.sidx_bytes)?;
    for fragment in &layout.fragments {
        writer.write_all(&fragment.moof_bytes)?;
        writer.write_all(&fragment.mdat_header)?;
        copy_fragment_payloads(sources, writer, fragment)?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(feature = "async")]
pub(super) async fn write_fragmented_mp4_mux_async<R, W>(
    sources: &mut [R],
    writer: &mut W,
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    single_sidx_reference: bool,
    plan: &MuxPlan,
) -> Result<(), MuxError>
where
    R: AsyncReadSeek,
    W: AsyncWrite + Unpin,
{
    let layout = build_fragmented_layout(file_config, track_configs, single_sidx_reference, plan)?;
    writer.write_all(&layout.ftyp_bytes).await?;
    writer.write_all(&layout.moov_bytes).await?;
    writer.write_all(&layout.sidx_bytes).await?;
    for fragment in &layout.fragments {
        writer.write_all(&fragment.moof_bytes).await?;
        writer.write_all(&fragment.mdat_header).await?;
        copy_fragment_payloads_async(sources, writer, fragment).await?;
    }
    writer.flush().await?;
    Ok(())
}

struct ContainerLayout {
    ftyp_bytes: Vec<u8>,
    moov_bytes: Vec<u8>,
    mdat_header: Vec<u8>,
    trailing_bytes: Vec<u8>,
}

struct FragmentedLayout {
    ftyp_bytes: Vec<u8>,
    moov_bytes: Vec<u8>,
    sidx_bytes: Vec<u8>,
    fragments: Vec<FragmentLayout>,
}

struct FragmentLayout {
    moof_bytes: Vec<u8>,
    mdat_header: Vec<u8>,
    samples: Vec<PreparedSample>,
}

type SampleEntryChildBoxes = Vec<Vec<u8>>;
type SampleEntryTrailingBytes = Vec<u8>;
type SampleEntryParts<T> = (T, SampleEntryChildBoxes, SampleEntryTrailingBytes);

struct PreparedTrack<'a> {
    config: &'a MuxTrackConfig,
    sample_entry_box: &'a [u8],
    samples: Vec<PreparedSample>,
    chunk_sample_counts: Vec<u32>,
    media_duration: u64,
    presentation_duration_media: u64,
    edit_media_time: Option<u64>,
    flat_timing_override: Option<&'a super::FlatTimingOverride>,
}

#[derive(Clone, Copy)]
struct PreparedSample {
    source_index: usize,
    source_data_offset: u64,
    decode_time_media: u64,
    output_offset: u64,
    sample_size: u64,
    duration_movie: u32,
    duration_media: u32,
    composition_offset_media: i32,
    is_sync_sample: bool,
}

fn build_container_layout(
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    plan: &MuxPlan,
) -> Result<ContainerLayout, MuxError> {
    if file_config.movie_timescale() == 0 {
        return Err(MuxError::InvalidMovieTimescale);
    }

    let prepared_tracks = prepare_tracks(file_config, track_configs, plan)?;
    let ftyp_bytes = build_ftyp_bytes(file_config, &prepared_tracks)?;
    let mdat_header = encode_header_only(
        FourCc::from_bytes(*b"mdat"),
        plan.total_payload_size(),
        "mdat header",
    )?;
    let provisional_moov = build_moov_bytes(
        file_config,
        &prepared_tracks,
        u64::try_from(ftyp_bytes.len()).map_err(|_| MuxError::LayoutOverflow("ftyp size"))?,
        u64::try_from(mdat_header.len()).map_err(|_| MuxError::LayoutOverflow("mdat header"))?,
        0,
    )?;
    let moov_size =
        u64::try_from(provisional_moov.len()).map_err(|_| MuxError::LayoutOverflow("moov size"))?;
    let mdat_data_start = u64::try_from(ftyp_bytes.len())
        .map_err(|_| MuxError::LayoutOverflow("ftyp size"))?
        .checked_add(moov_size)
        .and_then(|offset| offset.checked_add(u64::try_from(mdat_header.len()).ok()?))
        .ok_or(MuxError::LayoutOverflow("mdat data start"))?;
    let moov_bytes = build_moov_bytes(
        file_config,
        &prepared_tracks,
        u64::try_from(ftyp_bytes.len()).map_err(|_| MuxError::LayoutOverflow("ftyp size"))?,
        u64::try_from(mdat_header.len()).map_err(|_| MuxError::LayoutOverflow("mdat header"))?,
        mdat_data_start,
    )?;
    let trailing_bytes = if file_config.auto_flat_profile() {
        build_free_padding_bytes()?
    } else {
        Vec::new()
    };

    if moov_bytes.len() != provisional_moov.len() {
        return Err(MuxError::LayoutOverflow(
            "moov size changed after chunk-offset resolution",
        ));
    }

    Ok(ContainerLayout {
        ftyp_bytes,
        moov_bytes,
        mdat_header,
        trailing_bytes,
    })
}

fn build_fragmented_layout(
    file_config: &MuxFileConfig,
    track_configs: &[MuxTrackConfig],
    single_sidx_reference: bool,
    plan: &MuxPlan,
) -> Result<FragmentedLayout, MuxError> {
    if file_config.movie_timescale() == 0 {
        return Err(MuxError::InvalidMovieTimescale);
    }

    let prepared_tracks = prepare_tracks(file_config, track_configs, plan)?;
    let [track] = prepared_tracks.as_slice() else {
        return Err(MuxError::InvalidOutputLayout {
            layout: "fragmented",
            message: "the current fragmented mux writer expects exactly one prepared track"
                .to_string(),
        });
    };
    let fragment_layouts = build_fragment_layouts(file_config, track)?;
    let ftyp_bytes = build_fragmented_ftyp_bytes(track)?;
    let moov_bytes = build_fragmented_moov_bytes(file_config, &prepared_tracks)?;
    let sidx_bytes =
        build_sidx_bytes(file_config, track, &fragment_layouts, single_sidx_reference)?;

    Ok(FragmentedLayout {
        ftyp_bytes,
        moov_bytes,
        sidx_bytes,
        fragments: fragment_layouts,
    })
}

fn build_fragmented_ftyp_bytes(track: &PreparedTrack<'_>) -> Result<Vec<u8>, MuxError> {
    let sample_entry_type = sample_entry_box_type(track.sample_entry_box)?;
    let mut compatible_brands = vec![
        FourCc::from_bytes(*b"iso8"),
        FourCc::from_bytes(*b"isom"),
        FourCc::from_bytes(*b"mp41"),
        FourCc::from_bytes(*b"dash"),
    ];
    match sample_entry_type {
        value if value == FourCc::from_bytes(*b"avc1") => {
            compatible_brands.push(FourCc::from_bytes(*b"avc1"));
            compatible_brands.push(FourCc::from_bytes(*b"cmfc"));
        }
        value
            if matches!(
                value,
                _
                    if value == FourCc::from_bytes(*b"hvc1")
                        || value == FourCc::from_bytes(*b"hev1")
                        || value == FourCc::from_bytes(*b"dvh1")
                        || value == FourCc::from_bytes(*b"dvhe")
            ) =>
        {
            compatible_brands.push(FourCc::from_bytes(*b"hev1"));
            if value == FourCc::from_bytes(*b"dvh1") || value == FourCc::from_bytes(*b"dvhe") {
                compatible_brands.push(FourCc::from_bytes(*b"dby1"));
            }
        }
        value if value == FourCc::from_bytes(*b"vvc1") || value == FourCc::from_bytes(*b"vvi1") => {
            compatible_brands.push(FourCc::from_bytes(*b"vvc1"));
            compatible_brands.push(FourCc::from_bytes(*b"cmfc"));
        }
        value if value == FourCc::from_bytes(*b"av01") => {
            compatible_brands.push(FourCc::from_bytes(*b"av01"));
            compatible_brands.push(FourCc::from_bytes(*b"cmfc"));
        }
        value if value == FourCc::from_bytes(*b"vp08") => {
            compatible_brands.push(FourCc::from_bytes(*b"vp08"));
            compatible_brands.push(FourCc::from_bytes(*b"cmfc"));
        }
        value if value == FourCc::from_bytes(*b"vp09") => {
            compatible_brands.push(FourCc::from_bytes(*b"vp09"));
            compatible_brands.push(FourCc::from_bytes(*b"cmfc"));
        }
        value if value == FourCc::from_bytes(*b"vp10") => {
            compatible_brands.push(FourCc::from_bytes(*b"vp10"));
            compatible_brands.push(FourCc::from_bytes(*b"cmfc"));
        }
        value if value == FourCc::from_bytes(*b"iamf") => {
            compatible_brands.push(FourCc::from_bytes(*b"cmfc"));
            compatible_brands.push(FourCc::from_bytes(*b"iamf"));
        }
        _ => {
            compatible_brands.push(FourCc::from_bytes(*b"cmfc"));
        }
    }

    encode_typed_box(
        &Ftyp {
            major_brand: FourCc::from_bytes(*b"mp41"),
            minor_version: 0,
            compatible_brands,
        },
        &[],
    )
}

fn build_fragment_layouts(
    file_config: &MuxFileConfig,
    track: &PreparedTrack<'_>,
) -> Result<Vec<FragmentLayout>, MuxError> {
    let mut fragments = Vec::new();
    let mut sample_index = 0_usize;
    for (fragment_index, &samples_per_chunk) in track.chunk_sample_counts.iter().enumerate() {
        let sample_count = usize::try_from(samples_per_chunk)
            .map_err(|_| MuxError::LayoutOverflow("fragment sample count"))?;
        let end_index = sample_index
            .checked_add(sample_count)
            .ok_or(MuxError::LayoutOverflow("fragment sample indexing"))?;
        let fragment_samples = track
            .samples
            .get(sample_index..end_index)
            .ok_or_else(|| MuxError::InvalidChunkPlan {
                track_id: track.config.track_id(),
                message: "fragment boundaries ran past the staged sample count".to_string(),
            })?
            .to_vec();
        let moof_bytes = build_fragment_moof_bytes(
            track,
            &fragment_samples,
            u32::try_from(fragment_index + 1)
                .map_err(|_| MuxError::LayoutOverflow("fragment sequence number"))?,
        )?;
        let payload_size = fragment_samples.iter().try_fold(0_u64, |total, sample| {
            total
                .checked_add(sample.sample_size)
                .ok_or(MuxError::LayoutOverflow("fragment payload size"))
        })?;
        let mdat_header = encode_header_only(FourCc::from_bytes(*b"mdat"), payload_size, "mdat")?;
        let _ = file_config;
        fragments.push(FragmentLayout {
            moof_bytes,
            mdat_header,
            samples: fragment_samples,
        });
        sample_index = end_index;
    }
    if sample_index != track.samples.len() {
        return Err(MuxError::InvalidChunkPlan {
            track_id: track.config.track_id(),
            message: "fragment boundaries did not cover every staged sample".to_string(),
        });
    }
    Ok(fragments)
}

fn build_fragmented_moov_bytes(
    file_config: &MuxFileConfig,
    tracks: &[PreparedTrack<'_>],
) -> Result<Vec<u8>, MuxError> {
    let mvhd = build_fragmented_mvhd(file_config, tracks)?;
    let mut children = vec![encode_typed_box(&mvhd, &[])?, build_meta_bytes()?];
    for track in tracks {
        children.push(build_fragmented_trak_bytes(track)?);
    }
    children.push(build_mvex_bytes(file_config.movie_timescale(), tracks)?);
    encode_typed_box(&Moov, &children.concat())
}

fn build_fragmented_mvhd(
    file_config: &MuxFileConfig,
    tracks: &[PreparedTrack<'_>],
) -> Result<Mvhd, MuxError> {
    let mut mvhd = build_mvhd(file_config, tracks)?;
    mvhd.set_version(0);
    mvhd.creation_time_v0 = u32::try_from(ISOM_UNIX_EPOCH_OFFSET)
        .map_err(|_| MuxError::LayoutOverflow("fragmented mvhd creation_time"))?;
    mvhd.modification_time_v0 = u32::try_from(ISOM_UNIX_EPOCH_OFFSET)
        .map_err(|_| MuxError::LayoutOverflow("fragmented mvhd modification_time"))?;
    mvhd.creation_time_v1 = 0;
    mvhd.modification_time_v1 = 0;
    mvhd.duration_v0 = 0;
    mvhd.duration_v1 = 0;
    Ok(mvhd)
}

fn build_fragmented_trak_bytes(track: &PreparedTrack<'_>) -> Result<Vec<u8>, MuxError> {
    let tkhd = build_fragmented_tkhd(track)?;
    let mdia = build_fragmented_mdia_bytes(track)?;
    let mut children = vec![encode_typed_box(&tkhd, &[])?, mdia];
    if let Some(edts) = build_edts_bytes(track, 0)? {
        children.push(edts);
    }
    encode_typed_box(&Trak, &children.concat())
}

fn build_fragmented_tkhd(track: &PreparedTrack<'_>) -> Result<Tkhd, MuxError> {
    let mut tkhd = build_tkhd_with_movie_timescale(track, track.config.timescale())?;
    tkhd.set_version(0);
    tkhd.creation_time_v0 = u32::try_from(ISOM_UNIX_EPOCH_OFFSET)
        .map_err(|_| MuxError::LayoutOverflow("fragmented tkhd creation_time"))?;
    tkhd.modification_time_v0 = u32::try_from(ISOM_UNIX_EPOCH_OFFSET)
        .map_err(|_| MuxError::LayoutOverflow("fragmented tkhd modification_time"))?;
    tkhd.creation_time_v1 = 0;
    tkhd.modification_time_v1 = 0;
    tkhd.duration_v0 = 0;
    tkhd.duration_v1 = 0;
    Ok(tkhd)
}

fn build_edts_bytes(
    track: &PreparedTrack<'_>,
    segment_duration: u64,
) -> Result<Option<Vec<u8>>, MuxError> {
    let Some(edit_media_time) = track.edit_media_time else {
        return Ok(None);
    };
    let mut elst = Elst::default();
    elst.entry_count = 1;
    if edit_media_time > u64::try_from(i32::MAX).unwrap_or(u64::MAX)
        || segment_duration > u64::from(u32::MAX)
    {
        elst.set_version(1);
        elst.entries.push(ElstEntry {
            segment_duration_v1: segment_duration,
            media_time_v1: i64::try_from(edit_media_time)
                .map_err(|_| MuxError::LayoutOverflow("fragmented edit-list media time"))?,
            media_rate_integer: 1,
            ..ElstEntry::default()
        });
    } else {
        elst.entries.push(ElstEntry {
            segment_duration_v0: u32::try_from(segment_duration)
                .map_err(|_| MuxError::LayoutOverflow("edit-list segment duration"))?,
            media_time_v0: i32::try_from(edit_media_time)
                .map_err(|_| MuxError::LayoutOverflow("fragmented edit-list media time"))?,
            media_rate_integer: 1,
            ..ElstEntry::default()
        });
    }
    Ok(Some(encode_typed_box(
        &Edts,
        &encode_typed_box(&elst, &[])?,
    )?))
}

fn build_fragmented_mdia_bytes(track: &PreparedTrack<'_>) -> Result<Vec<u8>, MuxError> {
    let mdhd = build_fragmented_mdhd(track)?;
    let hdlr = build_hdlr(track);
    let minf = build_fragmented_minf_bytes(track)?;
    let children = [
        encode_typed_box(&mdhd, &[])?,
        encode_typed_box(&hdlr, &[])?,
        minf,
    ]
    .concat();
    encode_typed_box(&Mdia, &children)
}

fn build_fragmented_mdhd(track: &PreparedTrack<'_>) -> Result<Mdhd, MuxError> {
    let mut mdhd = build_mdhd(track)?;
    mdhd.set_version(0);
    mdhd.creation_time_v0 = u32::try_from(ISOM_UNIX_EPOCH_OFFSET)
        .map_err(|_| MuxError::LayoutOverflow("fragmented mdhd creation_time"))?;
    mdhd.modification_time_v0 = u32::try_from(ISOM_UNIX_EPOCH_OFFSET)
        .map_err(|_| MuxError::LayoutOverflow("fragmented mdhd modification_time"))?;
    mdhd.creation_time_v1 = 0;
    mdhd.modification_time_v1 = 0;
    mdhd.duration_v0 = 0;
    mdhd.duration_v1 = 0;
    Ok(mdhd)
}

fn build_fragmented_minf_bytes(track: &PreparedTrack<'_>) -> Result<Vec<u8>, MuxError> {
    let media_header = match track.config.kind() {
        MuxTrackKind::Audio => encode_typed_box(&Smhd::default(), &[])?,
        MuxTrackKind::Video => {
            let mut vmhd = Vmhd::default();
            vmhd.set_flags(VMHD_DEFAULT_FLAGS);
            encode_typed_box(&vmhd, &[])?
        }
        MuxTrackKind::Text => encode_typed_box(&Nmhd::default(), &[])?,
        MuxTrackKind::Subtitle => {
            build_subtitle_media_header_bytes(track.config.sample_entry_box())?
        }
    };
    let dinf = build_dinf_bytes()?;
    let stbl = build_fragmented_stbl_bytes(track)?;
    encode_typed_box(&Minf, &[dinf, stbl, media_header].concat())
}

fn build_fragmented_stbl_bytes(track: &PreparedTrack<'_>) -> Result<Vec<u8>, MuxError> {
    let stsd = build_fragmented_stsd_bytes(track)?;
    let mut stts = Stts::default();
    stts.entry_count = 0;
    let mut stsc = Stsc::default();
    stsc.entry_count = 0;
    let mut stsz = Stsz::default();
    stsz.sample_size = 0;
    stsz.sample_count = 0;
    let mut stco = Stco::default();
    stco.entry_count = 0;
    encode_typed_box(
        &Stbl,
        &[
            stsd,
            encode_typed_box(&stts, &[])?,
            encode_typed_box(&stsc, &[])?,
            encode_typed_box(&stsz, &[])?,
            encode_typed_box(&stco, &[])?,
        ]
        .concat(),
    )
}

fn build_fragmented_stsd_bytes(track: &PreparedTrack<'_>) -> Result<Vec<u8>, MuxError> {
    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    let sample_entry_box = canonicalize_fragmented_sample_entry_box(track.sample_entry_box)?;
    encode_typed_box(&stsd, &sample_entry_box)
}

fn build_meta_bytes() -> Result<Vec<u8>, MuxError> {
    let mut hdlr = Hdlr::default();
    hdlr.handler_type = FourCc::from_bytes(*b"ID32");
    let mut id32 = Id32::default();
    id32.language = "eng".to_string();
    id32.id3v2_data = build_mux_identity_id3_payload(ID3_VERSION);
    encode_typed_box(
        &Meta::default(),
        &[encode_typed_box(&hdlr, &[])?, encode_typed_box(&id32, &[])?].concat(),
    )
}

fn build_mvex_bytes(
    movie_timescale: u32,
    tracks: &[PreparedTrack<'_>],
) -> Result<Vec<u8>, MuxError> {
    let mut fragment_duration = 0_u64;
    for track in tracks {
        fragment_duration =
            fragment_duration.max(fragmented_mehd_duration(movie_timescale, track)?);
    }
    let mut mehd = Mehd::default();
    if fragment_duration > u64::from(u32::MAX) {
        mehd.set_version(1);
        mehd.fragment_duration_v1 = fragment_duration;
    } else {
        mehd.fragment_duration_v0 = u32::try_from(fragment_duration)
            .map_err(|_| MuxError::LayoutOverflow("fragmented mehd duration"))?;
    }
    let mut children = vec![encode_typed_box(&mehd, &[])?];
    for track in tracks {
        let mut trex = Trex::default();
        trex.track_id = track.config.track_id();
        trex.default_sample_description_index = 1;
        trex.default_sample_duration =
            dominant_sample_duration(track.samples.iter().map(|sample| sample.duration_media))
                .unwrap_or(0);
        trex.default_sample_size = 0;
        trex.default_sample_flags = 0;
        children.push(encode_typed_box(&trex, &[])?);
    }
    encode_typed_box(&Mvex, &children.concat())
}

fn build_fragment_moof_bytes(
    track: &PreparedTrack<'_>,
    samples: &[PreparedSample],
    sequence_number: u32,
) -> Result<Vec<u8>, MuxError> {
    let mut mfhd = Mfhd::default();
    mfhd.sequence_number = sequence_number;
    let provisional_traf = build_traf_bytes(track, samples, 0)?;
    let provisional_moof = encode_typed_box(
        &Moof,
        &[encode_typed_box(&mfhd, &[])?, provisional_traf].concat(),
    )?;
    let data_offset = i32::try_from(provisional_moof.len() + 8)
        .map_err(|_| MuxError::LayoutOverflow("fragment data offset"))?;
    let traf = build_traf_bytes(track, samples, data_offset)?;
    encode_typed_box(&Moof, &[encode_typed_box(&mfhd, &[])?, traf].concat())
}

fn build_traf_bytes(
    track: &PreparedTrack<'_>,
    samples: &[PreparedSample],
    data_offset: i32,
) -> Result<Vec<u8>, MuxError> {
    let mut tfhd = Tfhd::default();
    tfhd.track_id = track.config.track_id();
    tfhd.sample_description_index = 1;
    tfhd.set_flags(TFHD_DEFAULT_BASE_IS_MOOF | TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT);

    if let Some(default_duration) =
        all_equal_u32(samples.iter().map(|sample| sample.duration_media))
    {
        tfhd.set_flags(tfhd.flags() | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT);
        tfhd.default_sample_duration = default_duration;
    }
    if let Some(default_size) = all_equal_u32(
        samples
            .iter()
            .map(|sample| u32::try_from(sample.sample_size).unwrap_or(u32::MAX)),
    ) {
        tfhd.set_flags(tfhd.flags() | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT);
        tfhd.default_sample_size = default_size;
    }
    if let Some(default_flags) = all_equal_u32(samples.iter().map(sample_flags)) {
        tfhd.set_flags(tfhd.flags() | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT);
        tfhd.default_sample_flags = default_flags;
    }

    let mut tfdt = Tfdt::default();
    let base_decode_time = samples
        .first()
        .map(|sample| sample.decode_time_media)
        .unwrap_or(0);
    if base_decode_time > u64::from(u32::MAX) {
        tfdt.set_version(1);
        tfdt.base_media_decode_time_v1 = base_decode_time;
    } else {
        tfdt.base_media_decode_time_v0 = u32::try_from(base_decode_time)
            .map_err(|_| MuxError::LayoutOverflow("tfdt decode time"))?;
    }

    let trun = build_trun(samples, data_offset)?;
    encode_typed_box(
        &Traf,
        &[
            encode_typed_box(&tfhd, &[])?,
            encode_typed_box(&tfdt, &[])?,
            encode_typed_box(&trun, &[])?,
        ]
        .concat(),
    )
}

fn build_trun(samples: &[PreparedSample], data_offset: i32) -> Result<Trun, MuxError> {
    let mut trun = Trun::default();
    trun.sample_count =
        u32::try_from(samples.len()).map_err(|_| MuxError::LayoutOverflow("trun sample count"))?;
    trun.data_offset = data_offset;
    trun.set_flags(TRUN_DATA_OFFSET_PRESENT);
    if !samples
        .iter()
        .all(|sample| sample.composition_offset_media == 0)
    {
        trun.set_flags(trun.flags() | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT);
        if samples
            .iter()
            .any(|sample| sample.composition_offset_media < 0)
        {
            trun.set_version(1);
        }
    }
    if all_equal_u32(samples.iter().map(|sample| sample.duration_media)).is_none() {
        trun.set_flags(trun.flags() | TRUN_SAMPLE_DURATION_PRESENT);
    }
    if all_equal_u32(
        samples
            .iter()
            .map(|sample| u32::try_from(sample.sample_size).unwrap_or(u32::MAX)),
    )
    .is_none()
    {
        trun.set_flags(trun.flags() | TRUN_SAMPLE_SIZE_PRESENT);
    }
    if all_equal_u32(samples.iter().map(sample_flags)).is_none() {
        trun.set_flags(trun.flags() | TRUN_SAMPLE_FLAGS_PRESENT);
    }
    trun.entries = samples
        .iter()
        .map(|sample| {
            Ok(TrunEntry {
                sample_duration: sample.duration_media,
                sample_size: u32::try_from(sample.sample_size)
                    .map_err(|_| MuxError::LayoutOverflow("trun sample size"))?,
                sample_flags: sample_flags(sample),
                sample_composition_time_offset_v0: u32::try_from(sample.composition_offset_media)
                    .unwrap_or(0),
                sample_composition_time_offset_v1: sample.composition_offset_media,
            })
        })
        .collect::<Result<Vec<_>, MuxError>>()?;
    Ok(trun)
}

fn build_sidx_bytes(
    file_config: &MuxFileConfig,
    track: &PreparedTrack<'_>,
    fragments: &[FragmentLayout],
    single_sidx_reference: bool,
) -> Result<Vec<u8>, MuxError> {
    let mut sidx = Sidx::default();
    sidx.reference_id = track.config.track_id();
    sidx.timescale = file_config.movie_timescale();
    let earliest_presentation_time = 0_u64;
    if earliest_presentation_time > u64::from(u32::MAX) {
        sidx.set_version(1);
        sidx.earliest_presentation_time_v1 = earliest_presentation_time;
        sidx.first_offset_v1 = 0;
    } else {
        sidx.earliest_presentation_time_v0 = u32::try_from(earliest_presentation_time)
            .map_err(|_| MuxError::LayoutOverflow("sidx earliest presentation time"))?;
        sidx.first_offset_v0 = 0;
    }

    let presentation_trim = if track.config.kind() == MuxTrackKind::Audio {
        track
            .edit_media_time
            .map(|media_time| {
                scale_track_time_to_movie(
                    track.config.track_id(),
                    i64::try_from(media_time)
                        .map_err(|_| MuxError::LayoutOverflow("sidx edit-list trim"))?,
                    track.config.timescale(),
                    file_config.movie_timescale(),
                )
                .and_then(|value| {
                    u64::try_from(value)
                        .map_err(|_| MuxError::LayoutOverflow("sidx edit-list trim"))
                })
            })
            .transpose()?
            .unwrap_or(0)
    } else {
        0
    };
    sidx.references = if single_sidx_reference {
        vec![build_sidx_reference(fragments.iter(), presentation_trim)?]
    } else {
        fragments
            .iter()
            .enumerate()
            .map(|(index, fragment)| {
                build_sidx_reference(
                    std::iter::once(fragment),
                    if index == 0 { presentation_trim } else { 0 },
                )
            })
            .collect::<Result<Vec<_>, MuxError>>()?
    };
    sidx.reference_count = u16::try_from(sidx.references.len())
        .map_err(|_| MuxError::LayoutOverflow("sidx reference count"))?;
    encode_typed_box(&sidx, &[])
}

fn build_sidx_reference<'a, I>(
    fragments: I,
    presentation_trim: u64,
) -> Result<SidxReference, MuxError>
where
    I: IntoIterator<Item = &'a FragmentLayout>,
{
    let mut referenced_size = 0_usize;
    let mut subsegment_duration = 0_u64;
    let mut starts_with_sap = false;
    let mut saw_any_sample = false;

    for fragment in fragments {
        if !saw_any_sample {
            starts_with_sap = fragment
                .samples
                .first()
                .map(|sample| sample.is_sync_sample)
                .unwrap_or(false);
            saw_any_sample = true;
        }
        referenced_size = referenced_size
            .checked_add(fragment.moof_bytes.len())
            .and_then(|size| size.checked_add(fragment.mdat_header.len()))
            .ok_or(MuxError::LayoutOverflow("sidx referenced size"))?;
        for sample in &fragment.samples {
            referenced_size = referenced_size
                .checked_add(
                    usize::try_from(sample.sample_size)
                        .map_err(|_| MuxError::LayoutOverflow("sidx referenced size"))?,
                )
                .ok_or(MuxError::LayoutOverflow("sidx referenced size"))?;
            subsegment_duration = subsegment_duration
                .checked_add(u64::from(sample.duration_movie))
                .ok_or(MuxError::LayoutOverflow("sidx subsegment duration"))?;
        }
    }

    if presentation_trim > subsegment_duration {
        return Err(MuxError::LayoutOverflow("sidx edit-list trim"));
    }
    subsegment_duration -= presentation_trim;

    Ok(SidxReference {
        reference_type: false,
        referenced_size: u32::try_from(referenced_size)
            .map_err(|_| MuxError::LayoutOverflow("sidx referenced size"))?,
        subsegment_duration: u32::try_from(subsegment_duration)
            .map_err(|_| MuxError::LayoutOverflow("sidx subsegment duration"))?,
        starts_with_sap,
        sap_type: if starts_with_sap { 1 } else { 0 },
        sap_delta_time: 0,
    })
}

fn build_ftyp_bytes(
    file_config: &MuxFileConfig,
    tracks: &[PreparedTrack<'_>],
) -> Result<Vec<u8>, MuxError> {
    let (major_brand, minor_version, compatible_brands) = if file_config.auto_flat_profile() {
        infer_auto_flat_ftyp_profile(tracks)
    } else {
        (
            file_config.major_brand(),
            file_config.minor_version(),
            file_config.compatible_brands().to_vec(),
        )
    };
    let ftyp = Ftyp {
        major_brand,
        minor_version,
        compatible_brands,
    };
    encode_typed_box(&ftyp, &[])
}

fn infer_auto_flat_ftyp_profile(tracks: &[PreparedTrack<'_>]) -> (FourCc, u32, Vec<FourCc>) {
    let has_iamf = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"iamf"]));
    let has_qcp = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"sqcp", b"sevc", b"ssmv"]));
    let has_av1 = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"av01"]));
    let has_hevc = tracks.iter().any(|track| {
        sample_entry_matches(
            track.sample_entry_box,
            &[b"hvc1", b"hev1", b"dvh1", b"dvhe"],
        )
    });
    let has_vvc = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"vvc1", b"vvi1"]));
    let has_avc = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"avc1"]));
    let has_h263 = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"s263"]));

    if has_iamf {
        let mut brands = vec![FourCc::from_bytes(*b"isom")];
        if has_avc {
            brands.push(FourCc::from_bytes(*b"avc1"));
        }
        if has_av1 {
            brands.push(FourCc::from_bytes(*b"av01"));
        }
        brands.push(FourCc::from_bytes(*b"mp42"));
        brands.push(FourCc::from_bytes(*b"iso6"));
        brands.push(FourCc::from_bytes(*b"iamf"));
        return (FourCc::from_bytes(*b"mp42"), 1, brands);
    }
    if has_qcp {
        let mut brands = vec![FourCc::from_bytes(*b"isom")];
        if has_avc {
            brands.push(FourCc::from_bytes(*b"avc1"));
        }
        brands.push(FourCc::from_bytes(*b"3g2a"));
        return (FourCc::from_bytes(*b"3g2a"), 65_536, brands);
    }
    if has_av1 {
        return (
            FourCc::from_bytes(*b"iso4"),
            1,
            vec![FourCc::from_bytes(*b"iso4"), FourCc::from_bytes(*b"av01")],
        );
    }
    if has_hevc {
        return (
            FourCc::from_bytes(*b"iso4"),
            1,
            vec![FourCc::from_bytes(*b"iso4")],
        );
    }
    if has_vvc {
        return (
            FourCc::from_bytes(*b"iso4"),
            1,
            vec![FourCc::from_bytes(*b"iso4")],
        );
    }
    if has_h263 {
        return (
            FourCc::from_bytes(*b"isom"),
            1,
            vec![
                FourCc::from_bytes(*b"isom"),
                FourCc::from_bytes(*b"3gg6"),
                FourCc::from_bytes(*b"3gg5"),
            ],
        );
    }
    if has_avc {
        return (
            FourCc::from_bytes(*b"isom"),
            1,
            vec![FourCc::from_bytes(*b"isom"), FourCc::from_bytes(*b"avc1")],
        );
    }
    (
        FourCc::from_bytes(*b"isom"),
        1,
        vec![FourCc::from_bytes(*b"isom")],
    )
}

fn sample_entry_matches(sample_entry_box: &[u8], names: &[&[u8; 4]]) -> bool {
    sample_entry_box
        .get(4..8)
        .map(|box_type| {
            names
                .iter()
                .any(|candidate| box_type == candidate.as_slice())
        })
        .unwrap_or(false)
}

fn sample_entry_esds_oti_matches(
    sample_entry_box: &[u8],
    sample_entry_types: &[&[u8; 4]],
    object_type_indication: u8,
) -> Result<bool, MuxError> {
    if !sample_entry_matches(sample_entry_box, sample_entry_types) {
        return Ok(false);
    }
    let sample_entry_type = sample_entry_box_type(sample_entry_box)?;
    let child_boxes = match sample_entry_type {
        value if value == FourCc::from_bytes(*b"mp4a") => {
            decode_audio_sample_entry_parts(sample_entry_box)?.1
        }
        value if value == FourCc::from_bytes(*b"mp4v") => {
            decode_visual_sample_entry_parts(sample_entry_box)?.1
        }
        _ => return Ok(false),
    };
    for child_box in child_boxes {
        if sample_entry_box_type(&child_box)? != FourCc::from_bytes(*b"esds") {
            continue;
        }
        let esds = decode_typed_box::<Esds>(&child_box)?;
        for descriptor in esds.descriptors {
            if let Some(decoder_config) = descriptor.decoder_config_descriptor
                && decoder_config.object_type_indication == object_type_indication
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn mp4a_sample_entry_oti_matches(
    sample_entry_box: &[u8],
    object_type_indication: u8,
) -> Result<bool, MuxError> {
    sample_entry_esds_oti_matches(sample_entry_box, &[b"mp4a"], object_type_indication)
}

fn prepare_tracks<'a>(
    file_config: &MuxFileConfig,
    track_configs: &'a [MuxTrackConfig],
    plan: &'a MuxPlan,
) -> Result<Vec<PreparedTrack<'a>>, MuxError> {
    let mut config_by_track_id = BTreeMap::<u32, &'a MuxTrackConfig>::new();
    for config in track_configs {
        if config.timescale() == 0 {
            return Err(MuxError::InvalidTrackTimescale {
                track_id: config.track_id(),
            });
        }
        validate_language(config)?;
        validate_sample_entry_box(config)?;
        match config_by_track_id.entry(config.track_id()) {
            Entry::Vacant(slot) => {
                slot.insert(config);
            }
            Entry::Occupied(_) => {
                return Err(MuxError::DuplicateTrackId {
                    track_id: config.track_id(),
                });
            }
        }
    }

    let mut samples_by_track = BTreeMap::<u32, Vec<&super::MuxPlannedMediaItem>>::new();
    for item in plan.planned_items() {
        samples_by_track
            .entry(item.staged().track_id())
            .or_default()
            .push(item);
    }

    for track_id in samples_by_track.keys().copied() {
        if !config_by_track_id.contains_key(&track_id) {
            return Err(MuxError::MissingTrackId { track_id });
        }
    }

    let mut prepared_tracks = Vec::with_capacity(track_configs.len());
    for config in track_configs {
        let samples = samples_by_track
            .remove(&config.track_id())
            .unwrap_or_default();
        prepared_tracks.push(prepare_track(file_config, plan, config, samples)?);
    }

    if file_config.auto_flat_profile()
        && prepared_tracks.len() == 1
        && prepared_tracks[0].config.kind().is_video()
    {
        let track = &mut prepared_tracks[0];
        track.chunk_sample_counts = if track.samples.is_empty() {
            Vec::new()
        } else {
            vec![
                u32::try_from(track.samples.len())
                    .map_err(|_| MuxError::LayoutOverflow("single-track chunk collapse"))?,
            ]
        };
    }

    Ok(prepared_tracks)
}

fn prepare_track<'a>(
    file_config: &MuxFileConfig,
    plan: &'a MuxPlan,
    config: &'a MuxTrackConfig,
    samples: Vec<&'a super::MuxPlannedMediaItem>,
) -> Result<PreparedTrack<'a>, MuxError> {
    let mut previous_decode_time = None::<u64>;
    let mut prepared_samples = Vec::with_capacity(samples.len());
    let mut max_decode_end_media = 0_u64;
    let mut max_presentation_end_media = 0_u64;

    for sample in samples {
        let staged = sample.staged();
        if let Some(previous_decode_time) = previous_decode_time
            && staged.decode_time() < previous_decode_time
        {
            return Err(MuxError::NonMonotonicTrackDecodeTime {
                track_id: config.track_id(),
                previous_decode_time,
                next_decode_time: staged.decode_time(),
            });
        }
        previous_decode_time = Some(staged.decode_time());

        let duration_media = scale_movie_time_to_track(
            config.track_id(),
            u64::from(staged.duration()),
            file_config.movie_timescale(),
            config.timescale(),
        )?;
        let composition_offset_media = scale_movie_offset_to_track(
            config.track_id(),
            i64::from(staged.composition_time_offset()),
            file_config.movie_timescale(),
            config.timescale(),
        )?;
        let decode_time_media = scale_movie_time_to_track(
            config.track_id(),
            staged.decode_time(),
            file_config.movie_timescale(),
            config.timescale(),
        )?;
        let decode_end_movie = staged
            .decode_time()
            .checked_add(u64::from(staged.duration()))
            .ok_or(MuxError::LayoutOverflow("track decode end"))?;
        let decode_end_media = scale_movie_time_to_track(
            config.track_id(),
            decode_end_movie,
            file_config.movie_timescale(),
            config.timescale(),
        )?;
        max_decode_end_media = max_decode_end_media.max(decode_end_media);
        let presentation_end_media = i128::from(decode_time_media)
            .saturating_add(i128::from(composition_offset_media))
            .saturating_add(i128::from(duration_media));
        if presentation_end_media > 0 {
            max_presentation_end_media = max_presentation_end_media.max(
                u64::try_from(presentation_end_media)
                    .map_err(|_| MuxError::LayoutOverflow("presentation end time"))?,
            );
        }
        prepared_samples.push(PreparedSample {
            source_index: staged.source_index(),
            source_data_offset: staged.data_offset(),
            decode_time_media,
            output_offset: sample.output_offset(),
            sample_size: u64::from(staged.data_size()),
            duration_movie: staged.duration(),
            duration_media: u32::try_from(duration_media)
                .map_err(|_| MuxError::LayoutOverflow("sample duration"))?,
            composition_offset_media,
            is_sync_sample: staged.is_sync_sample(),
        });
    }

    let media_duration = max_decode_end_media.max(max_presentation_end_media);
    let presentation_duration_media = config
        .edit_media_time()
        .map_or(media_duration, |edit_media_time| {
            media_duration.saturating_sub(edit_media_time)
        });
    let flat_timing_override = if file_config.auto_flat_profile() {
        if let Some(override_value) = config.flat_timing_override() {
            if override_value.sample_durations.len() != prepared_samples.len() {
                return Err(MuxError::InvalidOutputLayout {
                    layout: "flat",
                    message: format!(
                        "track {} authored a flat timing override with {} sample durations for {} samples",
                        config.track_id(),
                        override_value.sample_durations.len(),
                        prepared_samples.len(),
                    ),
                });
            }
            Some(override_value)
        } else {
            None
        }
    } else {
        None
    };
    Ok(PreparedTrack {
        config,
        sample_entry_box: config.sample_entry_box(),
        samples: prepared_samples,
        chunk_sample_counts: if previous_decode_time.is_some() {
            plan.chunk_sample_counts(config.track_id())?.to_vec()
        } else {
            Vec::new()
        },
        media_duration,
        presentation_duration_media,
        edit_media_time: config.edit_media_time(),
        flat_timing_override,
    })
}

fn build_moov_bytes(
    file_config: &MuxFileConfig,
    tracks: &[PreparedTrack<'_>],
    ftyp_size: u64,
    mdat_header_size: u64,
    mdat_data_start: u64,
) -> Result<Vec<u8>, MuxError> {
    let mvhd = build_mvhd(file_config, tracks)?;
    let mut children = Vec::new();
    children.extend_from_slice(&encode_typed_box(&mvhd, &[])?);
    if let Some(iods_bytes) = build_flat_iods_bytes(file_config, tracks)? {
        children.extend_from_slice(&iods_bytes);
    }
    for track in tracks {
        children.extend_from_slice(&build_trak_bytes(
            file_config,
            track,
            ftyp_size,
            mdat_header_size,
            mdat_data_start,
        )?);
    }
    if let Some(udta_bytes) = build_flat_udta_bytes(file_config)? {
        children.extend_from_slice(&udta_bytes);
    }
    encode_typed_box(&Moov, &children)
}

fn build_flat_iods_bytes(
    file_config: &MuxFileConfig,
    tracks: &[PreparedTrack<'_>],
) -> Result<Option<Vec<u8>>, MuxError> {
    if !file_config.auto_flat_profile() {
        return Ok(None);
    }

    let has_audio = tracks.iter().any(|track| track.config.kind().is_audio());
    let has_mp4a = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"mp4a"]));
    let has_vorbis_mp4a = tracks
        .iter()
        .any(|track| mp4a_sample_entry_oti_matches(track.sample_entry_box, 0xDD).unwrap_or(false));
    let has_opus = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"Opus"]));
    let has_speex = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"spex"]));
    let has_voice_3gpp_audio = tracks.iter().any(|track| {
        sample_entry_matches(
            track.sample_entry_box,
            &[b"samr", b"sawb", b"sqcp", b"sevc", b"ssmv"],
        )
    });
    let has_visual_track = tracks.iter().any(|track| track.config.kind().is_video());
    let has_mhm1 = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"mhm1"]));
    let has_mp4s = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"mp4s"]));
    let has_mp4v = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"mp4v"]));
    let has_theora_mp4v = tracks.iter().any(|track| {
        sample_entry_esds_oti_matches(track.sample_entry_box, &[b"mp4v"], 0xDF).unwrap_or(false)
    });
    let has_other_iods_codec = tracks.iter().any(|track| {
        sample_entry_matches(
            track.sample_entry_box,
            &[b"mp4v", b"mp4s", b"Opus", b"spex", b"mhm1"],
        )
    });
    let has_non_mp4a_audio = has_audio
        && tracks.iter().any(|track| {
            track.config.kind().is_audio()
                && !sample_entry_matches(track.sample_entry_box, &[b"mp4a"])
        });
    let has_avc = tracks
        .iter()
        .any(|track| sample_entry_matches(track.sample_entry_box, &[b"avc1"]));
    if !has_mp4a && !has_avc && !has_other_iods_codec && !has_mp4s {
        return Ok(None);
    }

    let descriptor = Descriptor::from_initial_object_descriptor(InitialObjectDescriptor {
        object_descriptor_id: 1,
        include_inline_profile_level_flag: false,
        od_profile_level_indication: 0xff,
        scene_profile_level_indication: 0xff,
        audio_profile_level_indication: if has_mhm1 && has_avc {
            0xfe
        } else if has_mhm1 {
            0x0c
        } else if has_vorbis_mp4a {
            0x10
        } else if has_mp4a {
            0x29
        } else if (has_voice_3gpp_audio && has_visual_track)
            || has_opus
            || (has_speex && !has_visual_track)
        {
            0xfe
        } else {
            0xff
        },
        visual_profile_level_indication: if has_avc && has_mp4a {
            0x7f
        } else if has_avc && has_non_mp4a_audio {
            0x15
        } else if has_avc {
            0x7f
        } else if has_theora_mp4v {
            0xfe
        } else if has_mp4v {
            0x01
        } else {
            0xff
        },
        graphics_profile_level_indication: 0xff,
        ..InitialObjectDescriptor::default()
    })
    .map_err(|error| MuxError::InvalidOutputLayout {
        layout: "flat",
        message: format!("failed to build iods descriptor: {error}"),
    })?;

    let mut iods = Iods::default();
    iods.descriptor = Some(descriptor);
    Ok(Some(encode_typed_box(&iods, &[])?))
}

fn build_flat_udta_bytes(file_config: &MuxFileConfig) -> Result<Option<Vec<u8>>, MuxError> {
    if !file_config.auto_flat_profile() {
        return Ok(None);
    }

    let mut hdlr = Hdlr::default();
    hdlr.handler_type = FourCc::from_bytes(*b"mdir");
    hdlr.name.clear();

    let mut tool_item = IlstMetaContainer::default();
    tool_item.set_box_type(TOOL_METADATA_ITEM_TYPE);
    let tool_data = Data {
        data_type: DATA_TYPE_STRING_UTF8,
        data_lang: 0,
        data: ID3_VERSION.as_bytes().to_vec(),
    };
    let tool_item_bytes = encode_typed_box(&tool_item, &encode_typed_box(&tool_data, &[])?)?;
    let ilst_bytes = encode_typed_box(&Ilst, &tool_item_bytes)?;
    let meta_bytes = encode_typed_box(
        &Meta::default(),
        &[encode_typed_box(&hdlr, &[])?, ilst_bytes].concat(),
    )?;
    Ok(Some(encode_typed_box(&Udta, &meta_bytes)?))
}

fn build_free_padding_bytes() -> Result<Vec<u8>, MuxError> {
    encode_raw_box(
        FourCc::from_bytes(*b"free"),
        &[0_u8; DEFAULT_FREE_PADDING_SIZE],
    )
}

fn build_mvhd(file_config: &MuxFileConfig, tracks: &[PreparedTrack<'_>]) -> Result<Mvhd, MuxError> {
    let movie_timescale = flat_movie_header_timescale(file_config);
    let movie_duration = tracks
        .iter()
        .map(|track| flat_movie_duration(track, movie_timescale))
        .max()
        .unwrap_or(0);
    let next_track_id = tracks
        .iter()
        .map(|track| track.config.track_id())
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(MuxError::LayoutOverflow("next_track_id"))?;

    let mut mvhd = Mvhd::default();
    mvhd.timescale = movie_timescale;
    if movie_duration > u64::from(u32::MAX) {
        mvhd.set_version(1);
        mvhd.duration_v1 = movie_duration;
    } else {
        mvhd.duration_v0 =
            u32::try_from(movie_duration).map_err(|_| MuxError::LayoutOverflow("mvhd duration"))?;
    }
    mvhd.rate = 0x0001_0000;
    mvhd.volume = 0x0100;
    mvhd.matrix = IDENTITY_MATRIX;
    mvhd.next_track_id = next_track_id;
    Ok(mvhd)
}

fn flat_movie_header_timescale(file_config: &MuxFileConfig) -> u32 {
    if file_config.auto_flat_profile() {
        AUTO_FLAT_MOVIE_TIMESCALE
    } else {
        file_config.movie_timescale()
    }
}

fn flat_movie_duration(track: &PreparedTrack<'_>, movie_timescale: u32) -> u64 {
    let presentation_duration_media = track
        .flat_timing_override
        .map(|override_value| override_value.presentation_duration)
        .unwrap_or(track.presentation_duration_media);
    if movie_timescale == track.config.timescale() {
        return presentation_duration_media;
    }
    presentation_duration_media.saturating_mul(u64::from(movie_timescale))
        / u64::from(track.config.timescale())
}

fn build_trak_bytes(
    file_config: &MuxFileConfig,
    track: &PreparedTrack<'_>,
    ftyp_size: u64,
    mdat_header_size: u64,
    mdat_data_start: u64,
) -> Result<Vec<u8>, MuxError> {
    let tkhd = build_tkhd(file_config, track)?;
    let mdia = build_mdia_bytes(
        file_config,
        track,
        ftyp_size,
        mdat_header_size,
        mdat_data_start,
    )?;
    let mut children = vec![encode_typed_box(&tkhd, &[])?];
    if let Some(edts) = build_edts_bytes(
        track,
        flat_movie_duration(track, flat_movie_header_timescale(file_config)),
    )? {
        children.push(edts);
    }
    children.push(mdia);
    encode_typed_box(&Trak, &children.concat())
}

fn build_tkhd(file_config: &MuxFileConfig, track: &PreparedTrack<'_>) -> Result<Tkhd, MuxError> {
    build_tkhd_with_movie_timescale(track, flat_movie_header_timescale(file_config))
}

fn build_tkhd_with_movie_timescale(
    track: &PreparedTrack<'_>,
    movie_timescale: u32,
) -> Result<Tkhd, MuxError> {
    let mut tkhd = Tkhd::default();
    tkhd.set_flags(
        TKHD_FLAGS_TRACK_ENABLED | TKHD_FLAGS_TRACK_IN_MOVIE | TKHD_FLAGS_TRACK_IN_PREVIEW,
    );
    tkhd.track_id = track.config.track_id();
    let movie_duration = flat_movie_duration(track, movie_timescale);
    if movie_duration > u64::from(u32::MAX) {
        tkhd.set_version(1);
        tkhd.duration_v1 = movie_duration;
    } else {
        tkhd.duration_v0 =
            u32::try_from(movie_duration).map_err(|_| MuxError::LayoutOverflow("tkhd duration"))?;
    }
    tkhd.layer = 0;
    tkhd.alternate_group = 1;
    tkhd.volume = track.config.volume();
    tkhd.matrix = IDENTITY_MATRIX;
    tkhd.width = u32::from(track.config.track_width()) << 16;
    tkhd.height = u32::from(track.config.track_height()) << 16;
    Ok(tkhd)
}

fn build_mdia_bytes(
    file_config: &MuxFileConfig,
    track: &PreparedTrack<'_>,
    ftyp_size: u64,
    mdat_header_size: u64,
    mdat_data_start: u64,
) -> Result<Vec<u8>, MuxError> {
    let mdhd = build_mdhd(track)?;
    let hdlr = build_hdlr(track);
    let minf = build_minf_bytes(
        file_config,
        track,
        ftyp_size,
        mdat_header_size,
        mdat_data_start,
    )?;
    let children = [
        encode_typed_box(&mdhd, &[])?,
        encode_typed_box(&hdlr, &[])?,
        minf,
    ]
    .concat();
    encode_typed_box(&Mdia, &children)
}

fn build_mdhd(track: &PreparedTrack<'_>) -> Result<Mdhd, MuxError> {
    let mut mdhd = Mdhd::default();
    mdhd.timescale = track.config.timescale();
    let media_duration = track
        .flat_timing_override
        .map(|override_value| override_value.media_duration)
        .unwrap_or(track.media_duration);
    if media_duration > u64::from(u32::MAX) {
        mdhd.set_version(1);
        mdhd.duration_v1 = media_duration;
    } else {
        mdhd.duration_v0 =
            u32::try_from(media_duration).map_err(|_| MuxError::LayoutOverflow("mdhd duration"))?;
    }
    mdhd.language = encode_iso639_2_language(track.config)?;
    Ok(mdhd)
}

fn build_hdlr(track: &PreparedTrack<'_>) -> Hdlr {
    let mut hdlr = Hdlr::default();
    hdlr.handler_type = match track.config.kind() {
        MuxTrackKind::Audio => FourCc::from_bytes(*b"soun"),
        MuxTrackKind::Video => FourCc::from_bytes(*b"vide"),
        MuxTrackKind::Text => FourCc::from_bytes(*b"text"),
        MuxTrackKind::Subtitle => subtitle_handler_type(track.config.sample_entry_box()),
    };
    hdlr.name = track.config.handler_name().to_string();
    hdlr
}

fn subtitle_handler_type(sample_entry_box: &[u8]) -> FourCc {
    if sample_entry_box.len() >= 8
        && FourCc::from_bytes([
            sample_entry_box[4],
            sample_entry_box[5],
            sample_entry_box[6],
            sample_entry_box[7],
        ]) == FourCc::from_bytes(*b"mp4s")
    {
        FourCc::from_bytes(*b"subp")
    } else {
        FourCc::from_bytes(*b"subt")
    }
}

fn fragmented_mehd_duration(
    movie_timescale: u32,
    track: &PreparedTrack<'_>,
) -> Result<u64, MuxError> {
    let media_duration = if track.config.kind() == MuxTrackKind::Audio {
        track.media_duration
    } else {
        track.presentation_duration_media
    };
    scale_track_time_to_movie(
        track.config.track_id(),
        i64::try_from(media_duration)
            .map_err(|_| MuxError::LayoutOverflow("fragmented mehd duration"))?,
        track.config.timescale(),
        movie_timescale,
    )
    .and_then(|value| {
        u64::try_from(value).map_err(|_| MuxError::LayoutOverflow("fragmented mehd duration"))
    })
}

fn build_minf_bytes(
    file_config: &MuxFileConfig,
    track: &PreparedTrack<'_>,
    ftyp_size: u64,
    mdat_header_size: u64,
    mdat_data_start: u64,
) -> Result<Vec<u8>, MuxError> {
    let media_header = match track.config.kind() {
        MuxTrackKind::Audio => {
            let smhd = Smhd::default();
            encode_typed_box(&smhd, &[])?
        }
        MuxTrackKind::Video => {
            let mut vmhd = Vmhd::default();
            vmhd.set_flags(VMHD_DEFAULT_FLAGS);
            encode_typed_box(&vmhd, &[])?
        }
        MuxTrackKind::Text => {
            let nmhd = Nmhd::default();
            encode_typed_box(&nmhd, &[])?
        }
        MuxTrackKind::Subtitle => {
            build_subtitle_media_header_bytes(track.config.sample_entry_box())?
        }
    };
    let dinf = build_dinf_bytes()?;
    let stbl = build_stbl_bytes(
        file_config,
        track,
        ftyp_size,
        mdat_header_size,
        mdat_data_start,
    )?;
    encode_typed_box(&Minf, &[media_header, dinf, stbl].concat())
}

fn build_dinf_bytes() -> Result<Vec<u8>, MuxError> {
    let mut url = Url::default();
    url.set_flags(0x0000_0001);
    let mut dref = Dref::default();
    dref.entry_count = 1;
    let dref_children = encode_typed_box(&url, &[])?;
    let dref_bytes = encode_typed_box(&dref, &dref_children)?;
    encode_typed_box(&Dinf, &dref_bytes)
}

fn build_subtitle_media_header_bytes(sample_entry_box: &[u8]) -> Result<Vec<u8>, MuxError> {
    if sample_entry_matches(sample_entry_box, &[b"mp4s"]) {
        return encode_typed_box(&Nmhd::default(), &[]);
    }
    encode_typed_box(&Sthd::default(), &[])
}

fn build_stbl_bytes(
    _file_config: &MuxFileConfig,
    track: &PreparedTrack<'_>,
    _ftyp_size: u64,
    _mdat_header_size: u64,
    mdat_data_start: u64,
) -> Result<Vec<u8>, MuxError> {
    let stsd = build_stsd_bytes(track)?;
    let stts = build_stts(track)?;
    let stsc = build_stsc(track)?;
    let stsz = build_stsz(track)?;
    let chunk_offsets = build_chunk_offsets(track, mdat_data_start)?;
    let mut children = vec![stsd, encode_typed_box(&stts, &[])?];
    if let Some(ctts) = build_ctts(track)? {
        children.push(encode_typed_box(&ctts, &[])?);
    }
    if let Some(stss) = build_stss(track)? {
        children.push(encode_typed_box(&stss, &[])?);
    }
    children.push(encode_typed_box(&stsc, &[])?);
    children.push(encode_typed_box(&stsz, &[])?);
    if chunk_offsets
        .iter()
        .all(|offset| *offset <= u64::from(u32::MAX))
    {
        children.push(encode_typed_box(&build_stco(&chunk_offsets)?, &[])?);
    } else {
        children.push(encode_typed_box(&build_co64(&chunk_offsets)?, &[])?);
    }
    if let Some(sample_roll_distance) = track.config.sample_roll_distance() {
        children.push(encode_typed_box(
            &build_roll_sgpd(sample_roll_distance),
            &[],
        )?);
        children.push(encode_typed_box(
            &build_roll_sbgp(
                u32::try_from(track.samples.len())
                    .map_err(|_| MuxError::LayoutOverflow("roll sample count"))?,
            ),
            &[],
        )?);
    }

    encode_typed_box(&Stbl, &children.concat())
}

fn build_stsd_bytes(track: &PreparedTrack<'_>) -> Result<Vec<u8>, MuxError> {
    let mut stsd = Stsd::default();
    stsd.entry_count = 1;
    encode_typed_box(&stsd, track.sample_entry_box)
}

fn build_stts(track: &PreparedTrack<'_>) -> Result<Stts, MuxError> {
    let entries = if let Some(override_value) = track.flat_timing_override {
        run_length_encode_u32(override_value.sample_durations.iter().copied())
    } else {
        run_length_encode_u32(track.samples.iter().map(|sample| sample.duration_media))
    };
    let mut stts = Stts::default();
    stts.entry_count =
        u32::try_from(entries.len()).map_err(|_| MuxError::LayoutOverflow("stts entry_count"))?;
    stts.entries = entries
        .into_iter()
        .map(|(sample_count, sample_delta)| SttsEntry {
            sample_count,
            sample_delta,
        })
        .collect();
    Ok(stts)
}

fn build_ctts(track: &PreparedTrack<'_>) -> Result<Option<Ctts>, MuxError> {
    if track
        .samples
        .iter()
        .all(|sample| sample.composition_offset_media == 0)
    {
        return Ok(None);
    }

    let use_version_one = track
        .samples
        .iter()
        .any(|sample| sample.composition_offset_media < 0);
    let runs = run_length_encode_i32(
        track
            .samples
            .iter()
            .map(|sample| sample.composition_offset_media),
    );
    let mut ctts = Ctts::default();
    if use_version_one {
        ctts.set_version(1);
    }
    ctts.entry_count =
        u32::try_from(runs.len()).map_err(|_| MuxError::LayoutOverflow("ctts entry_count"))?;
    ctts.entries = runs
        .into_iter()
        .map(|(sample_count, sample_offset)| CttsEntry {
            sample_count,
            sample_offset_v0: u32::try_from(sample_offset).unwrap_or(0),
            sample_offset_v1: sample_offset,
        })
        .collect();
    Ok(Some(ctts))
}

fn build_stsc(track: &PreparedTrack<'_>) -> Result<Stsc, MuxError> {
    let encoded_runs = build_stsc_runs(track)?;
    let mut stsc = Stsc::default();
    stsc.entry_count = u32::try_from(encoded_runs.len())
        .map_err(|_| MuxError::LayoutOverflow("stsc entry_count"))?;
    let mut first_chunk = 1_u32;
    stsc.entries = Vec::with_capacity(encoded_runs.len());
    for (chunk_run_length, samples_per_chunk) in encoded_runs {
        stsc.entries.push(StscEntry {
            first_chunk,
            samples_per_chunk,
            sample_description_index: 1,
        });
        first_chunk = first_chunk
            .checked_add(chunk_run_length)
            .ok_or(MuxError::LayoutOverflow("stsc first_chunk"))?;
    }
    Ok(stsc)
}

fn build_stsc_runs(track: &PreparedTrack<'_>) -> Result<Vec<(u32, u32)>, MuxError> {
    let mut encoded_runs = run_length_encode_u32(track.chunk_sample_counts.iter().copied());
    if track.config.stsc_run_encoding_mode() == super::StscRunEncodingMode::PreserveTerminalBoundary
        && track.chunk_sample_counts.len() > 1
        && let Some((run_length, samples_per_chunk)) = encoded_runs.last().copied()
        && run_length > 1
    {
        encoded_runs.pop();
        encoded_runs.push((run_length - 1, samples_per_chunk));
        encoded_runs.push((1, samples_per_chunk));
    }
    Ok(encoded_runs)
}

fn build_stsz(track: &PreparedTrack<'_>) -> Result<Stsz, MuxError> {
    let mut stsz = Stsz::default();
    stsz.sample_count =
        u32::try_from(track.samples.len()).map_err(|_| MuxError::LayoutOverflow("sample_count"))?;
    if let Some(sample_size) = all_equal_u64(track.samples.iter().map(|sample| sample.sample_size))
    {
        stsz.sample_size =
            u32::try_from(sample_size).map_err(|_| MuxError::LayoutOverflow("sample size"))?;
    } else {
        stsz.sample_size = 0;
        stsz.entry_size = track
            .samples
            .iter()
            .map(|sample| sample.sample_size)
            .collect();
    }
    Ok(stsz)
}

fn build_chunk_offsets(
    track: &PreparedTrack<'_>,
    mdat_data_start: u64,
) -> Result<Vec<u64>, MuxError> {
    let mut chunk_offsets = Vec::with_capacity(track.chunk_sample_counts.len());
    let mut sample_index = 0_usize;
    for &samples_per_chunk in &track.chunk_sample_counts {
        let sample = track
            .samples
            .get(sample_index)
            .ok_or_else(|| MuxError::InvalidChunkPlan {
                track_id: track.config.track_id(),
                message: "chunk boundaries ran past the staged sample count".to_string(),
            })?;
        chunk_offsets.push(
            mdat_data_start
                .checked_add(sample.output_offset)
                .ok_or(MuxError::LayoutOverflow("chunk offset"))?,
        );
        sample_index = sample_index
            .checked_add(
                usize::try_from(samples_per_chunk)
                    .map_err(|_| MuxError::LayoutOverflow("chunk sample-count conversion"))?,
            )
            .ok_or(MuxError::LayoutOverflow("chunk sample indexing"))?;
    }
    Ok(chunk_offsets)
}

fn build_stco(chunk_offsets: &[u64]) -> Result<Stco, MuxError> {
    let mut stco = Stco::default();
    stco.entry_count =
        u32::try_from(chunk_offsets.len()).map_err(|_| MuxError::LayoutOverflow("chunk_count"))?;
    for &offset in chunk_offsets {
        let _ = u32::try_from(offset).map_err(|_| MuxError::LayoutOverflow("chunk offset"))?;
    }
    stco.chunk_offset = chunk_offsets.to_vec();
    Ok(stco)
}

fn build_co64(chunk_offsets: &[u64]) -> Result<Co64, MuxError> {
    let mut co64 = Co64::default();
    co64.entry_count =
        u32::try_from(chunk_offsets.len()).map_err(|_| MuxError::LayoutOverflow("chunk_count"))?;
    co64.chunk_offset = chunk_offsets.to_vec();
    Ok(co64)
}

fn build_stss(track: &PreparedTrack<'_>) -> Result<Option<Stss>, MuxError> {
    if matches!(
        track.config.sync_sample_table_mode,
        super::SyncSampleTableMode::ForceEmpty
    ) {
        return Ok(Some(Stss::default()));
    }

    if track.samples.iter().all(|sample| sample.is_sync_sample)
        && !matches!(
            track.config.sync_sample_table_mode,
            super::SyncSampleTableMode::ForceAll
        )
    {
        return Ok(None);
    }

    let mut stss = Stss::default();
    stss.sample_number = track
        .samples
        .iter()
        .enumerate()
        .filter_map(|(index, sample)| {
            sample
                .is_sync_sample
                .then_some(u64::try_from(index + 1).ok())
                .flatten()
        })
        .collect();
    stss.entry_count = u32::try_from(stss.sample_number.len())
        .map_err(|_| MuxError::LayoutOverflow("stss entry_count"))?;
    Ok(Some(stss))
}

fn build_roll_sgpd(sample_roll_distance: i16) -> Sgpd {
    let mut sgpd = Sgpd::default();
    sgpd.set_version(1);
    sgpd.grouping_type = FourCc::from_bytes(*b"roll");
    sgpd.default_length = 2;
    sgpd.entry_count = 1;
    sgpd.roll_distances = vec![sample_roll_distance];
    sgpd
}

fn build_roll_sbgp(sample_count: u32) -> Sbgp {
    let mut sbgp = Sbgp::default();
    sbgp.grouping_type = u32::from_be_bytes(*b"roll");
    sbgp.entry_count = 1;
    sbgp.entries = vec![SbgpEntry {
        sample_count,
        group_description_index: 1,
    }];
    sbgp
}

pub(super) fn encode_typed_box<B>(box_value: &B, children: &[u8]) -> Result<Vec<u8>, MuxError>
where
    B: CodecBox,
{
    let mut payload = Vec::new();
    marshal(&mut payload, box_value, None)?;
    payload.extend_from_slice(children);
    encode_raw_box(box_value.box_type(), &payload)
}

pub(super) fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Result<Vec<u8>, MuxError> {
    let mut cursor = Cursor::new(Vec::new());
    let payload_size =
        u64::try_from(payload.len()).map_err(|_| MuxError::LayoutOverflow("box payload"))?;
    let header = BoxInfo::new(box_type, BoxInfo::new(box_type, 8).size() + payload_size);
    let written = header.write(&mut cursor)?;
    if written.payload_size()? != payload_size {
        return Err(MuxError::LayoutOverflow("box header normalization"));
    }
    cursor.get_mut().extend_from_slice(payload);
    Ok(cursor.into_inner())
}

fn encode_header_only(
    box_type: FourCc,
    payload_size: u64,
    field_name: &'static str,
) -> Result<Vec<u8>, MuxError> {
    let mut cursor = Cursor::new(Vec::new());
    let header = BoxInfo::new(
        box_type,
        BoxInfo::new(box_type, 8)
            .size()
            .checked_add(payload_size)
            .ok_or(MuxError::LayoutOverflow(field_name))?,
    );
    header.write(&mut cursor)?;
    Ok(cursor.into_inner())
}

fn validate_sample_entry_box(config: &MuxTrackConfig) -> Result<(), MuxError> {
    let mut cursor = Cursor::new(config.sample_entry_box());
    let info = BoxInfo::read(&mut cursor).map_err(|error| MuxError::InvalidSampleEntryBox {
        track_id: config.track_id(),
        message: error.to_string(),
    })?;
    let end = usize::try_from(info.size()).map_err(|_| MuxError::InvalidSampleEntryBox {
        track_id: config.track_id(),
        message: "box size is too large".to_string(),
    })?;
    if info.extend_to_eof() || end != config.sample_entry_box().len() {
        return Err(MuxError::InvalidSampleEntryBox {
            track_id: config.track_id(),
            message: "expected exactly one complete encoded sample-entry box".to_string(),
        });
    }
    Ok(())
}

fn validate_language(config: &MuxTrackConfig) -> Result<(), MuxError> {
    let language = config.language();
    if language.iter().all(|byte| byte.is_ascii_lowercase()) {
        return Ok(());
    }
    Err(MuxError::InvalidTrackLanguage {
        track_id: config.track_id(),
        language: String::from_utf8_lossy(&language).into_owned(),
    })
}

fn encode_iso639_2_language(config: &MuxTrackConfig) -> Result<[u8; 3], MuxError> {
    let language = config.language();
    if !language.iter().all(|byte| byte.is_ascii_lowercase()) {
        return Err(MuxError::InvalidTrackLanguage {
            track_id: config.track_id(),
            language: String::from_utf8_lossy(&language).into_owned(),
        });
    }
    Ok([language[0] - b'`', language[1] - b'`', language[2] - b'`'])
}

fn scale_movie_time_to_track(
    track_id: u32,
    value: u64,
    movie_timescale: u32,
    track_timescale: u32,
) -> Result<u64, MuxError> {
    if track_timescale == 0 {
        return Err(MuxError::InvalidTrackTimescale { track_id });
    }
    if movie_timescale == track_timescale {
        return Ok(value);
    }
    let scaled = value
        .checked_mul(u64::from(track_timescale))
        .ok_or(MuxError::LayoutOverflow("track time scaling"))?;
    if scaled % u64::from(movie_timescale) != 0 {
        return Err(MuxError::InvalidTrackTimescale { track_id });
    }
    Ok(scaled / u64::from(movie_timescale))
}

fn scale_track_time_to_movie(
    track_id: u32,
    value: i64,
    track_timescale: u32,
    movie_timescale: u32,
) -> Result<i64, MuxError> {
    if track_timescale == 0 || movie_timescale == 0 {
        return Err(MuxError::InvalidTrackTimescale { track_id });
    }
    let sign = value.signum();
    let magnitude = value.unsigned_abs();
    let scaled = magnitude
        .checked_mul(u64::from(movie_timescale))
        .ok_or(MuxError::LayoutOverflow("movie time scaling"))?;
    if scaled % u64::from(track_timescale) != 0 {
        return Err(MuxError::InvalidTrackTimescale { track_id });
    }
    i64::try_from(scaled / u64::from(track_timescale))
        .map(|normalized| normalized * sign)
        .map_err(|_| MuxError::LayoutOverflow("movie time scaling"))
}

fn scale_movie_offset_to_track(
    track_id: u32,
    value: i64,
    movie_timescale: u32,
    track_timescale: u32,
) -> Result<i32, MuxError> {
    if value == 0 {
        return Ok(0);
    }

    let sign = value.signum();
    let magnitude =
        u64::try_from(value.abs()).map_err(|_| MuxError::LayoutOverflow("composition offset"))?;
    let scaled =
        scale_movie_time_to_track(track_id, magnitude, movie_timescale, track_timescale)? as i64;
    let signed = scaled
        .checked_mul(sign)
        .ok_or(MuxError::LayoutOverflow("composition offset"))?;
    i32::try_from(signed).map_err(|_| MuxError::LayoutOverflow("composition offset"))
}

fn run_length_encode_u32<I>(values: I) -> Vec<(u32, u32)>
where
    I: IntoIterator<Item = u32>,
{
    let mut runs = Vec::new();
    for value in values {
        match runs.last_mut() {
            Some((sample_count, last_value)) if *last_value == value => {
                *sample_count += 1;
            }
            _ => runs.push((1, value)),
        }
    }
    runs
}

fn run_length_encode_i32<I>(values: I) -> Vec<(u32, i32)>
where
    I: IntoIterator<Item = i32>,
{
    let mut runs = Vec::new();
    for value in values {
        match runs.last_mut() {
            Some((sample_count, last_value)) if *last_value == value => {
                *sample_count += 1;
            }
            _ => runs.push((1, value)),
        }
    }
    runs
}

fn canonicalize_fragmented_sample_entry_box(sample_entry_box: &[u8]) -> Result<Vec<u8>, MuxError> {
    let sample_entry_type = sample_entry_box_type(sample_entry_box)?;
    match sample_entry_type {
        value if value == FourCc::from_bytes(*b"avc1") => {
            canonicalize_fragmented_visual_sample_entry_box(sample_entry_box, "AVC Coding", &[])
        }
        value
            if value == FourCc::from_bytes(*b"hvc1")
                || value == FourCc::from_bytes(*b"hev1")
                || value == FourCc::from_bytes(*b"dvh1")
                || value == FourCc::from_bytes(*b"dvhe") =>
        {
            canonicalize_fragmented_visual_sample_entry_box(
                sample_entry_box,
                "HEVC Coding",
                &[FourCc::from_bytes(*b"fiel")],
            )
        }
        value if value == FourCc::from_bytes(*b"vvc1") || value == FourCc::from_bytes(*b"vvi1") => {
            canonicalize_fragmented_visual_sample_entry_box(
                sample_entry_box,
                "VVC Coding",
                &[FourCc::from_bytes(*b"fiel")],
            )
        }
        value if value == FourCc::from_bytes(*b"av01") => {
            canonicalize_fragmented_visual_sample_entry_box(
                sample_entry_box,
                "AOM Coding",
                &[FourCc::from_bytes(*b"fiel")],
            )
        }
        value
            if value == FourCc::from_bytes(*b"vp08")
                || value == FourCc::from_bytes(*b"vp09")
                || value == FourCc::from_bytes(*b"vp10") =>
        {
            canonicalize_fragmented_visual_sample_entry_box(
                sample_entry_box,
                "VPC Coding",
                &[
                    FourCc::from_bytes(*b"fiel"),
                    FourCc::from_bytes(*b"pasp"),
                    FourCc::from_bytes(*b"btrt"),
                ],
            )
        }
        value if value == FourCc::from_bytes(*b"mp4a") => {
            canonicalize_fragmented_audio_sample_entry_box(sample_entry_box, true, &[])
        }
        value if value == FourCc::from_bytes(*b"alac") => {
            canonicalize_fragmented_audio_sample_entry_box(
                sample_entry_box,
                false,
                &[FourCc::from_bytes(*b"btrt")],
            )
        }
        value
            if value == FourCc::from_bytes(*b"dtsc")
                || value == FourCc::from_bytes(*b"dtse")
                || value == FourCc::from_bytes(*b"dtsh")
                || value == FourCc::from_bytes(*b"dtsl")
                || value == FourCc::from_bytes(*b"dtsm")
                || value == FourCc::from_bytes(*b"dtsx")
                || value == FourCc::from_bytes(*b"dtsy") =>
        {
            canonicalize_fragmented_audio_sample_entry_box(
                sample_entry_box,
                false,
                &[FourCc::from_bytes(*b"btrt")],
            )
        }
        _ => Ok(sample_entry_box.to_vec()),
    }
}

fn canonicalize_fragmented_visual_sample_entry_box(
    sample_entry_box: &[u8],
    compressor_name: &str,
    stripped_children: &[FourCc],
) -> Result<Vec<u8>, MuxError> {
    let (mut sample_entry, child_boxes, trailing_bytes) =
        decode_visual_sample_entry_parts(sample_entry_box)?;
    sample_entry.compressorname = encode_compressor_name(compressor_name);

    let mut normalized_children = Vec::with_capacity(child_boxes.len());
    for child_box in child_boxes {
        if stripped_children.contains(&sample_entry_box_type(&child_box)?) {
            continue;
        }
        normalized_children.push(child_box);
    }

    let mut child_payload = normalized_children.concat();
    child_payload.extend_from_slice(&trailing_bytes);
    encode_typed_box(&sample_entry, &child_payload)
}

fn canonicalize_fragmented_audio_sample_entry_box(
    sample_entry_box: &[u8],
    normalize_esds: bool,
    stripped_children: &[FourCc],
) -> Result<Vec<u8>, MuxError> {
    let (sample_entry, child_boxes, trailing_bytes) =
        decode_audio_sample_entry_parts(sample_entry_box)?;
    let mut normalized_children = Vec::with_capacity(child_boxes.len());
    for child_box in child_boxes {
        let child_type = sample_entry_box_type(&child_box)?;
        if stripped_children.contains(&child_type) {
            continue;
        }
        if normalize_esds && child_type == FourCc::from_bytes(*b"esds") {
            normalized_children.push(canonicalize_fragmented_esds_box(&child_box)?);
        } else {
            normalized_children.push(child_box);
        }
    }

    let mut child_payload = normalized_children.concat();
    child_payload.extend_from_slice(&trailing_bytes);
    encode_typed_box(&sample_entry, &child_payload)
}

fn canonicalize_fragmented_esds_box(esds_box: &[u8]) -> Result<Vec<u8>, MuxError> {
    let mut esds = decode_typed_box::<Esds>(esds_box)?;
    for descriptor in &mut esds.descriptors {
        if descriptor.tag == ES_DESCRIPTOR_TAG
            && let Some(es_descriptor) = descriptor.es_descriptor.as_mut()
        {
            es_descriptor.es_id = 0;
        }
    }
    esds.normalize_descriptor_sizes_for_mux()
        .map_err(|_| MuxError::LayoutOverflow("fragmented esds normalization"))?;
    encode_typed_box(&esds, &[])
}

fn decode_visual_sample_entry_parts(
    sample_entry_box: &[u8],
) -> Result<SampleEntryParts<VisualSampleEntry>, MuxError> {
    let mut cursor = Cursor::new(sample_entry_box);
    let info = BoxInfo::read(&mut cursor)
        .map_err(|_| MuxError::LayoutOverflow("visual sample-entry header"))?;
    let mut sample_entry = VisualSampleEntry::default();
    sample_entry.sample_entry.box_type = info.box_type();
    unmarshal(
        &mut cursor,
        info.payload_size()
            .map_err(|_| MuxError::LayoutOverflow("visual sample-entry payload"))?,
        &mut sample_entry,
        None,
    )
    .map_err(|_| MuxError::LayoutOverflow("visual sample-entry decode"))?;
    split_box_children_and_trailing(sample_entry_box, cursor.position())
        .map(|(children, trailing)| (sample_entry, children, trailing))
}

fn decode_audio_sample_entry_parts(
    sample_entry_box: &[u8],
) -> Result<SampleEntryParts<AudioSampleEntry>, MuxError> {
    let mut cursor = Cursor::new(sample_entry_box);
    let info = BoxInfo::read(&mut cursor)
        .map_err(|_| MuxError::LayoutOverflow("audio sample-entry header"))?;
    let mut sample_entry = AudioSampleEntry::default();
    sample_entry.sample_entry.box_type = info.box_type();
    unmarshal(
        &mut cursor,
        info.payload_size()
            .map_err(|_| MuxError::LayoutOverflow("audio sample-entry payload"))?,
        &mut sample_entry,
        None,
    )
    .map_err(|_| MuxError::LayoutOverflow("audio sample-entry decode"))?;
    split_box_children_and_trailing(sample_entry_box, cursor.position())
        .map(|(children, trailing)| (sample_entry, children, trailing))
}

fn decode_typed_box<B>(encoded_box: &[u8]) -> Result<B, MuxError>
where
    B: CodecBox + Default,
{
    let mut cursor = Cursor::new(encoded_box);
    let info =
        BoxInfo::read(&mut cursor).map_err(|_| MuxError::LayoutOverflow("typed box header"))?;
    let mut decoded = B::default();
    unmarshal(
        &mut cursor,
        info.payload_size()
            .map_err(|_| MuxError::LayoutOverflow("typed box payload"))?,
        &mut decoded,
        None,
    )
    .map_err(|_| MuxError::LayoutOverflow("typed box decode"))?;
    Ok(decoded)
}

fn split_box_children_and_trailing(
    sample_entry_box: &[u8],
    child_start: u64,
) -> Result<(SampleEntryChildBoxes, SampleEntryTrailingBytes), MuxError> {
    let child_start = usize::try_from(child_start)
        .map_err(|_| MuxError::LayoutOverflow("sample-entry child offset"))?;
    let remaining = sample_entry_box
        .get(child_start..)
        .ok_or(MuxError::LayoutOverflow("sample-entry child offset"))?;
    let child_bytes_len = split_box_children_with_optional_trailing_bytes(remaining);
    let child_boxes = split_immediate_box_bytes(&remaining[..child_bytes_len])?;
    Ok((child_boxes, remaining[child_bytes_len..].to_vec()))
}

fn split_immediate_box_bytes(bytes: &[u8]) -> Result<Vec<Vec<u8>>, MuxError> {
    let mut cursor = Cursor::new(bytes);
    let mut child_boxes = Vec::new();
    while cursor.position() < bytes.len() as u64 {
        let start = cursor.position();
        let info =
            BoxInfo::read(&mut cursor).map_err(|_| MuxError::LayoutOverflow("child box header"))?;
        let end = usize::try_from(
            start
                .checked_add(info.size())
                .ok_or(MuxError::LayoutOverflow("child box size"))?,
        )
        .map_err(|_| MuxError::LayoutOverflow("child box size"))?;
        child_boxes.push(bytes[start as usize..end].to_vec());
        cursor.set_position(end as u64);
    }
    Ok(child_boxes)
}

fn encode_compressor_name(name: &str) -> [u8; 32] {
    let mut encoded = [0_u8; 32];
    let visible = name.as_bytes();
    let visible_len = visible.len().min(31);
    encoded[0] = u8::try_from(visible_len).unwrap_or(31);
    encoded[1..1 + visible_len].copy_from_slice(&visible[..visible_len]);
    encoded
}

fn sample_entry_box_type(sample_entry_box: &[u8]) -> Result<FourCc, MuxError> {
    let mut cursor = Cursor::new(sample_entry_box);
    let info = BoxInfo::read(&mut cursor)
        .map_err(|_| MuxError::LayoutOverflow("sample-entry box header"))?;
    Ok(info.box_type())
}

fn build_mux_identity_id3_payload(version: &str) -> Vec<u8> {
    if version.is_empty() {
        return Vec::new();
    }

    let owner = ID3_OWNER.as_bytes();
    let value = version.as_bytes();
    let frame_payload_size = owner
        .len()
        .checked_add(1)
        .and_then(|size| size.checked_add(value.len()))
        .and_then(|size| u32::try_from(size).ok())
        .unwrap_or(0);

    let mut frames = Vec::new();
    frames.extend_from_slice(b"PRIV");
    frames.extend_from_slice(&encode_synchsafe_u32(frame_payload_size));
    frames.extend_from_slice(&0_u16.to_be_bytes());
    frames.extend_from_slice(owner);
    frames.push(0);
    frames.extend_from_slice(value);

    let mut id3 = Vec::new();
    id3.extend_from_slice(b"ID3");
    id3.push(0x04);
    id3.push(0x00);
    id3.push(0x00);
    id3.extend_from_slice(&encode_synchsafe_u32(
        u32::try_from(frames.len()).unwrap_or(0),
    ));
    id3.extend_from_slice(&frames);
    id3
}

fn encode_synchsafe_u32(value: u32) -> [u8; 4] {
    let encoded = (value & 0x7F)
        | (((value >> 7) & 0x7F) << 8)
        | (((value >> 14) & 0x7F) << 16)
        | (((value >> 21) & 0x7F) << 24);
    encoded.to_be_bytes()
}

fn copy_fragment_payloads<R, W>(
    sources: &mut [R],
    writer: &mut W,
    fragment: &FragmentLayout,
) -> Result<(), MuxError>
where
    R: Read + Seek,
    W: Write,
{
    let mut buffer = [0_u8; 16 * 1024];
    for sample in &fragment.samples {
        let source = sources
            .get_mut(sample.source_index)
            .ok_or(MuxError::LayoutOverflow("fragment source index"))?;
        source.seek(std::io::SeekFrom::Start(sample.source_data_offset))?;
        let mut remaining = sample.sample_size;
        while remaining > 0 {
            let chunk_len = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| MuxError::LayoutOverflow("fragment copy chunk"))?;
            source.read_exact(&mut buffer[..chunk_len])?;
            writer.write_all(&buffer[..chunk_len])?;
            remaining -= u64::try_from(chunk_len)
                .map_err(|_| MuxError::LayoutOverflow("fragment copy chunk"))?;
        }
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn copy_fragment_payloads_async<R, W>(
    sources: &mut [R],
    writer: &mut W,
    fragment: &FragmentLayout,
) -> Result<(), MuxError>
where
    R: AsyncReadSeek,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0_u8; 16 * 1024];
    for sample in &fragment.samples {
        let source = sources
            .get_mut(sample.source_index)
            .ok_or(MuxError::LayoutOverflow("fragment source index"))?;
        source
            .seek(std::io::SeekFrom::Start(sample.source_data_offset))
            .await?;
        let mut remaining = sample.sample_size;
        while remaining > 0 {
            let chunk_len = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| MuxError::LayoutOverflow("fragment copy chunk"))?;
            source.read_exact(&mut buffer[..chunk_len]).await?;
            writer.write_all(&buffer[..chunk_len]).await?;
            remaining -= u64::try_from(chunk_len)
                .map_err(|_| MuxError::LayoutOverflow("fragment copy chunk"))?;
        }
    }
    Ok(())
}

fn sample_flags(sample: &PreparedSample) -> u32 {
    if sample.is_sync_sample {
        0
    } else {
        NON_KEY_SAMPLE_FLAGS
    }
}

fn all_equal_u32<I>(mut values: I) -> Option<u32>
where
    I: Iterator<Item = u32>,
{
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

fn all_equal_u64<I>(mut values: I) -> Option<u64>
where
    I: Iterator<Item = u64>,
{
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

fn dominant_sample_duration<I>(values: I) -> Option<u32>
where
    I: Iterator<Item = u32>,
{
    let mut counts = BTreeMap::<u32, u32>::new();
    let mut best = None::<(u32, u32)>;
    for value in values.filter(|value| *value != 0) {
        let count = counts
            .entry(value)
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        match best {
            Some((best_value, best_count))
                if *count < best_count || (*count == best_count && value > best_value) => {}
            _ => best = Some((value, *count)),
        }
    }
    best.map(|(value, _)| value)
}
