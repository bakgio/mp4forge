use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;
#[cfg(feature = "async")]
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader as TokioBufReader};

use super::super::MuxError;
use super::super::MuxTrackKind;
#[cfg(feature = "async")]
use super::super::import::read_exact_at_async;
use super::super::import::{
    CandidateSample, CompositeTrackCandidate, SegmentedMuxSourceSegment,
    SegmentedMuxSourceSegmentData, SegmentedMuxSourceSpec, TrackCandidate,
    build_generic_media_sample_entry_box, direct_ingest_handler_name, direct_ingest_mux_policy,
    read_exact_at_sync,
};
use super::container_common::append_file_range_segment;
use crate::FourCc;
use crate::boxes::iso14496_14::{
    DECODER_CONFIG_DESCRIPTOR_TAG, DECODER_SPECIFIC_INFO_TAG, DecoderConfigDescriptor, Descriptor,
    ES_DESCRIPTOR_TAG, EsDescriptor, Esds, SL_CONFIG_DESCRIPTOR_TAG,
};

const VOBSUB_SECTOR_SIZE: u64 = 0x800;
pub(super) const VOBSUB_TIMESCALE: u32 = 90_000;
const VOBSUB_ENTRY: FourCc = FourCc::from_bytes(*b"mp4s");
const VOBSUB_OBJECT_TYPE_INDICATION: u8 = 0xE0;
const VOBSUB_STREAM_TYPE: u8 = 0x38;
const NULL_SUBPICTURE: [u8; 9] = [0x00, 0x09, 0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0xFF];

const VOBSUB_PREFIX: &[u8] = b"# VobSub";

const LANGUAGE_TABLE: &[([u8; 2], [u8; 3])] = &[
    (*b"--", *b"und"),
    (*b"aa", *b"aar"),
    (*b"ab", *b"abk"),
    (*b"af", *b"afr"),
    (*b"am", *b"amh"),
    (*b"ar", *b"ara"),
    (*b"as", *b"ast"),
    (*b"ay", *b"aym"),
    (*b"az", *b"aze"),
    (*b"ba", *b"bak"),
    (*b"be", *b"bel"),
    (*b"bg", *b"bul"),
    (*b"bh", *b"bih"),
    (*b"bi", *b"bis"),
    (*b"bn", *b"ben"),
    (*b"bo", *b"bod"),
    (*b"br", *b"bre"),
    (*b"ca", *b"cat"),
    (*b"cc", *b"und"),
    (*b"co", *b"cos"),
    (*b"cs", *b"ces"),
    (*b"cy", *b"cym"),
    (*b"da", *b"dan"),
    (*b"de", *b"deu"),
    (*b"dz", *b"dzo"),
    (*b"el", *b"ell"),
    (*b"en", *b"eng"),
    (*b"eo", *b"epo"),
    (*b"es", *b"spa"),
    (*b"et", *b"est"),
    (*b"eu", *b"eus"),
    (*b"fa", *b"fas"),
    (*b"fi", *b"fin"),
    (*b"fj", *b"fij"),
    (*b"fo", *b"fao"),
    (*b"fr", *b"fra"),
    (*b"fy", *b"fry"),
    (*b"ga", *b"gle"),
    (*b"gl", *b"glg"),
    (*b"gn", *b"grn"),
    (*b"gu", *b"guj"),
    (*b"ha", *b"hau"),
    (*b"he", *b"heb"),
    (*b"hi", *b"hin"),
    (*b"hr", *b"scr"),
    (*b"hu", *b"hun"),
    (*b"hy", *b"hye"),
    (*b"ia", *b"ina"),
    (*b"id", *b"ind"),
    (*b"ik", *b"ipk"),
    (*b"is", *b"isl"),
    (*b"it", *b"ita"),
    (*b"iu", *b"iku"),
    (*b"ja", *b"jpn"),
    (*b"jv", *b"jav"),
    (*b"ka", *b"kat"),
    (*b"kk", *b"kaz"),
    (*b"kl", *b"kal"),
    (*b"km", *b"khm"),
    (*b"kn", *b"kan"),
    (*b"ko", *b"kor"),
    (*b"ks", *b"kas"),
    (*b"ku", *b"kur"),
    (*b"ky", *b"kir"),
    (*b"la", *b"lat"),
    (*b"ln", *b"lin"),
    (*b"lo", *b"lao"),
    (*b"lt", *b"lit"),
    (*b"lv", *b"lav"),
    (*b"mg", *b"mlg"),
    (*b"mi", *b"mri"),
    (*b"mk", *b"mkd"),
    (*b"ml", *b"mlt"),
    (*b"mn", *b"mon"),
    (*b"mo", *b"mol"),
    (*b"mr", *b"mar"),
    (*b"ms", *b"msa"),
    (*b"my", *b"mya"),
    (*b"na", *b"nau"),
    (*b"ne", *b"nep"),
    (*b"nl", *b"nld"),
    (*b"no", *b"nor"),
    (*b"oc", *b"oci"),
    (*b"om", *b"orm"),
    (*b"or", *b"ori"),
    (*b"pa", *b"pan"),
    (*b"pl", *b"pol"),
    (*b"ps", *b"pus"),
    (*b"pt", *b"por"),
    (*b"qu", *b"que"),
    (*b"rm", *b"roh"),
    (*b"rn", *b"run"),
    (*b"ro", *b"ron"),
    (*b"ru", *b"rus"),
    (*b"rw", *b"kin"),
    (*b"sa", *b"san"),
    (*b"sd", *b"snd"),
    (*b"sg", *b"sag"),
    (*b"sh", *b"scr"),
    (*b"si", *b"sin"),
    (*b"sk", *b"slk"),
    (*b"sl", *b"slv"),
    (*b"sm", *b"smo"),
    (*b"sn", *b"sna"),
    (*b"so", *b"som"),
    (*b"sq", *b"sqi"),
    (*b"sr", *b"srp"),
    (*b"ss", *b"ssw"),
    (*b"st", *b"sot"),
    (*b"su", *b"sun"),
    (*b"sv", *b"swe"),
    (*b"sw", *b"swa"),
    (*b"ta", *b"tam"),
    (*b"te", *b"tel"),
    (*b"tg", *b"tgk"),
    (*b"th", *b"tha"),
    (*b"ti", *b"tir"),
    (*b"tk", *b"tuk"),
    (*b"tl", *b"tgl"),
    (*b"tn", *b"tsn"),
    (*b"to", *b"tog"),
    (*b"tr", *b"tur"),
    (*b"ts", *b"tso"),
    (*b"tt", *b"tat"),
    (*b"tw", *b"twi"),
    (*b"ug", *b"uig"),
    (*b"uk", *b"ukr"),
    (*b"ur", *b"urd"),
    (*b"uz", *b"uzb"),
    (*b"vi", *b"vie"),
    (*b"vo", *b"vol"),
    (*b"wo", *b"wol"),
    (*b"xh", *b"xho"),
    (*b"yi", *b"yid"),
    (*b"yo", *b"yor"),
    (*b"za", *b"zha"),
    (*b"zh", *b"zho"),
    (*b"zu", *b"zul"),
];

