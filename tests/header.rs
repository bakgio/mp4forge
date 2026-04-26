use std::io::{Cursor, Seek, SeekFrom};

use mp4forge::{BoxInfo, FourCc, HeaderError, HeaderForm, LARGE_HEADER_SIZE, SMALL_HEADER_SIZE};

#[test]
fn write_preserves_offsets_and_selects_expected_header_forms() {
    let cases = [
        (
            "small",
            Vec::new(),
            BoxInfo::new(fourcc(), 0x0001_2345).with_header_size(SMALL_HEADER_SIZE),
            BoxInfo::new(fourcc(), 0x0001_2345).with_header_size(SMALL_HEADER_SIZE),
            vec![0x00, 0x01, 0x23, 0x45, b't', b'e', b's', b't'],
        ),
        (
            "large",
            Vec::new(),
            BoxInfo::new(fourcc(), 0x0000_1234_5678_9abc).with_header_size(SMALL_HEADER_SIZE),
            BoxInfo::new(fourcc(), 0x0000_1234_5678_9ac4).with_header_size(LARGE_HEADER_SIZE),
            vec![
                0x00, 0x00, 0x00, 0x01, b't', b'e', b's', b't', 0x00, 0x00, 0x12, 0x34, 0x56, 0x78,
                0x9a, 0xbc,
            ],
        ),
        (
            "extend-to-eof",
            Vec::new(),
            BoxInfo::new(fourcc(), 0x0123)
                .with_header_size(SMALL_HEADER_SIZE)
                .with_extend_to_eof(true),
            BoxInfo::new(fourcc(), 0x0123)
                .with_header_size(SMALL_HEADER_SIZE)
                .with_extend_to_eof(true),
            vec![0x00, 0x00, 0x00, 0x00, b't', b'e', b's', b't'],
        ),
        (
            "offset",
            vec![0x00, 0x00, 0x00],
            BoxInfo::new(fourcc(), 0x0001_2345).with_header_size(SMALL_HEADER_SIZE),
            BoxInfo::new(fourcc(), 0x0001_2345)
                .with_offset(3)
                .with_header_size(SMALL_HEADER_SIZE),
            vec![
                0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, b't', b'e', b's', b't',
            ],
        ),
    ];

    for (name, prefix, info, expected_info, expected_bytes) in cases {
        let mut cursor = Cursor::new(prefix);
        cursor.seek(SeekFrom::End(0)).unwrap();

        let written = info.write(&mut cursor).unwrap();
        assert_eq!(written, expected_info, "{name}");
        assert_eq!(cursor.into_inner(), expected_bytes, "{name}");
    }
}

