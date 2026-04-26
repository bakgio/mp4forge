use std::io::Cursor;

use mp4forge::boxes::iso14496_12::{Meta, Moov, Trak, Udta};
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