#[derive(Clone)]
struct VobSubIndex {
    width: u16,
    height: u16,
    palette: [[u8; 4]; 16],
    tracks: Vec<VobSubTrack>,
}

#[derive(Clone)]
struct VobSubTrack {
    index: u8,
    language: [u8; 3],
    positions: Vec<VobSubPosition>,
}

#[derive(Clone, Copy)]
struct VobSubPosition {
    start_pts: u64,
    filepos: u64,
}

struct CollectedPacket {
    packet_bytes: Vec<u8>,
    duration: u32,
    spans: Vec<(u64, u32)>,
}

struct VobSubTrackBuildContext<'a> {
    file_size: u64,
    sub_path: &'a Path,
    spec: &'a str,
    width: u16,
    height: u16,
    palette: &'a [[u8; 4]; 16],
}

#[derive(Default)]
struct VobSubIndexBuilder {
    width: Option<u16>,
    height: Option<u16>,
    palette: Option<[[u8; 4]; 16]>,
    languages: BTreeMap<u8, VobSubTrack>,
    current_track: Option<u8>,
    delays_ms: BTreeMap<u8, i64>,
}

pub(in crate::mux) fn scan_vobsub_source_sync(
    path: &Path,
    spec: &str,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    let (idx_path, sub_path) = resolve_vobsub_paths(path, spec)?;
    let index = parse_vobsub_index(&idx_path, spec)?;
    let mut file = File::open(&sub_path)?;
    let file_size = file.metadata()?.len();
    build_vobsub_tracks_sync(&mut file, file_size, &sub_path, spec, index)
}

#[cfg(feature = "async")]
pub(in crate::mux) async fn scan_vobsub_source_async(
    path: &Path,
    spec: &str,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    let (idx_path, sub_path) = resolve_vobsub_paths_async(path, spec).await?;
    let index = parse_vobsub_index_async(&idx_path, spec).await?;
    let mut file = TokioFile::open(&sub_path).await?;
    let file_size = file.metadata().await?.len();
    build_vobsub_tracks_async(&mut file, file_size, &sub_path, spec, index).await
}

pub(in crate::mux) fn looks_like_vobsub_prefix(prefix: &[u8]) -> bool {
    prefix.starts_with(VOBSUB_PREFIX)
}

fn resolve_vobsub_paths(path: &Path, spec: &str) -> Result<(PathBuf, PathBuf), MuxError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(MuxError::Io)?.join(path)
    };
    let extension = absolute
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    match extension.as_deref() {
        Some("idx") => {
            ensure_vobsub_idx_signature(&absolute, spec)?;
            let sub_path = absolute.with_extension("sub");
            if !sub_path.is_file() {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!(
                        "VobSub index input `{}` is missing its sibling `.sub` media file",
                        absolute.display()
                    ),
                });
            }
            Ok((absolute, sub_path))
        }
        Some("sub") => {
            let idx_path = absolute.with_extension("idx");
            ensure_vobsub_idx_signature(&idx_path, spec)?;
            Ok((idx_path, absolute))
        }
        _ => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "VobSub direct ingest expects one `.idx` path or one `.sub` path with a sibling `.idx` file"
                    .to_string(),
        }),
    }
}

#[cfg(feature = "async")]
async fn resolve_vobsub_paths_async(
    path: &Path,
    spec: &str,
) -> Result<(PathBuf, PathBuf), MuxError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(MuxError::Io)?.join(path)
    };
    let extension = absolute
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    match extension.as_deref() {
        Some("idx") => {
            ensure_vobsub_idx_signature_async(&absolute, spec).await?;
            let sub_path = absolute.with_extension("sub");
            let sub_exists = tokio::fs::metadata(&sub_path)
                .await
                .map(|metadata| metadata.is_file())
                .unwrap_or(false);
            if !sub_exists {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: format!(
                        "VobSub index input `{}` is missing its sibling `.sub` media file",
                        absolute.display()
                    ),
                });
            }
            Ok((absolute, sub_path))
        }
        Some("sub") => {
            let idx_path = absolute.with_extension("idx");
            ensure_vobsub_idx_signature_async(&idx_path, spec).await?;
            Ok((idx_path, absolute))
        }
        _ => Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message:
                "VobSub direct ingest expects one `.idx` path or one `.sub` path with a sibling `.idx` file"
                    .to_string(),
        }),
    }
}

fn ensure_vobsub_idx_signature(path: &Path, spec: &str) -> Result<(), MuxError> {
    let prefix = read_vobsub_prefix_sync(path)?;
    if !looks_like_vobsub_prefix(&prefix) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "`{}` is not a VobSub index file with the expected `# VobSub` signature",
                path.display()
            ),
        });
    }
    Ok(())
}

