use std::io::Cursor;

use mp4forge::boxes::etsi_ts_102_366::Dac3;
use mp4forge::boxes::iso14496_12::{AudioSampleEntry, Btrt, Meta, Moov, SampleEntry, Trak, Udta};
use mp4forge::codec::{CodecBox, marshal};
use mp4forge::header::HeaderError;
#[cfg(feature = "async")]
use mp4forge::walk::{
    AsyncWalkFuture, AsyncWalkHandle, AsyncWalkVisitor, walk_structure_async,
    walk_structure_from_box_async,
};
use mp4forge::walk::{BoxPath, WalkControl, WalkError, walk_structure, walk_structure_from_box};
use mp4forge::{BoxInfo, FourCc};

#[cfg(feature = "async")]
type AsyncCursorWalkHandle<'a> = AsyncWalkHandle<'a, Cursor<Vec<u8>>>;

#[cfg(feature = "async")]
struct AsyncTrackingVisitor<'a> {
    visited: &'a mut Vec<BoxPath>,
}

#[cfg(feature = "async")]
impl AsyncWalkVisitor<Cursor<Vec<u8>>> for AsyncTrackingVisitor<'_> {
    type Future<'a>
        = AsyncWalkFuture<'a>
    where
        Self: 'a;

    fn visit<'a, 'r>(&'a mut self, handle: &'a mut AsyncCursorWalkHandle<'r>) -> Self::Future<'a>
    where
        'r: 'a,
    {
        Box::pin(async move {
            self.visited.push(handle.path().clone());

            match handle.info().box_type() {
                box_type if box_type == fourcc("moov") => {
                    let (payload, read) = handle.read_payload_async().await?;
                    assert_eq!(read, 0);
                    assert!(payload.as_ref().as_any().is::<Moov>());
                    Ok(WalkControl::Descend)
                }
                box_type if box_type == fourcc("trak") => {
                    let (payload, read) = handle.read_payload_async().await?;
                    assert_eq!(read, 0);
                    assert!(payload.as_ref().as_any().is::<Trak>());
                    Ok(WalkControl::Continue)
                }
                box_type if box_type == fourcc("meta") => {
                    let (payload, read) = handle.read_payload_async().await?;
                    assert_eq!(read, 4);
                    let meta = payload.as_ref().as_any().downcast_ref::<Meta>().unwrap();
                    assert!(!meta.is_quicktime_headerless());
                    Ok(WalkControl::Continue)
                }
                box_type if box_type == fourcc("udta") => Ok(WalkControl::Descend),
                box_type if box_type == fourcc("zzzz") => {
                    assert!(!handle.is_supported_type());
                    let mut raw = Vec::new();
                    assert_eq!(handle.read_data_async(&mut raw).await?, 4);
                    assert_eq!(raw, vec![0xde, 0xad, 0xbe, 0xef]);
                    Ok(WalkControl::Continue)
                }
                other => panic!("unexpected box {other}"),
            }
        })
    }
}

#[cfg(feature = "async")]
struct AsyncMoovInfoVisitor<'a> {
    moov_info: &'a mut Option<BoxInfo>,
}

#[cfg(feature = "async")]
impl AsyncWalkVisitor<Cursor<Vec<u8>>> for AsyncMoovInfoVisitor<'_> {
    type Future<'a>
        = AsyncWalkFuture<'a>
    where
        Self: 'a;

    fn visit<'a, 'r>(&'a mut self, handle: &'a mut AsyncCursorWalkHandle<'r>) -> Self::Future<'a>
    where
        'r: 'a,
    {
        Box::pin(async move {
            if handle.info().box_type() == fourcc("moov") {
                *self.moov_info = Some(*handle.info());
            }
            Ok(WalkControl::Continue)
        })
    }
}

#[cfg(feature = "async")]
struct AsyncDescendMoovVisitor<'a> {
    visited: &'a mut Vec<BoxPath>,
}

#[cfg(feature = "async")]
impl AsyncWalkVisitor<Cursor<Vec<u8>>> for AsyncDescendMoovVisitor<'_> {
    type Future<'a>
        = AsyncWalkFuture<'a>
    where
        Self: 'a;

    fn visit<'a, 'r>(&'a mut self, handle: &'a mut AsyncCursorWalkHandle<'r>) -> Self::Future<'a>
    where
        'r: 'a,
    {
        Box::pin(async move {
            self.visited.push(handle.path().clone());

            if handle.info().box_type() == fourcc("moov") {
                return Ok(WalkControl::Descend);
            }

            Ok(WalkControl::Continue)
        })
    }
}

