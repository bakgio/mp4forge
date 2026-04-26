#![allow(clippy::field_reassign_with_default)]

mod support;

use std::fs;
use std::io::Cursor;

use mp4forge::boxes::iso14496_12::{Emib, Meta, Moof, Sgpd, Silb, Tfdt, Traf};
use mp4forge::extract::extract_box_as;
use mp4forge::rewrite::{
    RewriteError, rewrite_box_as, rewrite_box_as_bytes, rewrite_boxes_as_bytes,
};
#[cfg(feature = "async")]
use mp4forge::rewrite::{rewrite_box_as_async, rewrite_box_as_bytes_async};
use mp4forge::walk::BoxPath;
#[cfg(feature = "async")]
use tokio::fs::File as TokioFile;

#[cfg(feature = "async")]
use support::write_temp_file;
use support::{
    build_encrypted_fragmented_video_file, build_event_message_movie_file, encode_raw_box,
    encode_supported_box, fixture_path, fourcc,
};

#[test]
fn rewrite_box_as_updates_matching_typed_payloads() {
    let input = build_rewrite_input_file();
    let mut reader = Cursor::new(input);
    let mut output = Cursor::new(Vec::new());

    let rewritten = rewrite_box_as::<_, _, Tfdt, _>(
        &mut reader,
        &mut output,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
        |tfdt| {
            tfdt.base_media_decode_time_v0 = 12_345;
        },
    )
    .unwrap();

    let tfdt = extract_box_as::<_, Tfdt>(
        &mut Cursor::new(output.into_inner()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    )
    .unwrap();

    assert_eq!(rewritten, 1);
    assert_eq!(tfdt.len(), 1);
    assert_eq!(tfdt[0].base_media_decode_time_v0, 12_345);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_rewrite_box_as_updates_matching_typed_payloads() {
    let input = build_rewrite_input_file();
    let mut reader = Cursor::new(input);
    let mut output = Cursor::new(Vec::new());

    let rewritten = rewrite_box_as_async::<_, _, Tfdt, _>(
        &mut reader,
        &mut output,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
        |tfdt| {
            tfdt.base_media_decode_time_v0 = 12_345;
        },
    )
    .await
    .unwrap();

    let tfdt = extract_box_as::<_, Tfdt>(
        &mut Cursor::new(output.into_inner()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    )
    .unwrap();

    assert_eq!(rewritten, 1);
    assert_eq!(tfdt.len(), 1);
    assert_eq!(tfdt[0].base_media_decode_time_v0, 12_345);
}

#[test]
fn rewrite_box_as_bytes_updates_matching_typed_payloads() {
    let input = build_rewrite_input_file();
    let output = rewrite_box_as_bytes::<Tfdt, _>(
        &input,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
        |tfdt| {
            tfdt.base_media_decode_time_v0 = 12_345;
        },
    )
    .unwrap();

    let tfdt = extract_box_as::<_, Tfdt>(
        &mut Cursor::new(output),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    )
    .unwrap();

    assert_eq!(tfdt.len(), 1);
    assert_eq!(tfdt[0].base_media_decode_time_v0, 12_345);
}

#[test]
fn rewrite_box_as_bytes_updates_fragmented_encrypted_sample_group_descriptions() {
    let input = build_encrypted_fragmented_video_file();
    let output = rewrite_box_as_bytes::<Sgpd, _>(
        &input,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
        |sgpd| {
            sgpd.seig_entries_l[0].seig_entry.crypt_byte_block = 5;
            sgpd.seig_entries_l[0].seig_entry.skip_byte_block = 6;
        },
    )
    .unwrap();

    let sgpd = extract_box_as::<_, Sgpd>(
        &mut Cursor::new(output),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
    )
    .unwrap();

    assert_eq!(sgpd.len(), 1);
    assert_eq!(sgpd[0].grouping_type, fourcc("seig"));
    assert_eq!(sgpd[0].seig_entries_l.len(), 1);
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.crypt_byte_block, 5);
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.skip_byte_block, 6);
    assert_eq!(sgpd[0].seig_entries_l[0].description_length, 20);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_rewrite_box_as_bytes_updates_fragmented_encrypted_sample_group_descriptions() {
    let input = build_encrypted_fragmented_video_file();
    let output = rewrite_box_as_bytes_async::<Sgpd, _>(
        &input,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
        |sgpd| {
            sgpd.seig_entries_l[0].seig_entry.crypt_byte_block = 5;
            sgpd.seig_entries_l[0].seig_entry.skip_byte_block = 6;
        },
    )
    .await
    .unwrap();

    let sgpd = extract_box_as::<_, Sgpd>(
        &mut Cursor::new(output),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
    )
    .unwrap();

    assert_eq!(sgpd.len(), 1);
    assert_eq!(sgpd[0].grouping_type, fourcc("seig"));
    assert_eq!(sgpd[0].seig_entries_l.len(), 1);
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.crypt_byte_block, 5);
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.skip_byte_block, 6);
    assert_eq!(sgpd[0].seig_entries_l[0].description_length, 20);
}