#[cfg(feature = "async")]
async fn ensure_vobsub_idx_signature_async(path: &Path, spec: &str) -> Result<(), MuxError> {
    let prefix = read_vobsub_prefix_async(path).await?;
    if !looks_like_vobsub_prefix(&prefix) {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "`{}` is not a VobSub index file with the expected `# VobSub` signature",
                path.display()
            ),
        });
    }
    Ok(())
}

fn parse_vobsub_index(path: &Path, spec: &str) -> Result<VobSubIndex, MuxError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    parse_vobsub_index_reader(&mut reader, path, spec)
}

#[cfg(feature = "async")]
async fn parse_vobsub_index_async(path: &Path, spec: &str) -> Result<VobSubIndex, MuxError> {
    let file = TokioFile::open(path).await?;
    let mut reader = TokioBufReader::new(file);
    parse_vobsub_index_reader_async(&mut reader, path, spec).await
}

fn read_vobsub_prefix_sync(path: &Path) -> Result<Vec<u8>, MuxError> {
    let mut file = File::open(path)?;
    let mut prefix = vec![0_u8; VOBSUB_PREFIX.len()];
    let mut total = 0;
    while total < prefix.len() {
        let read = file.read(&mut prefix[total..])?;
        if read == 0 {
            break;
        }
        total += read;
    }
    prefix.truncate(total);
    Ok(prefix)
}

#[cfg(feature = "async")]
async fn read_vobsub_prefix_async(path: &Path) -> Result<Vec<u8>, MuxError> {
    let mut file = TokioFile::open(path).await?;
    let mut prefix = vec![0_u8; VOBSUB_PREFIX.len()];
    let mut total = 0;
    while total < prefix.len() {
        let read = file.read(&mut prefix[total..]).await?;
        if read == 0 {
            break;
        }
        total += read;
    }
    prefix.truncate(total);
    Ok(prefix)
}

fn parse_vobsub_index_reader<R>(
    reader: &mut R,
    path: &Path,
    spec: &str,
) -> Result<VobSubIndex, MuxError>
where
    R: BufRead,
{
    let mut builder = VobSubIndexBuilder::default();
    let mut line = String::new();
    let mut line_number = 0_usize;
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                line_number += 1;
                builder.push_line(&line, spec, path, line_number)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "VobSub index files must be valid UTF-8 or ASCII text".to_string(),
                });
            }
            Err(error) => return Err(MuxError::Io(error)),
        }
    }
    builder.finish(spec, path)
}

#[cfg(feature = "async")]
async fn parse_vobsub_index_reader_async<R>(
    reader: &mut R,
    path: &Path,
    spec: &str,
) -> Result<VobSubIndex, MuxError>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut builder = VobSubIndexBuilder::default();
    let mut line = String::new();
    let mut line_number = 0_usize;
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                line_number += 1;
                builder.push_line(&line, spec, path, line_number)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "VobSub index files must be valid UTF-8 or ASCII text".to_string(),
                });
            }
            Err(error) => return Err(MuxError::Io(error)),
        }
    }
    builder.finish(spec, path)
}

impl VobSubIndexBuilder {
    fn push_line(
        &mut self,
        raw_line: &str,
        spec: &str,
        path: &Path,
        line_number: usize,
    ) -> Result<(), MuxError> {
        let line = raw_line.trim();
        if line_number == 1 {
            if !line.contains("VobSub index file, v") {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message:
                        "VobSub index files must begin with one `VobSub index file, v...` header line"
                            .to_string(),
                });
            }
            return Ok(());
        }
        if line.is_empty() || line.starts_with('#') {
            return Ok(());
        }
        let Some((entry, value)) = line.split_once(':') else {
            return Ok(());
        };
        let entry = entry.trim();
        let value = value.trim();
        if value.is_empty() {
            return Ok(());
        }
        match entry.to_ascii_lowercase().as_str() {
            "size" => {
                let (parsed_width, parsed_height) =
                    parse_vobsub_size(value, spec, path, line_number)?;
                self.width = Some(parsed_width);
                self.height = Some(parsed_height);
            }
            "palette" => {
                self.palette = Some(parse_vobsub_palette(value, spec, path, line_number)?);
            }
            "id" => {
                let (track_index, language) = parse_vobsub_id(value, spec, path, line_number)?;
                self.languages.insert(
                    track_index,
                    VobSubTrack {
                        index: track_index,
                        language,
                        positions: Vec::new(),
                    },
                );
                self.delays_ms.insert(track_index, 0);
                self.current_track = Some(track_index);
            }
            "delay" => {
                let Some(track_index) = self.current_track else {
                    return Ok(());
                };
                let delay = parse_vobsub_timestamp_ms(value, spec, path, line_number)?;
                let entry = self.delays_ms.entry(track_index).or_default();
                *entry = entry
                    .checked_add(delay)
                    .ok_or(MuxError::LayoutOverflow("VobSub delay accumulation"))?;
            }
            "timestamp" => {
                let Some(track_index) = self.current_track else {
                    return Ok(());
                };
                let (start_ms, filepos) =
                    parse_vobsub_timestamp_entry(value, spec, path, line_number)?;
                let delay_ms = *self.delays_ms.get(&track_index).unwrap_or(&0);
                let track = self.languages.get_mut(&track_index).unwrap();
                let mut adjusted_start_ms = start_ms
                    .checked_add(delay_ms)
                    .ok_or(MuxError::LayoutOverflow("VobSub timestamp adjustment"))?;
                if delay_ms < 0
                    && let Some(previous) = track.positions.last()
                {
                    let previous_ms = i64::try_from(previous.start_pts / 90)
                        .map_err(|_| MuxError::LayoutOverflow("VobSub timestamp normalization"))?;
                    if adjusted_start_ms < previous_ms {
                        let correction = previous_ms - adjusted_start_ms;
                        let entry = self.delays_ms.entry(track_index).or_default();
                        *entry = entry
                            .checked_add(correction)
                            .ok_or(MuxError::LayoutOverflow("VobSub delay correction"))?;
                        adjusted_start_ms = previous_ms;
                    }
                }
                if adjusted_start_ms < 0 {
                    return Err(MuxError::UnsupportedTrackImport {
                        spec: spec.to_string(),
                        message: format!(
                            "VobSub timestamp on line {} resolved to a negative media time",
                            line_number
                        ),
                    });
                }
                let start_pts = u64::try_from(adjusted_start_ms)
                    .map_err(|_| MuxError::LayoutOverflow("VobSub timestamp"))?
                    .checked_mul(90)
                    .ok_or(MuxError::LayoutOverflow("VobSub timestamp"))?;
                track.positions.push(VobSubPosition { start_pts, filepos });
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(self, spec: &str, path: &Path) -> Result<VobSubIndex, MuxError> {
        let width = self.width.ok_or_else(|| MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "VobSub index file `{}` is missing one `size:` declaration",
                path.display()
            ),
        })?;
        let height = self
            .height
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "VobSub index file `{}` is missing one `size:` declaration",
                    path.display()
                ),
            })?;
        let palette = self
            .palette
            .ok_or_else(|| MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "VobSub index file `{}` is missing one 16-color `palette:` declaration",
                    path.display()
                ),
            })?;
        let tracks = self
            .languages
            .into_values()
            .filter(|track| !track.positions.is_empty())
            .collect::<Vec<_>>();
        if tracks.is_empty() {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: format!(
                    "VobSub index file `{}` did not declare any subtitle positions",
                    path.display()
                ),
            });
        }
        Ok(VobSubIndex {
            width,
            height,
            palette,
            tracks,
        })
    }
}