#[cfg(feature = "async")]
struct AsyncAudioSampleEntryTailVisitor<'a> {
    visited: &'a mut Vec<BoxPath>,
}

#[cfg(feature = "async")]
impl AsyncWalkVisitor<Cursor<Vec<u8>>> for AsyncAudioSampleEntryTailVisitor<'_> {
    type Future<'a>
        = AsyncWalkFuture<'a>
    where
        Self: 'a;

    fn visit<'a, 'r>(&'a mut self, handle: &'a mut AsyncCursorWalkHandle<'r>) -> Self::Future<'a>
    where
        'r: 'a,
    {
        Box::pin(async move {
            self.visited.push(handle.path().clone());
            match handle.info().box_type() {
                box_type if box_type == fourcc("ac-3") => {
                    let (payload, read) = handle.read_payload_async().await?;
                    assert_eq!(read, 28);
                    assert!(payload.as_ref().as_any().is::<AudioSampleEntry>());
                    Ok(WalkControl::Descend)
                }
                box_type if box_type == fourcc("dac3") => {
                    let (payload, read) = handle.read_payload_async().await?;
                    assert_eq!(read, 3);
                    assert!(payload.as_ref().as_any().is::<Dac3>());
                    Ok(WalkControl::Continue)
                }
                other => panic!("unexpected box {other}"),
            }
        })
    }
}

#[test]
fn walk_structure_tracks_paths_and_supports_raw_payload_reads() {
    let unknown = encode_raw_box(fourcc("zzzz"), &[0xde, 0xad, 0xbe, 0xef]);
    let trak = encode_supported_box(&Trak, &[]);
    let udta = encode_supported_box(&Udta, &unknown);
    let meta = encode_supported_box(&Meta::default(), &[]);
    let moov = encode_supported_box(&Moov, &[trak.clone(), meta, udta.clone()].concat());
    let file = moov.clone();

    let mut visited = Vec::new();
    walk_structure(&mut Cursor::new(file), |handle| {
        visited.push(handle.path().clone());

        match handle.info().box_type() {
            box_type if box_type == fourcc("moov") => {
                let (payload, read) = handle.read_payload()?;
                assert_eq!(read, 0);
                assert!(payload.as_ref().as_any().is::<Moov>());
                Ok(WalkControl::Descend)
            }
            box_type if box_type == fourcc("trak") => {
                let (payload, read) = handle.read_payload()?;
                assert_eq!(read, 0);
                assert!(payload.as_ref().as_any().is::<Trak>());
                Ok(WalkControl::Continue)
            }
            box_type if box_type == fourcc("meta") => {
                let (payload, read) = handle.read_payload()?;
                assert_eq!(read, 4);
                let meta = payload.as_ref().as_any().downcast_ref::<Meta>().unwrap();
                assert!(!meta.is_quicktime_headerless());
                Ok(WalkControl::Continue)
            }
            box_type if box_type == fourcc("udta") => Ok(WalkControl::Descend),
            box_type if box_type == fourcc("zzzz") => {
                assert!(!handle.is_supported_type());
                let mut raw = Vec::new();
                assert_eq!(handle.read_data(&mut raw)?, 4);
                assert_eq!(raw, vec![0xde, 0xad, 0xbe, 0xef]);
                Ok(WalkControl::Continue)
            }
            other => panic!("unexpected box {other}"),
        }
    })
    .unwrap();

    assert_eq!(
        visited,
        vec![
            BoxPath::from([fourcc("moov")]),
            BoxPath::from([fourcc("moov"), fourcc("trak")]),
            BoxPath::from([fourcc("moov"), fourcc("meta")]),
            BoxPath::from([fourcc("moov"), fourcc("udta")]),
            BoxPath::from([fourcc("moov"), fourcc("udta"), fourcc("zzzz")]),
        ]
    );
}

#[test]
fn walk_structure_from_box_reuses_parent_metadata_and_paths() {
    let trak = encode_supported_box(&Trak, &[]);
    let udta = encode_supported_box(&Udta, &[]);
    let moov_bytes = encode_supported_box(&Moov, &[trak, udta].concat());

    let mut moov_info = None;
    walk_structure(&mut Cursor::new(moov_bytes.clone()), |handle| {
        if handle.info().box_type() == fourcc("moov") {
            moov_info = Some(*handle.info());
            return Ok(WalkControl::Continue);
        }

        Ok(WalkControl::Continue)
    })
    .unwrap();

    let parent = moov_info.unwrap();
    let mut visited = Vec::new();
    walk_structure_from_box(&mut Cursor::new(moov_bytes), &parent, |handle| {
        visited.push(handle.path().clone());

        if handle.info().box_type() == fourcc("moov") {
            return Ok(WalkControl::Descend);
        }

        Ok(WalkControl::Continue)
    })
    .unwrap();

    assert_eq!(
        visited,
        vec![
            BoxPath::from([fourcc("moov")]),
            BoxPath::from([fourcc("moov"), fourcc("trak")]),
            BoxPath::from([fourcc("moov"), fourcc("udta")]),
        ]
    );
}

