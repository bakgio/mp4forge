#![cfg(feature = "async")]

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use mp4forge::FourCc;
use mp4forge::async_io::{AsyncReadSeek, AsyncWriteSeek};
use mp4forge::boxes::iso14496_12::Ftyp;
use mp4forge::codec::{marshal_async, unmarshal_async};
use mp4forge::header::BoxInfo;
use mp4forge::probe::probe_async;
use mp4forge::walk::{
    AsyncWalkFuture, AsyncWalkHandle, AsyncWalkVisitor, WalkControl, walk_structure_async,
};
use tokio::fs::File as TokioFile;

fn assert_async_read_seek<T: AsyncReadSeek>(_value: &mut T) {}

fn assert_async_write_seek<T: AsyncWriteSeek>(_value: &mut T) {}

#[test]
fn cursor_satisfies_async_seek_aliases() {
    let mut reader = Cursor::new(vec![0_u8; 4]);
    assert_async_read_seek(&mut reader);

    let mut writer = Cursor::new(Vec::<u8>::new());
    assert_async_write_seek(&mut writer);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_async_codec_futures_can_run_on_tokio_worker_threads() {
    let ftyp = Ftyp {
        major_brand: FourCc::from_bytes(*b"isom"),
        minor_version: 0x0200,
        compatible_brands: vec![FourCc::from_bytes(*b"isom")],
    };

    let encoded = tokio::spawn(async move {
        let mut writer = Cursor::new(Vec::new());
        marshal_async(&mut writer, &ftyp, None).await.unwrap();
        writer.into_inner()
    })
    .await
    .unwrap();

    let decoded = tokio::spawn(async move {
        let mut header_and_payload =
            Cursor::new(encode_raw_box(FourCc::from_bytes(*b"ftyp"), &encoded));
        let info = BoxInfo::read_async(&mut header_and_payload).await.unwrap();
        let mut decoded = Ftyp::default();
        unmarshal_async(
            &mut header_and_payload,
            info.payload_size().unwrap(),
            &mut decoded,
            None,
        )
        .await
        .unwrap();
        decoded
    })
    .await
    .unwrap();

    assert_eq!(decoded.major_brand, FourCc::from_bytes(*b"isom"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_walk_visitor_future_can_run_on_tokio_worker_threads() {
    let fixture = fixture_bytes();
    let visited = Arc::new(AtomicUsize::new(0));
    let visited_for_task = Arc::clone(&visited);

    let handle = tokio::spawn(async move {
        let mut reader = Cursor::new(fixture);
        walk_structure_async(
            &mut reader,
            CountingVisitor {
                visited: visited_for_task,
            },
        )
        .await
    });

    handle.await.unwrap().unwrap();
    assert!(visited.load(Ordering::Relaxed) > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probe_async_file_handles_can_run_on_tokio_worker_threads() {
    let fixture = fixture_path();

    let summary = tokio::spawn(async move {
        let mut file = TokioFile::open(fixture).await.unwrap();
        probe_async(&mut file).await.unwrap()
    })
    .await
    .unwrap();

    assert_eq!(summary.tracks.len(), 2);
}

struct CountingVisitor {
    visited: Arc<AtomicUsize>,
}

impl<R> AsyncWalkVisitor<R> for CountingVisitor
where
    R: AsyncReadSeek,
{
    type Future<'a>
        = AsyncWalkFuture<'a>
    where
        Self: 'a,
        R: 'a;

    fn visit<'a, 'r>(&'a mut self, _handle: &'a mut AsyncWalkHandle<'r, R>) -> Self::Future<'a>
    where
        'r: 'a,
    {
        let visited = Arc::clone(&self.visited);
        Box::pin(async move {
            visited.fetch_add(1, Ordering::Relaxed);
            Ok(WalkControl::Continue)
        })
    }
}

fn fixture_bytes() -> Vec<u8> {
    fs::read(fixture_path()).unwrap()
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample.mp4")
}

fn encode_raw_box(box_type: FourCc, payload: &[u8]) -> Vec<u8> {
    let info = BoxInfo::new(box_type, 8 + payload.len() as u64);
    let mut bytes = info.encode();
    bytes.extend_from_slice(payload);
    bytes
}