fn parse_vobsub_size(
    value: &str,
    spec: &str,
    path: &Path,
    line_number: usize,
) -> Result<(u16, u16), MuxError> {
    let Some((width, height)) = value.split_once('x') else {
        return Err(vobsub_line_error(
            spec,
            path,
            line_number,
            "expected `size: WIDTHxHEIGHT`",
        ));
    };
    let width = width
        .trim()
        .parse::<u16>()
        .map_err(|_| vobsub_line_error(spec, path, line_number, "invalid VobSub width"))?;
    let height = height
        .trim()
        .parse::<u16>()
        .map_err(|_| vobsub_line_error(spec, path, line_number, "invalid VobSub height"))?;
    Ok((width, height))
}

fn parse_vobsub_palette(
    value: &str,
    spec: &str,
    path: &Path,
    line_number: usize,
) -> Result<[[u8; 4]; 16], MuxError> {
    let values = value
        .split(',')
        .map(|entry| {
            u32::from_str_radix(entry.trim(), 16)
                .map_err(|_| vobsub_line_error(spec, path, line_number, "invalid palette entry"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let values: [u32; 16] = values.try_into().map_err(|_| {
        vobsub_line_error(
            spec,
            path,
            line_number,
            "expected 16 comma-separated palette colors",
        )
    })?;
    let mut palette = [[0_u8; 4]; 16];
    for (index, value) in values.into_iter().enumerate() {
        let r = u8::try_from((value >> 16) & 0xFF).unwrap();
        let g = u8::try_from((value >> 8) & 0xFF).unwrap();
        let b = u8::try_from(value & 0xFF).unwrap();
        palette[index][0] = 0;
        palette[index][1] =
            ((66 * i32::from(r) + 129 * i32::from(g) + 25 * i32::from(b) + 128 + 4096) >> 8) as u8;
        palette[index][2] =
            ((112 * i32::from(r) - 94 * i32::from(g) - 18 * i32::from(b) + 128 + 32768) >> 8) as u8;
        palette[index][3] =
            ((-38 * i32::from(r) - 74 * i32::from(g) + 112 * i32::from(b) + 128 + 32768) >> 8)
                as u8;
    }
    Ok(palette)
}

fn parse_vobsub_id(
    value: &str,
    spec: &str,
    path: &Path,
    line_number: usize,
) -> Result<(u8, [u8; 3]), MuxError> {
    let lowered = value.to_ascii_lowercase();
    let language = lowered.as_bytes();
    if language.len() < 2 {
        return Err(vobsub_line_error(
            spec,
            path,
            line_number,
            "expected a two-letter VobSub language code",
        ));
    }
    let Some(index_position) = lowered.find("index:") else {
        return Err(vobsub_line_error(
            spec,
            path,
            line_number,
            "expected `id: xx, index: N`",
        ));
    };
    let index_value = lowered[index_position + "index:".len()..]
        .trim()
        .parse::<u8>()
        .map_err(|_| vobsub_line_error(spec, path, line_number, "invalid VobSub language index"))?;
    if index_value >= 32 {
        return Err(vobsub_line_error(
            spec,
            path,
            line_number,
            "VobSub language indices must stay below 32",
        ));
    }
    Ok((
        index_value,
        vobsub_language_from_two_letter([language[0], language[1]]),
    ))
}

fn parse_vobsub_timestamp_entry(
    value: &str,
    spec: &str,
    path: &Path,
    line_number: usize,
) -> Result<(i64, u64), MuxError> {
    let Some(filepos_position) = value.to_ascii_lowercase().find("filepos:") else {
        return Err(vobsub_line_error(
            spec,
            path,
            line_number,
            "expected `timestamp: HH:MM:SS:MS, filepos:XXXXXXXX`",
        ));
    };
    let start_ms = parse_vobsub_timestamp_ms(
        value[..filepos_position]
            .trim()
            .trim_end_matches(',')
            .trim(),
        spec,
        path,
        line_number,
    )?;
    let filepos = u64::from_str_radix(value[filepos_position + "filepos:".len()..].trim(), 16)
        .map_err(|_| vobsub_line_error(spec, path, line_number, "invalid VobSub filepos value"))?;
    Ok((start_ms, filepos))
}

fn parse_vobsub_timestamp_ms(
    value: &str,
    spec: &str,
    path: &Path,
    line_number: usize,
) -> Result<i64, MuxError> {
    let trimmed = value.trim();
    let (sign, digits) = if let Some(rest) = trimmed.strip_prefix('-') {
        (-1_i64, rest)
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        (1_i64, rest)
    } else {
        (1_i64, trimmed)
    };
    let parts = digits.split(':').collect::<Vec<_>>();
    if parts.len() != 4 {
        return Err(vobsub_line_error(
            spec,
            path,
            line_number,
            "expected one `HH:MM:SS:MS` timestamp value",
        ));
    }
    let hours = parts[0]
        .parse::<i64>()
        .map_err(|_| vobsub_line_error(spec, path, line_number, "invalid hour field"))?;
    let minutes = parts[1]
        .parse::<i64>()
        .map_err(|_| vobsub_line_error(spec, path, line_number, "invalid minute field"))?;
    let seconds = parts[2]
        .parse::<i64>()
        .map_err(|_| vobsub_line_error(spec, path, line_number, "invalid second field"))?;
    let milliseconds = parts[3]
        .parse::<i64>()
        .map_err(|_| vobsub_line_error(spec, path, line_number, "invalid millisecond field"))?;
    let total_ms = hours
        .checked_mul(60 * 60 * 1_000)
        .and_then(|value| value.checked_add(minutes * 60 * 1_000))
        .and_then(|value| value.checked_add(seconds * 1_000))
        .and_then(|value| value.checked_add(milliseconds))
        .ok_or(MuxError::LayoutOverflow("VobSub timestamp"))?;
    Ok(total_ms * sign)
}

fn vobsub_line_error(spec: &str, path: &Path, line_number: usize, message: &str) -> MuxError {
    MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: format!(
            "{message} in VobSub index file `{}` on line {line_number}",
            path.display()
        ),
    }
}

fn vobsub_language_from_two_letter(language: [u8; 2]) -> [u8; 3] {
    LANGUAGE_TABLE
        .iter()
        .find(|(key, _)| *key == language)
        .map(|(_, value)| *value)
        .unwrap_or(*b"und")
}

fn build_vobsub_tracks_sync(
    file: &mut File,
    file_size: u64,
    sub_path: &Path,
    spec: &str,
    index: VobSubIndex,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    let context = VobSubTrackBuildContext {
        file_size,
        sub_path,
        spec,
        width: index.width,
        height: index.height,
        palette: &index.palette,
    };
    let mut tracks = Vec::with_capacity(index.tracks.len());
    for track in index.tracks {
        tracks.push(build_vobsub_track_sync(file, &context, track)?);
    }
    Ok(tracks)
}

#[cfg(feature = "async")]
async fn build_vobsub_tracks_async(
    file: &mut TokioFile,
    file_size: u64,
    sub_path: &Path,
    spec: &str,
    index: VobSubIndex,
) -> Result<Vec<CompositeTrackCandidate>, MuxError> {
    let context = VobSubTrackBuildContext {
        file_size,
        sub_path,
        spec,
        width: index.width,
        height: index.height,
        palette: &index.palette,
    };
    let mut tracks = Vec::with_capacity(index.tracks.len());
    for track in index.tracks {
        tracks.push(build_vobsub_track_async(file, &context, track).await?);
    }
    Ok(tracks)
}

fn build_vobsub_track_sync(
    file: &mut File,
    context: &VobSubTrackBuildContext<'_>,
    track: VobSubTrack,
) -> Result<CompositeTrackCandidate, MuxError> {
    let mut segments = Vec::<SegmentedMuxSourceSegment>::new();
    let mut total_size = 0_u64;
    let mut samples = Vec::<CandidateSample>::new();
    if let Some(first) = track.positions.first()
        && first.start_pts > 0
    {
        let data_size = u32::try_from(NULL_SUBPICTURE.len())
            .map_err(|_| MuxError::LayoutOverflow("VobSub blank sample"))?;
        let data_offset = total_size;
        segments.push(SegmentedMuxSourceSegment {
            logical_offset: total_size,
            data: SegmentedMuxSourceSegmentData::Bytes(NULL_SUBPICTURE.to_vec()),
        });
        total_size = total_size
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow("VobSub blank sample"))?;
        samples.push(CandidateSample {
            source_index: usize::MAX,
            data_offset,
            data_size,
            duration: u32::try_from(first.start_pts)
                .map_err(|_| MuxError::LayoutOverflow("VobSub blank duration"))?,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }

    for (position_index, position) in track.positions.iter().copied().enumerate() {
        let next_start = track
            .positions
            .get(position_index + 1)
            .map(|value| value.start_pts);
        let packet = collect_vobsub_packet_sync(
            file,
            context.file_size,
            position,
            track.index,
            context.spec,
        )?;
        let sample_offset = total_size;
        for (source_offset, size) in &packet.spans {
            append_file_range_segment(&mut segments, &mut total_size, *source_offset, *size);
        }
        let data_size = u32::try_from(packet.packet_bytes.len())
            .map_err(|_| MuxError::LayoutOverflow("VobSub sample size"))?;
        let duration = effective_vobsub_duration(packet.duration, position.start_pts, next_start)?;
        samples.push(CandidateSample {
            source_index: usize::MAX,
            data_offset: sample_offset,
            data_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    let sample_entry_box = build_vobsub_sample_entry_box(context.palette, &samples)?;

    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(track.index) + 1,
            kind: MuxTrackKind::Subtitle,
            timescale: VOBSUB_TIMESCALE,
            language: track.language,
            handler_name: direct_ingest_handler_name("vobsub"),
            mux_policy: direct_ingest_mux_policy("vobsub", MuxTrackKind::Subtitle),
            width: context.width,
            height: context.height,
            sample_entry_box,
            source_edit_media_time: None,
            samples,
        },
        source_spec: SegmentedMuxSourceSpec {
            path: context.sub_path.to_path_buf(),
            segments,
            total_size,
        },
    })
}

#[cfg(feature = "async")]
async fn build_vobsub_track_async(
    file: &mut TokioFile,
    context: &VobSubTrackBuildContext<'_>,
    track: VobSubTrack,
) -> Result<CompositeTrackCandidate, MuxError> {
    let mut segments = Vec::<SegmentedMuxSourceSegment>::new();
    let mut total_size = 0_u64;
    let mut samples = Vec::<CandidateSample>::new();
    if let Some(first) = track.positions.first()
        && first.start_pts > 0
    {
        let data_size = u32::try_from(NULL_SUBPICTURE.len())
            .map_err(|_| MuxError::LayoutOverflow("VobSub blank sample"))?;
        let data_offset = total_size;
        segments.push(SegmentedMuxSourceSegment {
            logical_offset: total_size,
            data: SegmentedMuxSourceSegmentData::Bytes(NULL_SUBPICTURE.to_vec()),
        });
        total_size = total_size
            .checked_add(u64::from(data_size))
            .ok_or(MuxError::LayoutOverflow("VobSub blank sample"))?;
        samples.push(CandidateSample {
            source_index: usize::MAX,
            data_offset,
            data_size,
            duration: u32::try_from(first.start_pts)
                .map_err(|_| MuxError::LayoutOverflow("VobSub blank duration"))?,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }

    for (position_index, position) in track.positions.iter().copied().enumerate() {
        let next_start = track
            .positions
            .get(position_index + 1)
            .map(|value| value.start_pts);
        let packet = collect_vobsub_packet_async(
            file,
            context.file_size,
            position,
            track.index,
            context.spec,
        )
        .await?;
        let sample_offset = total_size;
        for (source_offset, size) in &packet.spans {
            append_file_range_segment(&mut segments, &mut total_size, *source_offset, *size);
        }
        let data_size = u32::try_from(packet.packet_bytes.len())
            .map_err(|_| MuxError::LayoutOverflow("VobSub sample size"))?;
        let duration = effective_vobsub_duration(packet.duration, position.start_pts, next_start)?;
        samples.push(CandidateSample {
            source_index: usize::MAX,
            data_offset: sample_offset,
            data_size,
            duration,
            composition_time_offset: 0,
            is_sync_sample: true,
        });
    }
    let sample_entry_box = build_vobsub_sample_entry_box(context.palette, &samples)?;

    Ok(CompositeTrackCandidate {
        track: TrackCandidate {
            track_id: u32::from(track.index) + 1,
            kind: MuxTrackKind::Subtitle,
            timescale: VOBSUB_TIMESCALE,
            language: track.language,
            handler_name: direct_ingest_handler_name("vobsub"),
            mux_policy: direct_ingest_mux_policy("vobsub", MuxTrackKind::Subtitle),
            width: context.width,
            height: context.height,
            sample_entry_box,
            source_edit_media_time: None,
            samples,
        },
        source_spec: SegmentedMuxSourceSpec {
            path: context.sub_path.to_path_buf(),
            segments,
            total_size,
        },
    })
}

pub(super) fn effective_vobsub_duration(
    parsed_duration: u32,
    start_pts: u64,
    next_start: Option<u64>,
) -> Result<u32, MuxError> {
    if parsed_duration != 0 {
        return Ok(parsed_duration);
    }
    if let Some(next_start) = next_start
        && next_start > start_pts
    {
        return u32::try_from(next_start - start_pts)
            .map_err(|_| MuxError::LayoutOverflow("VobSub sample duration"));
    }
    Ok(0)
}

fn collect_vobsub_packet_sync(
    file: &mut File,
    file_size: u64,
    position: VobSubPosition,
    track_index: u8,
    spec: &str,
) -> Result<CollectedPacket, MuxError> {
    let expected_substream_id = 0x20_u8 | track_index;
    let mut sector_offset = position.filepos;
    let mut packet_size = None::<u32>;
    let mut control_offset = None::<u32>;
    let mut packet_bytes = Vec::<u8>::new();
    let mut spans = Vec::<(u64, u32)>::new();
    loop {
        let sector = read_vobsub_sector_sync(file, file_size, sector_offset, spec)?;
        let header =
            parse_vobsub_sector_header(&sector, spec, sector_offset, expected_substream_id)?;
        if packet_size.is_none() {
            packet_size = Some(header.packet_size);
            control_offset = Some(header.control_offset);
        }
        let remaining = packet_size
            .unwrap()
            .checked_sub(u32::try_from(packet_bytes.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow("VobSub packet remaining bytes"))?;
        let chunk_size = remaining.min(
            u32::try_from(VOBSUB_SECTOR_SIZE - u64::try_from(header.payload_offset).unwrap())
                .map_err(|_| MuxError::LayoutOverflow("VobSub sector payload"))?,
        );
        packet_bytes.extend_from_slice(
            &sector[header.payload_offset
                ..header.payload_offset + usize::try_from(chunk_size).unwrap()],
        );
        spans.push((
            sector_offset + u64::try_from(header.payload_offset).unwrap(),
            chunk_size,
        ));
        if packet_bytes.len() == usize::try_from(packet_size.unwrap()).unwrap() {
            break;
        }
        sector_offset = find_next_vobsub_sector_sync(
            file,
            file_size,
            sector_offset + VOBSUB_SECTOR_SIZE,
            expected_substream_id,
            spec,
        )?;
    }
    let duration = parse_vobsub_duration(
        &packet_bytes,
        packet_size.unwrap(),
        control_offset.unwrap(),
        spec,
    )?;
    Ok(CollectedPacket {
        packet_bytes,
        duration,
        spans,
    })
}

#[cfg(feature = "async")]
async fn collect_vobsub_packet_async(
    file: &mut TokioFile,
    file_size: u64,
    position: VobSubPosition,
    track_index: u8,
    spec: &str,
) -> Result<CollectedPacket, MuxError> {
    let expected_substream_id = 0x20_u8 | track_index;
    let mut sector_offset = position.filepos;
    let mut packet_size = None::<u32>;
    let mut control_offset = None::<u32>;
    let mut packet_bytes = Vec::<u8>::new();
    let mut spans = Vec::<(u64, u32)>::new();
    loop {
        let sector = read_vobsub_sector_async(file, file_size, sector_offset, spec).await?;
        let header =
            parse_vobsub_sector_header(&sector, spec, sector_offset, expected_substream_id)?;
        if packet_size.is_none() {
            packet_size = Some(header.packet_size);
            control_offset = Some(header.control_offset);
        }
        let remaining = packet_size
            .unwrap()
            .checked_sub(u32::try_from(packet_bytes.len()).unwrap())
            .ok_or(MuxError::LayoutOverflow("VobSub packet remaining bytes"))?;
        let chunk_size = remaining.min(
            u32::try_from(VOBSUB_SECTOR_SIZE - u64::try_from(header.payload_offset).unwrap())
                .map_err(|_| MuxError::LayoutOverflow("VobSub sector payload"))?,
        );
        packet_bytes.extend_from_slice(
            &sector[header.payload_offset
                ..header.payload_offset + usize::try_from(chunk_size).unwrap()],
        );
        spans.push((
            sector_offset + u64::try_from(header.payload_offset).unwrap(),
            chunk_size,
        ));
        if packet_bytes.len() == usize::try_from(packet_size.unwrap()).unwrap() {
            break;
        }
        sector_offset = find_next_vobsub_sector_async(
            file,
            file_size,
            sector_offset + VOBSUB_SECTOR_SIZE,
            expected_substream_id,
            spec,
        )
        .await?;
    }
    let duration = parse_vobsub_duration(
        &packet_bytes,
        packet_size.unwrap(),
        control_offset.unwrap(),
        spec,
    )?;
    Ok(CollectedPacket {
        packet_bytes,
        duration,
        spans,
    })
}

fn read_vobsub_sector_sync(
    file: &mut File,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<[u8; VOBSUB_SECTOR_SIZE as usize], MuxError> {
    if offset
        .checked_add(VOBSUB_SECTOR_SIZE)
        .is_none_or(|end| end > file_size)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated VobSub sector at byte offset {offset}"),
        });
    }
    let mut sector = [0_u8; VOBSUB_SECTOR_SIZE as usize];
    read_exact_at_sync(file, offset, &mut sector, spec, "truncated VobSub sector")?;
    Ok(sector)
}

#[cfg(feature = "async")]
async fn read_vobsub_sector_async(
    file: &mut TokioFile,
    file_size: u64,
    offset: u64,
    spec: &str,
) -> Result<[u8; VOBSUB_SECTOR_SIZE as usize], MuxError> {
    if offset
        .checked_add(VOBSUB_SECTOR_SIZE)
        .is_none_or(|end| end > file_size)
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("truncated VobSub sector at byte offset {offset}"),
        });
    }
    let mut sector = [0_u8; VOBSUB_SECTOR_SIZE as usize];
    read_exact_at_async(file, offset, &mut sector, spec, "truncated VobSub sector").await?;
    Ok(sector)
}

struct VobSubSectorHeader {
    packet_size: u32,
    control_offset: u32,
    payload_offset: usize,
}

fn parse_vobsub_sector_header(
    sector: &[u8; VOBSUB_SECTOR_SIZE as usize],
    spec: &str,
    sector_offset: u64,
    expected_substream_id: u8,
) -> Result<VobSubSectorHeader, MuxError> {
    if sector[0..4] != [0x00, 0x00, 0x01, 0xBA]
        || sector[14] != 0
        || sector[15] != 0
        || sector[16] != 0x01
        || sector[17] != 0xBD
        || sector[21] & 0x80 == 0
        || sector[23] & 0xF0 != 0x20
    {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("corrupted VobSub sector header at byte offset {sector_offset}"),
        });
    }
    let header_extension_size = usize::from(sector[22]);
    let substream_id_offset = header_extension_size
        .checked_add(23)
        .ok_or(MuxError::LayoutOverflow("VobSub substream id offset"))?;
    if substream_id_offset >= sector.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("corrupted VobSub substream id offset at byte offset {sector_offset}"),
        });
    }
    if sector[substream_id_offset] != expected_substream_id {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!(
                "VobSub sector at byte offset {sector_offset} carried substream id 0x{:02X} instead of the expected 0x{:02X}",
                sector[substream_id_offset], expected_substream_id
            ),
        });
    }
    let payload_offset = 24usize
        .checked_add(header_extension_size)
        .ok_or(MuxError::LayoutOverflow("VobSub payload offset"))?;
    if payload_offset >= sector.len() {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("corrupted VobSub payload offset at byte offset {sector_offset}"),
        });
    }
    let packet_size = u32::from(u16::from_be_bytes([
        sector[payload_offset],
        sector[payload_offset + 1],
    ]));
    let control_offset = u32::from(u16::from_be_bytes([
        sector[payload_offset + 2],
        sector[payload_offset + 3],
    ]));
    if packet_size < control_offset || packet_size < 4 {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: format!("corrupted VobSub packet sizing at byte offset {sector_offset}"),
        });
    }
    Ok(VobSubSectorHeader {
        packet_size,
        control_offset,
        payload_offset,
    })
}