#[test]
fn walk_structure_reports_invalid_zero_sized_boxes() {
    let bytes = vec![
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01,
    ];

    let error = walk_structure(&mut Cursor::new(bytes), |_| Ok(WalkControl::Continue)).unwrap_err();
    assert!(matches!(error, WalkError::Header(HeaderError::InvalidSize)));
}

#[test]
fn walk_structure_rejects_truncated_root_payload_read_without_large_allocation() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u32::MAX.to_be_bytes());
    bytes.extend_from_slice(b"moov");
    bytes.extend_from_slice(&[0, 0, 0, 0]);

    let mut visited = false;
    let error = walk_structure(&mut Cursor::new(bytes), |handle| {
        visited = true;
        handle.read_payload()?;
        Ok(WalkControl::Continue)
    })
    .unwrap_err();

    assert!(visited);
    assert!(matches!(error, WalkError::UnexpectedEof));
}

#[test]
fn walk_structure_rejects_root_box_end_overflow_without_looping() {
    let mut bytes = encode_raw_box(fourcc("free"), &[]);
    bytes.extend_from_slice(&1_u32.to_be_bytes());
    bytes.extend_from_slice(b"mdat");
    bytes.extend_from_slice(&u64::MAX.to_be_bytes());

    let error = walk_structure(&mut Cursor::new(bytes), |_| Ok(WalkControl::Continue)).unwrap_err();

    assert!(matches!(error, WalkError::UnexpectedEof));
}

#[test]
fn walk_structure_handles_truncated_supported_root_payload_without_looping() {
    let bytes = vec![
        93, 93, 115, 98, 115, 105, 108, 98, 101, 118, 99, 115, 116, 116, 0, 4, 117,
    ];

    walk_structure(&mut Cursor::new(bytes), |handle| {
        if !handle.is_supported_type() {
            return Ok(WalkControl::Continue);
        }

        if handle.read_payload().is_ok() {
            Ok(WalkControl::Descend)
        } else {
            Ok(WalkControl::Continue)
        }
    })
    .unwrap();
}

#[test]
fn walk_structure_handles_truncated_supported_root_payload_from_slice_without_looping() {
    let bytes = [
        93, 93, 115, 98, 115, 105, 108, 98, 101, 118, 99, 115, 116, 116, 0, 4, 117,
    ];

    walk_structure(&mut Cursor::new(bytes.as_slice()), |handle| {
        if !handle.is_supported_type() {
            return Ok(WalkControl::Continue);
        }

        if handle.read_payload().is_ok() {
            Ok(WalkControl::Descend)
        } else {
            Ok(WalkControl::Continue)
        }
    })
    .unwrap();
}

#[test]
fn walk_structure_ignores_truncated_trailing_root_box_after_valid_boxes() {
    let moov = encode_supported_box(&Moov, &[]);
    let mut truncated_mdat = Vec::new();
    truncated_mdat.extend_from_slice(&32_u32.to_be_bytes());
    truncated_mdat.extend_from_slice(b"mdat");
    truncated_mdat.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    let file = [moov, truncated_mdat].concat();

    let mut visited = Vec::new();
    walk_structure(&mut Cursor::new(file), |handle| {
        visited.push(handle.path().clone());
        Ok(WalkControl::Continue)
    })
    .unwrap();

    assert_eq!(
        visited,
        vec![
            BoxPath::from([fourcc("moov")]),
            BoxPath::from([fourcc("mdat")]),
        ]
    );
}