#[test]
fn rewrite_box_as_returns_zero_and_preserves_bytes_when_nothing_matches() {
    let input = fs::read(fixture_path("sample_fragmented.mp4")).unwrap();
    let mut reader = Cursor::new(input.clone());
    let mut output = Cursor::new(Vec::new());

    let rewritten = rewrite_box_as::<_, _, Tfdt, _>(
        &mut reader,
        &mut output,
        BoxPath::from([fourcc("zzzz")]),
        |_| {},
    )
    .unwrap();

    assert_eq!(rewritten, 0);
    assert_eq!(output.into_inner(), input);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_rewrite_box_as_returns_zero_and_preserves_bytes_when_nothing_matches() {
    let input = fs::read(fixture_path("sample_fragmented.mp4")).unwrap();
    let mut reader = Cursor::new(input.clone());
    let mut output = Cursor::new(Vec::new());

    let rewritten = rewrite_box_as_async::<_, _, Tfdt, _>(
        &mut reader,
        &mut output,
        BoxPath::from([fourcc("zzzz")]),
        |_| {},
    )
    .await
    .unwrap();

    assert_eq!(rewritten, 0);
    assert_eq!(output.into_inner(), input);
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_rewrite_helpers_can_run_on_tokio_worker_threads() {
    let input = build_rewrite_input_file();
    let typed_handle = tokio::spawn(async move {
        let mut reader = Cursor::new(input);
        let mut output = Cursor::new(Vec::new());
        let rewritten = rewrite_box_as_async::<_, _, Tfdt, _>(
            &mut reader,
            &mut output,
            BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
            |tfdt| {
                tfdt.base_media_decode_time_v0 = 44_444;
            },
        )
        .await
        .unwrap();
        (rewritten, output.into_inner())
    });
    let (rewritten, typed_bytes) = typed_handle.await.unwrap();
    assert_eq!(rewritten, 1);
    assert_eq!(
        extract_box_as::<_, Tfdt>(
            &mut Cursor::new(typed_bytes),
            None,
            BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
        )
        .unwrap()[0]
            .base_media_decode_time_v0,
        44_444
    );

    let encrypted = build_encrypted_fragmented_video_file();
    let bytes_handle = tokio::spawn(async move {
        rewrite_box_as_bytes_async::<Sgpd, _>(
            &encrypted,
            BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
            |sgpd| {
                sgpd.seig_entries_l[0].seig_entry.crypt_byte_block = 7;
                sgpd.seig_entries_l[0].seig_entry.skip_byte_block = 3;
            },
        )
        .await
        .unwrap()
    });
    let rewritten_bytes = bytes_handle.await.unwrap();
    let sgpd = extract_box_as::<_, Sgpd>(
        &mut Cursor::new(rewritten_bytes),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
    )
    .unwrap();
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.crypt_byte_block, 7);
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.skip_byte_block, 3);
}