fn find_next_vobsub_sector_sync(
    file: &mut File,
    file_size: u64,
    mut search_offset: u64,
    expected_substream_id: u8,
    spec: &str,
) -> Result<u64, MuxError> {
    while search_offset
        .checked_add(VOBSUB_SECTOR_SIZE)
        .is_some_and(|end| end <= file_size)
    {
        let sector = read_vobsub_sector_sync(file, file_size, search_offset, spec)?;
        match parse_vobsub_sector_header(&sector, spec, search_offset, expected_substream_id) {
            Ok(_) => return Ok(search_offset),
            Err(MuxError::UnsupportedTrackImport { .. }) => {
                search_offset += VOBSUB_SECTOR_SIZE;
            }
            Err(error) => return Err(error),
        }
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "truncated VobSub packet continuation".to_string(),
    })
}

#[cfg(feature = "async")]
async fn find_next_vobsub_sector_async(
    file: &mut TokioFile,
    file_size: u64,
    mut search_offset: u64,
    expected_substream_id: u8,
    spec: &str,
) -> Result<u64, MuxError> {
    while search_offset
        .checked_add(VOBSUB_SECTOR_SIZE)
        .is_some_and(|end| end <= file_size)
    {
        let sector = read_vobsub_sector_async(file, file_size, search_offset, spec).await?;
        match parse_vobsub_sector_header(&sector, spec, search_offset, expected_substream_id) {
            Ok(_) => return Ok(search_offset),
            Err(MuxError::UnsupportedTrackImport { .. }) => {
                search_offset += VOBSUB_SECTOR_SIZE;
            }
            Err(error) => return Err(error),
        }
    }
    Err(MuxError::UnsupportedTrackImport {
        spec: spec.to_string(),
        message: "truncated VobSub packet continuation".to_string(),
    })
}