#[test]
fn walk_structure_stops_audio_sample_entry_children_before_zero_tail() {
    let sample_entry = AudioSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc("ac-3"),
            data_reference_index: 1,
        },
        channel_count: 2,
        sample_size: 16,
        sample_rate: 48_000 << 16,
        ..AudioSampleEntry::default()
    };
    let dac3 = Dac3 {
        fscod: 0,
        bsid: 8,
        bsmod: 0,
        acmod: 7,
        lfe_on: 1,
        bit_rate_code: 15,
    };

    let mut payload = Vec::new();
    marshal(&mut payload, &sample_entry, None).unwrap();
    payload.extend_from_slice(&encode_supported_box(&dac3, &[]));
    payload.extend_from_slice(&[0; 8]);
    let file = encode_raw_box(fourcc("ac-3"), &payload);

    let mut visited = Vec::new();
    walk_structure(&mut Cursor::new(file), |handle| {
        visited.push(handle.path().clone());
        match handle.info().box_type() {
            box_type if box_type == fourcc("ac-3") => {
                let (payload, read) = handle.read_payload()?;
                assert_eq!(read, 28);
                assert!(payload.as_ref().as_any().is::<AudioSampleEntry>());
                Ok(WalkControl::Descend)
            }
            box_type if box_type == fourcc("dac3") => {
                let (payload, read) = handle.read_payload()?;
                assert_eq!(read, 3);
                assert!(payload.as_ref().as_any().is::<Dac3>());
                Ok(WalkControl::Continue)
            }
            other => panic!("unexpected box {other}"),
        }
    })
    .unwrap();

    assert_eq!(
        visited,
        vec![
            BoxPath::from([fourcc("ac-3")]),
            BoxPath::from([fourcc("ac-3"), fourcc("dac3")]),
        ]
    );
}