#[test]
fn read_decodes_small_large_and_eof_headers() {
    let cases = [
        (
            "small",
            vec![0x00, 0x01, 0x23, 0x45, b't', b'e', b's', b't'],
            0,
            BoxInfo::new(fourcc(), 0x0001_2345).with_header_size(SMALL_HEADER_SIZE),
            SMALL_HEADER_SIZE,
        ),
        (
            "offset",
            vec![
                0x00, 0x00, 0x00, 0x00, 0x01, 0x23, 0x45, b't', b'e', b's', b't',
            ],
            3,
            BoxInfo::new(fourcc(), 0x0001_2345)
                .with_offset(3)
                .with_header_size(SMALL_HEADER_SIZE),
            11,
        ),
        (
            "large",
            vec![
                0x00, 0x00, 0x00, 0x01, b't', b'e', b's', b't', 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef,
            ],
            0,
            BoxInfo::new(fourcc(), 0x0123_4567_89ab_cdef).with_header_size(LARGE_HEADER_SIZE),
            LARGE_HEADER_SIZE,
        ),
        (
            "extend-to-eof",
            vec![
                0x00, 0x00, 0x00, 0x00, b't', b'e', b's', b't', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
            0,
            BoxInfo::new(fourcc(), 20)
                .with_header_size(SMALL_HEADER_SIZE)
                .with_extend_to_eof(true),
            SMALL_HEADER_SIZE,
        ),
    ];

    for (name, bytes, seek, expected, expected_position) in cases {
        let mut cursor = Cursor::new(bytes);
        cursor.seek(SeekFrom::Start(seek)).unwrap();

        let info = BoxInfo::read(&mut cursor).unwrap();
        assert_eq!(info, expected, "{name}");
        assert_eq!(
            cursor.stream_position().unwrap(),
            expected_position,
            "{name}"
        );
    }
}

#[test]
fn read_rejects_large_headers_with_zero_extended_size() {
    let mut cursor = Cursor::new(vec![
        0x00, 0x00, 0x00, 0x01, b't', b'e', b's', b't', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ]);

    let error = BoxInfo::read(&mut cursor).unwrap_err();
    assert!(matches!(error, HeaderError::InvalidSize));
}

#[test]
fn seek_helpers_follow_box_boundaries() {
    let info = BoxInfo::new(fourcc(), 40)
        .with_offset(10)
        .with_header_size(LARGE_HEADER_SIZE);
    assert_eq!(info.header_form(), HeaderForm::Large);

    let mut cursor = Cursor::new(vec![0_u8; 64]);
    assert_eq!(info.seek_to_start(&mut cursor).unwrap(), 10);
    assert_eq!(info.seek_to_payload(&mut cursor).unwrap(), 26);
    assert_eq!(info.seek_to_end(&mut cursor).unwrap(), 50);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_write_and_read_preserve_offsets_and_header_forms() {
    let cases = [
        (
            "small",
            Vec::new(),
            BoxInfo::new(fourcc(), 0x0001_2345).with_header_size(SMALL_HEADER_SIZE),
            BoxInfo::new(fourcc(), 0x0001_2345).with_header_size(SMALL_HEADER_SIZE),
            vec![0x00, 0x01, 0x23, 0x45, b't', b'e', b's', b't'],
        ),
        (
            "large",
            Vec::new(),
            BoxInfo::new(fourcc(), 0x0000_1234_5678_9abc).with_header_size(SMALL_HEADER_SIZE),
            BoxInfo::new(fourcc(), 0x0000_1234_5678_9ac4).with_header_size(LARGE_HEADER_SIZE),
            vec![
                0x00, 0x00, 0x00, 0x01, b't', b'e', b's', b't', 0x00, 0x00, 0x12, 0x34, 0x56, 0x78,
                0x9a, 0xbc,
            ],
        ),
        (
            "extend-to-eof",
            Vec::new(),
            BoxInfo::new(fourcc(), 0x0123)
                .with_header_size(SMALL_HEADER_SIZE)
                .with_extend_to_eof(true),
            BoxInfo::new(fourcc(), 0x0123)
                .with_header_size(SMALL_HEADER_SIZE)
                .with_extend_to_eof(true),
            vec![0x00, 0x00, 0x00, 0x00, b't', b'e', b's', b't'],
        ),
    ];

    for (name, prefix, info, expected_info, expected_bytes) in cases {
        let mut cursor = Cursor::new(prefix);
        cursor.seek(SeekFrom::End(0)).unwrap();

        let written = info.write_async(&mut cursor).await.unwrap();
        assert_eq!(written, expected_info, "{name}");
        assert_eq!(cursor.into_inner(), expected_bytes, "{name}");
    }

    let read_cases = [
        (
            "small",
            vec![0x00, 0x01, 0x23, 0x45, b't', b'e', b's', b't'],
            0,
            BoxInfo::new(fourcc(), 0x0001_2345).with_header_size(SMALL_HEADER_SIZE),
            SMALL_HEADER_SIZE,
        ),
        (
            "large",
            vec![
                0x00, 0x00, 0x00, 0x01, b't', b'e', b's', b't', 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef,
            ],
            0,
            BoxInfo::new(fourcc(), 0x0123_4567_89ab_cdef).with_header_size(LARGE_HEADER_SIZE),
            LARGE_HEADER_SIZE,
        ),
        (
            "extend-to-eof",
            vec![
                0x00, 0x00, 0x00, 0x00, b't', b'e', b's', b't', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
            0,
            BoxInfo::new(fourcc(), 20)
                .with_header_size(SMALL_HEADER_SIZE)
                .with_extend_to_eof(true),
            SMALL_HEADER_SIZE,
        ),
    ];

    for (name, bytes, seek, expected, expected_position) in read_cases {
        let mut cursor = Cursor::new(bytes);
        cursor.seek(SeekFrom::Start(seek)).unwrap();

        let info = BoxInfo::read_async(&mut cursor).await.unwrap();
        assert_eq!(info, expected, "{name}");
        assert_eq!(
            cursor.stream_position().unwrap(),
            expected_position,
            "{name}"
        );
    }
}

#[cfg(feature = "async")]
#[tokio::test]
async fn async_seek_helpers_follow_box_boundaries() {
    let info = BoxInfo::new(fourcc(), 40)
        .with_offset(10)
        .with_header_size(LARGE_HEADER_SIZE);

    let mut cursor = Cursor::new(vec![0_u8; 64]);
    assert_eq!(info.seek_to_start_async(&mut cursor).await.unwrap(), 10);
    assert_eq!(info.seek_to_payload_async(&mut cursor).await.unwrap(), 26);
    assert_eq!(info.seek_to_end_async(&mut cursor).await.unwrap(), 50);
}

fn fourcc() -> FourCc {
    FourCc::try_from("test").unwrap()
}