pub(super) fn parse_vobsub_duration(
    packet_bytes: &[u8],
    packet_size: u32,
    control_offset: u32,
    spec: &str,
) -> Result<u32, MuxError> {
    let packet_size =
        usize::try_from(packet_size).map_err(|_| MuxError::LayoutOverflow("VobSub packet size"))?;
    let control_offset = usize::try_from(control_offset)
        .map_err(|_| MuxError::LayoutOverflow("VobSub control offset"))?;
    if packet_bytes.len() != packet_size || control_offset > packet_size {
        return Err(MuxError::UnsupportedTrackImport {
            spec: spec.to_string(),
            message: "corrupted VobSub packet lengths".to_string(),
        });
    }
    let mut next_control = control_offset;
    let mut start_pts = 0_u32;
    let mut stop_pts = 0_u32;
    loop {
        let mut index = next_control;
        if index + 4 > packet_size {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "corrupted VobSub control sequence header".to_string(),
            });
        }
        let control_time = u32::from(u16::from_be_bytes([
            packet_bytes[index],
            packet_bytes[index + 1],
        ]));
        next_control = usize::from(u16::from_be_bytes([
            packet_bytes[index + 2],
            packet_bytes[index + 3],
        ]));
        index += 4;
        if next_control > packet_size || next_control < control_offset {
            return Err(MuxError::UnsupportedTrackImport {
                spec: spec.to_string(),
                message: "corrupted VobSub control-sequence offset".to_string(),
            });
        }
        loop {
            if index >= packet_size {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "truncated VobSub control command payload".to_string(),
                });
            }
            let command = packet_bytes[index];
            index += 1;
            let extra = match command {
                0x00..=0x02 => 0usize,
                0x03 | 0x04 => 2,
                0x05 => 6,
                0x06 => 4,
                _ => break,
            };
            if index + extra > packet_size {
                return Err(MuxError::UnsupportedTrackImport {
                    spec: spec.to_string(),
                    message: "truncated VobSub control command data".to_string(),
                });
            }
            index += extra;
            if matches!(command, 0x00 | 0x01) {
                start_pts = control_time.saturating_mul(1024);
            } else if command == 0x02 {
                stop_pts = control_time.saturating_mul(1024);
            }
        }
        if !(index <= next_control && index < packet_size) {
            break;
        }
    }
    Ok(stop_pts.saturating_sub(start_pts))
}