#[test]
fn walk_structure_accepts_zero_typed_audio_sample_entry_child_boxes() {
    let sample_entry = AudioSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc("ac-3"),
            data_reference_index: 1,
        },
        channel_count: 2,
        sample_size: 16,
        sample_rate: 48_000 << 16,
        ..AudioSampleEntry::default()
    };
    let dac3 = Dac3 {
        fscod: 0,
        bsid: 8,
        bsmod: 0,
        acmod: 7,
        lfe_on: 1,
        bit_rate_code: 15,
    };
    let btrt = Btrt {
        buffer_size_db: 1_792,
        max_bitrate: 473_088,
        avg_bitrate: 448_120,
    };

    let mut payload = Vec::new();
    marshal(&mut payload, &sample_entry, None).unwrap();
    payload.extend_from_slice(&encode_supported_box(&dac3, &[]));
    payload.extend_from_slice(&encode_raw_box(FourCc::from_u32(0), &[]));
    payload.extend_from_slice(&encode_supported_box(&btrt, &[]));
    let file = encode_raw_box(fourcc("ac-3"), &payload);

    let mut visited = Vec::new();
    walk_structure(&mut Cursor::new(file), |handle| {
        visited.push(handle.path().clone());
        match handle.info().box_type() {
            box_type if box_type == fourcc("ac-3") => {
                let (payload, read) = handle.read_payload()?;
                assert_eq!(read, 28);
                assert!(payload.as_ref().as_any().is::<AudioSampleEntry>());
                Ok(WalkControl::Descend)
            }
            box_type if box_type == fourcc("dac3") => Ok(WalkControl::Continue),
            box_type if box_type == FourCc::from_u32(0) => Ok(WalkControl::Continue),
            box_type if box_type == fourcc("btrt") => Ok(WalkControl::Continue),
            other => panic!("unexpected box {other}"),
        }
    })
    .unwrap();

    assert_eq!(
        visited,
        vec![
            BoxPath::from([fourcc("ac-3")]),
            BoxPath::from([fourcc("ac-3"), fourcc("dac3")]),
            BoxPath::from([fourcc("ac-3"), FourCc::from_u32(0)]),
            BoxPath::from([fourcc("ac-3"), fourcc("btrt")]),
        ]
    );
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_walk_structure_tracks_paths_and_supports_raw_payload_reads() {
    let unknown = encode_raw_box(fourcc("zzzz"), &[0xde, 0xad, 0xbe, 0xef]);
    let trak = encode_supported_box(&Trak, &[]);
    let udta = encode_supported_box(&Udta, &unknown);
    let meta = encode_supported_box(&Meta::default(), &[]);
    let moov = encode_supported_box(&Moov, &[trak.clone(), meta, udta.clone()].concat());
    let file = moov.clone();

    let mut visited = Vec::new();
    let visitor = AsyncTrackingVisitor {
        visited: &mut visited,
    };
    walk_structure_async(&mut Cursor::new(file), visitor)
        .await
        .unwrap();

    assert_eq!(
        visited,
        vec![
            BoxPath::from([fourcc("moov")]),
            BoxPath::from([fourcc("moov"), fourcc("trak")]),
            BoxPath::from([fourcc("moov"), fourcc("meta")]),
            BoxPath::from([fourcc("moov"), fourcc("udta")]),
            BoxPath::from([fourcc("moov"), fourcc("udta"), fourcc("zzzz")]),
        ]
    );
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_walk_structure_stops_audio_sample_entry_children_before_zero_tail() {
    let sample_entry = AudioSampleEntry {
        sample_entry: SampleEntry {
            box_type: fourcc("ac-3"),
            data_reference_index: 1,
        },
        channel_count: 2,
        sample_size: 16,
        sample_rate: 48_000 << 16,
        ..AudioSampleEntry::default()
    };
    let dac3 = Dac3 {
        fscod: 0,
        bsid: 8,
        bsmod: 0,
        acmod: 7,
        lfe_on: 1,
        bit_rate_code: 15,
    };

    let mut payload = Vec::new();
    marshal(&mut payload, &sample_entry, None).unwrap();
    payload.extend_from_slice(&encode_supported_box(&dac3, &[]));
    payload.extend_from_slice(&[0; 8]);
    let file = encode_raw_box(fourcc("ac-3"), &payload);

    let mut visited = Vec::new();
    let visitor = AsyncAudioSampleEntryTailVisitor {
        visited: &mut visited,
    };
    walk_structure_async(&mut Cursor::new(file), visitor)
        .await
        .unwrap();

    assert_eq!(
        visited,
        vec![
            BoxPath::from([fourcc("ac-3")]),
            BoxPath::from([fourcc("ac-3"), fourcc("dac3")]),
        ]
    );
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_walk_structure_from_box_reuses_parent_metadata_and_paths() {
    let trak = encode_supported_box(&Trak, &[]);
    let udta = encode_supported_box(&Udta, &[]);
    let moov_bytes = encode_supported_box(&Moov, &[trak, udta].concat());

    let mut moov_info = None;
    let visitor = AsyncMoovInfoVisitor {
        moov_info: &mut moov_info,
    };
    walk_structure_async(&mut Cursor::new(moov_bytes.clone()), visitor)
        .await
        .unwrap();

    let parent = moov_info.unwrap();
    let mut visited = Vec::new();
    let visitor = AsyncDescendMoovVisitor {
        visited: &mut visited,
    };
    walk_structure_from_box_async(&mut Cursor::new(moov_bytes), &parent, visitor)
        .await
        .unwrap();

    assert_eq!(
        visited,
        vec![
            BoxPath::from([fourcc("moov")]),
            BoxPath::from([fourcc("moov"), fourcc("trak")]),
            BoxPath::from([fourcc("moov"), fourcc("udta")]),
        ]
    );
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_walk_structure_ignores_truncated_trailing_root_box_after_valid_boxes() {
    let moov = encode_supported_box(&Moov, &[]);
    let mut truncated_mdat = Vec::new();
    truncated_mdat.extend_from_slice(&32_u32.to_be_bytes());
    truncated_mdat.extend_from_slice(b"mdat");
    truncated_mdat.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    let file = [moov, truncated_mdat].concat();

    let mut visited = Vec::new();
    struct AsyncCollectVisitor<'a> {
        visited: &'a mut Vec<BoxPath>,
    }

    impl AsyncWalkVisitor<Cursor<Vec<u8>>> for AsyncCollectVisitor<'_> {
        type Future<'a>
            = AsyncWalkFuture<'a>
        where
            Self: 'a;

        fn visit<'a, 'r>(
            &'a mut self,
            handle: &'a mut AsyncCursorWalkHandle<'r>,
        ) -> Self::Future<'a>
        where
            'r: 'a,
        {
            Box::pin(async move {
                self.visited.push(handle.path().clone());
                Ok(WalkControl::Continue)
            })
        }
    }

    let visitor = AsyncCollectVisitor {
        visited: &mut visited,
    };

    walk_structure_async(&mut Cursor::new(file), visitor)
        .await
        .unwrap();

    assert_eq!(
        visited,
        vec![
            BoxPath::from([fourcc("moov")]),
            BoxPath::from([fourcc("mdat")]),
        ]
    );
}

fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).unwrap()
}

fn encode_supported_box<B>(box_value: &B, children: &[u8]) -> Vec<u8>
where
    B: CodecBox,
{
    let mut payload = Vec::new();
    marshal(&mut payload, box_value, None).unwrap();
    payload.extend_from_slice(children);
    encode_raw_box(box_value.box_type(), &payload)
}

fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Vec<u8> {
    let info = BoxInfo::new(box_type, 8 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(payload);
    bytes
}