#[cfg(feature = "async")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn async_rewrite_independent_file_tasks_can_run_concurrently_on_tokio_worker_threads() {
    let tfdt_input = write_temp_file(
        "async-rewrite-concurrency-tfdt-in",
        &build_rewrite_input_file(),
    );
    let tfdt_output = write_temp_file("async-rewrite-concurrency-tfdt-out", &[]);
    let silb_input = write_temp_file(
        "async-rewrite-concurrency-silb-in",
        &build_event_message_movie_file(),
    );
    let silb_output = write_temp_file("async-rewrite-concurrency-silb-out", &[]);
    let sgpd_input = write_temp_file(
        "async-rewrite-concurrency-sgpd-in",
        &build_encrypted_fragmented_video_file(),
    );
    let sgpd_output = write_temp_file("async-rewrite-concurrency-sgpd-out", &[]);

    let tfdt_handle = tokio::spawn(async move {
        let mut reader = TokioFile::open(&tfdt_input).await.unwrap();
        let mut writer = TokioFile::create(&tfdt_output).await.unwrap();
        let rewritten = rewrite_box_as_async::<_, _, Tfdt, _>(
            &mut reader,
            &mut writer,
            BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
            |tfdt| {
                tfdt.base_media_decode_time_v0 = 55_555;
            },
        )
        .await
        .unwrap();
        (rewritten, tfdt_output)
    });

    let silb_handle = tokio::spawn(async move {
        let mut reader = TokioFile::open(&silb_input).await.unwrap();
        let mut writer = TokioFile::create(&silb_output).await.unwrap();
        let rewritten = rewrite_box_as_async::<_, _, Silb, _>(
            &mut reader,
            &mut writer,
            BoxPath::from([
                fourcc("moov"),
                fourcc("trak"),
                fourcc("mdia"),
                fourcc("minf"),
                fourcc("stbl"),
                fourcc("stsd"),
                fourcc("evte"),
                fourcc("silb"),
            ]),
            |silb| {
                silb.schemes[0].value = "event-1c".to_string();
                silb.other_schemes_flag = false;
            },
        )
        .await
        .unwrap();
        (rewritten, silb_output)
    });

    let sgpd_handle = tokio::spawn(async move {
        let mut reader = TokioFile::open(&sgpd_input).await.unwrap();
        let mut writer = TokioFile::create(&sgpd_output).await.unwrap();
        let rewritten = rewrite_box_as_async::<_, _, Sgpd, _>(
            &mut reader,
            &mut writer,
            BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
            |sgpd| {
                sgpd.seig_entries_l[0].seig_entry.crypt_byte_block = 9;
                sgpd.seig_entries_l[0].seig_entry.skip_byte_block = 4;
            },
        )
        .await
        .unwrap();
        (rewritten, sgpd_output)
    });

    let (tfdt_rewritten, tfdt_output) = tfdt_handle.await.unwrap();
    let (silb_rewritten, silb_output) = silb_handle.await.unwrap();
    let (sgpd_rewritten, sgpd_output) = sgpd_handle.await.unwrap();

    assert_eq!(tfdt_rewritten, 1);
    assert_eq!(silb_rewritten, 1);
    assert_eq!(sgpd_rewritten, 1);

    let tfdt = extract_box_as::<_, Tfdt>(
        &mut Cursor::new(fs::read(tfdt_output).unwrap()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
    )
    .unwrap();
    assert_eq!(tfdt[0].base_media_decode_time_v0, 55_555);

    let silb = extract_box_as::<_, Silb>(
        &mut Cursor::new(fs::read(silb_output).unwrap()),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("evte"),
            fourcc("silb"),
        ]),
    )
    .unwrap();
    assert_eq!(silb[0].schemes[0].value, "event-1c");
    assert!(!silb[0].other_schemes_flag);

    let sgpd = extract_box_as::<_, Sgpd>(
        &mut Cursor::new(fs::read(sgpd_output).unwrap()),
        None,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("sgpd")]),
    )
    .unwrap();
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.crypt_byte_block, 9);
    assert_eq!(sgpd[0].seig_entries_l[0].seig_entry.skip_byte_block, 4);
}

#[test]
fn rewrite_box_as_bytes_updates_event_message_boxes() {
    let input = build_event_message_movie_file();
    let output = rewrite_box_as_bytes::<Silb, _>(
        &input,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("evte"),
            fourcc("silb"),
        ]),
        |silb| {
            silb.schemes[0].value = "event-1b".to_string();
            silb.other_schemes_flag = false;
        },
    )
    .unwrap();
    let output =
        rewrite_box_as_bytes::<Emib, _>(&output, BoxPath::from([fourcc("emib")]), |emib| {
            emib.event_duration = 3_000;
            emib.value = "3".to_string();
        })
        .unwrap();

    let silb = extract_box_as::<_, Silb>(
        &mut Cursor::new(output.clone()),
        None,
        BoxPath::from([
            fourcc("moov"),
            fourcc("trak"),
            fourcc("mdia"),
            fourcc("minf"),
            fourcc("stbl"),
            fourcc("stsd"),
            fourcc("evte"),
            fourcc("silb"),
        ]),
    )
    .unwrap();
    assert_eq!(silb.len(), 1);
    assert_eq!(silb[0].schemes[0].value, "event-1b");
    assert!(!silb[0].other_schemes_flag);

    let emib = extract_box_as::<_, Emib>(
        &mut Cursor::new(output),
        None,
        BoxPath::from([fourcc("emib")]),
    )
    .unwrap();
    assert_eq!(emib.len(), 1);
    assert_eq!(emib[0].event_duration, 3_000);
    assert_eq!(emib[0].value, "3");
}

