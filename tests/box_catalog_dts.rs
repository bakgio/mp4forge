use std::any::type_name;
use std::fmt::Debug;
use std::io::Cursor;

use mp4forge::boxes::default_registry;
use mp4forge::boxes::dts::{Ddts, Udts};
use mp4forge::codec::{CodecBox, ImmutableBox, marshal, unmarshal, unmarshal_any};
#[cfg(feature = "async")]
use mp4forge::codec::{marshal_async, unmarshal_any_async, unmarshal_async};

fn assert_box_roundtrip<T>(src: T, payload: &[u8])
where
    T: CodecBox + Default + PartialEq + Debug + 'static,
{
    let mut encoded = Vec::new();
    let written = marshal(&mut encoded, &src, None).unwrap();
    assert_eq!(
        written,
        payload.len() as u64,
        "marshal length for {}",
        type_name::<T>()
    );
    assert_eq!(encoded, payload, "marshal bytes for {}", type_name::<T>());

    let mut decoded = T::default();
    let mut reader = Cursor::new(payload.to_vec());
    let read = unmarshal(&mut reader, payload.len() as u64, &mut decoded, None).unwrap();
    assert_eq!(
        read,
        payload.len() as u64,
        "unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(decoded, src, "unmarshal value for {}", type_name::<T>());

    let registry = default_registry();
    let mut any_reader = Cursor::new(payload.to_vec());
    let (any_box, any_read) = unmarshal_any(
        &mut any_reader,
        payload.len() as u64,
        src.box_type(),
        &registry,
        None,
    )
    .unwrap();
    assert_eq!(
        any_read,
        payload.len() as u64,
        "registry unmarshal length for {}",
        type_name::<T>()
    );
    assert_eq!(any_box.as_any().downcast_ref::<T>().unwrap(), &src);
}

#[test]
fn dts_catalog_roundtrips_ddts() {
    assert_box_roundtrip(
        Ddts {
            sampling_frequency: 48_000,
            max_bitrate: 1_536_000,
            avg_bitrate: 768_000,
            sample_depth: 16,
            frame_duration: 1,
            core_size: 1_024,
            channel_layout: 3,
            ..Ddts::default()
        },
        &[
            0x00, 0x00, 0xbb, 0x80, 0x00, 0x17, 0x70, 0x00, 0x00, 0x0b, 0xb8, 0x00, 0x10, 0x40,
            0x00, 0x40, 0x00, 0x00, 0x03, 0x00,
        ],
    );
}

#[test]
fn dts_catalog_roundtrips_udts() {
    assert_box_roundtrip(
        Udts {
            decoder_profile_code: 1,
            frame_duration_code: 1,
            max_payload_code: 1,
            num_presentations_code: 5,
            channel_mask: 3,
            id_tag_present: vec![false; 6],
            ..Udts::default()
        },
        &[0x05, 0x25, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00],
    );
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_dts_catalog_roundtrips() {
    let ddts = Ddts {
        sampling_frequency: 48_000,
        max_bitrate: 1_536_000,
        avg_bitrate: 768_000,
        sample_depth: 16,
        frame_duration: 1,
        core_size: 1_024,
        channel_layout: 3,
        ..Ddts::default()
    };
    let ddts_payload = vec![
        0x00, 0x00, 0xbb, 0x80, 0x00, 0x17, 0x70, 0x00, 0x00, 0x0b, 0xb8, 0x00, 0x10, 0x40, 0x00,
        0x40, 0x00, 0x00, 0x03, 0x00,
    ];

    let mut ddts_writer = Cursor::new(Vec::new());
    let ddts_written = marshal_async(&mut ddts_writer, &ddts, None).await.unwrap();
    assert_eq!(ddts_written, ddts_payload.len() as u64);
    assert_eq!(ddts_writer.into_inner(), ddts_payload);

    let mut ddts_reader = Cursor::new(ddts_payload.clone());
    let mut decoded_ddts = Ddts::default();
    let ddts_read = unmarshal_async(
        &mut ddts_reader,
        ddts_payload.len() as u64,
        &mut decoded_ddts,
        None,
    )
    .await
    .unwrap();
    assert_eq!(ddts_read, ddts_payload.len() as u64);
    assert_eq!(decoded_ddts, ddts);

    let registry = default_registry();
    let mut any_ddts_reader = Cursor::new(ddts_payload);
    let (any_ddts_box, any_ddts_read) =
        unmarshal_any_async(&mut any_ddts_reader, 20, ddts.box_type(), &registry, None)
            .await
            .unwrap();
    assert_eq!(any_ddts_read, 20);
    assert_eq!(any_ddts_box.as_any().downcast_ref::<Ddts>().unwrap(), &ddts);

    let udts = Udts {
        decoder_profile_code: 1,
        frame_duration_code: 1,
        max_payload_code: 1,
        num_presentations_code: 5,
        channel_mask: 3,
        id_tag_present: vec![false; 6],
        ..Udts::default()
    };
    let udts_payload = vec![0x05, 0x25, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00];

    let mut udts_writer = Cursor::new(Vec::new());
    let udts_written = marshal_async(&mut udts_writer, &udts, None).await.unwrap();
    assert_eq!(udts_written, udts_payload.len() as u64);
    assert_eq!(udts_writer.into_inner(), udts_payload);

    let mut udts_reader = Cursor::new(udts_payload.clone());
    let mut decoded_udts = Udts::default();
    let udts_read = unmarshal_async(
        &mut udts_reader,
        udts_payload.len() as u64,
        &mut decoded_udts,
        None,
    )
    .await
    .unwrap();
    assert_eq!(udts_read, udts_payload.len() as u64);
    assert_eq!(decoded_udts, udts);

    let mut any_udts_reader = Cursor::new(udts_payload);
    let (any_udts_box, any_udts_read) =
        unmarshal_any_async(&mut any_udts_reader, 8, udts.box_type(), &registry, None)
            .await
            .unwrap();
    assert_eq!(any_udts_read, 8);
    assert_eq!(any_udts_box.as_any().downcast_ref::<Udts>().unwrap(), &udts);
}