pub(super) fn build_subpicture_sample_entry_box(
    decoder_specific_info: &[u8],
    samples: &[CandidateSample],
) -> Result<Vec<u8>, MuxError> {
    let buffer_size_db = samples
        .iter()
        .map(|sample| sample.data_size)
        .max()
        .unwrap_or(0);
    let total_size_bits = samples
        .iter()
        .try_fold(0_u128, |total, sample| {
            total.checked_add(u128::from(sample.data_size) * 8)
        })
        .ok_or(MuxError::LayoutOverflow("VobSub total bitrate"))?;
    let total_duration = samples
        .iter()
        .try_fold(0_u64, |total, sample| {
            total.checked_add(u64::from(sample.duration))
        })
        .ok_or(MuxError::LayoutOverflow("VobSub total duration"))?;
    let average_bitrate = if total_duration == 0 || total_size_bits == 0 {
        0
    } else {
        u32::try_from(
            total_size_bits
                .checked_mul(u128::from(VOBSUB_TIMESCALE))
                .ok_or(MuxError::LayoutOverflow("VobSub total bitrate"))?
                / u128::from(total_duration),
        )
        .map_err(|_| MuxError::LayoutOverflow("VobSub average bitrate"))?
    };

    let mut esds = Esds::default();
    esds.descriptors = vec![
        Descriptor {
            tag: ES_DESCRIPTOR_TAG,
            es_descriptor: Some(EsDescriptor::default()),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_CONFIG_DESCRIPTOR_TAG,
            decoder_config_descriptor: Some(DecoderConfigDescriptor {
                object_type_indication: VOBSUB_OBJECT_TYPE_INDICATION,
                stream_type: VOBSUB_STREAM_TYPE,
                reserved: true,
                buffer_size_db,
                max_bitrate: average_bitrate,
                avg_bitrate: average_bitrate,
                ..DecoderConfigDescriptor::default()
            }),
            ..Descriptor::default()
        },
        Descriptor {
            tag: DECODER_SPECIFIC_INFO_TAG,
            size: u32::try_from(decoder_specific_info.len())
                .map_err(|_| MuxError::LayoutOverflow("VobSub decoder config size"))?,
            data: decoder_specific_info.to_vec(),
            ..Descriptor::default()
        },
        Descriptor {
            tag: SL_CONFIG_DESCRIPTOR_TAG,
            size: 1,
            data: vec![0x02],
            ..Descriptor::default()
        },
    ];
    esds.normalize_descriptor_sizes_for_mux()
        .map_err(|_| MuxError::LayoutOverflow("VobSub esds"))?;
    let esds_box = super::super::mp4::encode_typed_box(&esds, &[])?;
    build_generic_media_sample_entry_box(VOBSUB_ENTRY, &[esds_box])
}

fn build_vobsub_sample_entry_box(
    palette: &[[u8; 4]; 16],
    samples: &[CandidateSample],
) -> Result<Vec<u8>, MuxError> {
    let mut decoder_specific_info = Vec::with_capacity(palette.len() * 4);
    for color in palette {
        decoder_specific_info.extend_from_slice(color);
    }
    build_subpicture_sample_entry_box(&decoder_specific_info, samples)
}