#[test]
fn rewrite_boxes_as_bytes_preserves_bytes_when_nothing_matches() {
    let input = fs::read(fixture_path("sample_fragmented.mp4")).unwrap();

    let output =
        rewrite_boxes_as_bytes::<Tfdt, _>(&input, &[BoxPath::from([fourcc("zzzz")])], |_| {})
            .unwrap();

    assert_eq!(output, input);
}

#[test]
fn rewrite_box_as_reports_payload_type_context() {
    let input = build_rewrite_input_file();
    let mut reader = Cursor::new(input);
    let mut output = Cursor::new(Vec::new());

    let error = rewrite_box_as::<_, _, Meta, _>(
        &mut reader,
        &mut output,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
        |_| {},
    )
    .unwrap_err();

    assert!(matches!(
        error,
        RewriteError::UnexpectedPayloadType {
            ref path,
            box_type,
            offset,
            expected_type
        } if path.as_slice() == [fourcc("moof"), fourcc("traf"), fourcc("tfdt")]
            && box_type == fourcc("tfdt")
            && offset == 16
            && expected_type == std::any::type_name::<Meta>()
    ));
}

#[test]
fn rewrite_box_as_bytes_reports_payload_type_context() {
    let input = build_rewrite_input_file();

    let error = rewrite_box_as_bytes::<Meta, _>(
        &input,
        BoxPath::from([fourcc("moof"), fourcc("traf"), fourcc("tfdt")]),
        |_| {},
    )
    .unwrap_err();

    assert!(matches!(
        error,
        RewriteError::UnexpectedPayloadType {
            ref path,
            box_type,
            offset,
            expected_type
        } if path.as_slice() == [fourcc("moof"), fourcc("traf"), fourcc("tfdt")]
            && box_type == fourcc("tfdt")
            && offset == 16
            && expected_type == std::any::type_name::<Meta>()
    ));
}

#[test]
fn rewrite_box_as_reports_payload_decode_context() {
    let mut bytes = encode_raw_box(fourcc("tfdt"), &[0x00, 0x00, 0x00, 0x00]);
    bytes.truncate(12);

    let mut reader = Cursor::new(bytes);
    let mut output = Cursor::new(Vec::new());

    let error = rewrite_box_as::<_, _, Tfdt, _>(
        &mut reader,
        &mut output,
        BoxPath::from([fourcc("tfdt")]),
        |_| {},
    )
    .unwrap_err();

    assert!(matches!(
        error,
        RewriteError::PayloadDecode {
            path,
            box_type,
            offset: 0,
            source: mp4forge::codec::CodecError::Io(ref io_error)
        } if path.as_slice() == [fourcc("tfdt")]
            && box_type == fourcc("tfdt")
            && io_error.kind() == std::io::ErrorKind::UnexpectedEof
    ));
}

#[test]
fn rewrite_box_as_rejects_empty_paths() {
    let mut reader = Cursor::new(Vec::<u8>::new());
    let mut output = Cursor::new(Vec::new());

    let error =
        rewrite_box_as::<_, _, Tfdt, _>(&mut reader, &mut output, BoxPath::default(), |_| {})
            .unwrap_err();

    assert!(matches!(error, RewriteError::EmptyPath));
}

fn build_rewrite_input_file() -> Vec<u8> {
    let mut tfdt = Tfdt::default();
    tfdt.base_media_decode_time_v0 = 9_000;

    let tfdt = encode_supported_box(&tfdt, &[]);
    let traf = encode_supported_box(&Traf, &tfdt);
    let moof = encode_supported_box(&Moof, &traf);
    let mdat = encode_raw_box(fourcc("mdat"), &[0, 1, 2, 3]);

    [moof, mdat].concat()
}
