use std::collections::BTreeSet;
use std::io::Cursor;

use mp4forge::FourCc;
use mp4forge::cli::edit::{EditError, EditOptions, edit_reader};
use mp4forge::codec::CodecError;
use mp4forge::extract::{ExtractError, extract_box_with_payload};
use mp4forge::header::{BoxInfo, HeaderError};
use mp4forge::walk::{BoxPath, WalkControl, WalkError, walk_structure};

#[test]
fn walk_structure_rejects_truncated_child_headers() {
    let mut bytes = BoxInfo::new(fourcc("moov"), 16).encode();
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x0c]);

    let error = walk_structure(&mut Cursor::new(bytes), |handle| {
        Ok(if handle.info().box_type() == fourcc("moov") {
            WalkControl::Descend
        } else {
            WalkControl::Continue
        })
    })
    .unwrap_err();

    assert!(matches!(
        error,
        WalkError::Header(HeaderError::Io(ref io_error))
            if io_error.kind() == std::io::ErrorKind::UnexpectedEof
    ));
}

#[test]
fn walk_structure_rejects_huge_declared_supported_payloads_without_preallocating_them() {
    let mut bytes = BoxInfo::new(fourcc("styp"), u64::from(u32::MAX)).encode();
    bytes.extend_from_slice(b"isom");
    bytes.extend_from_slice(&0_u32.to_be_bytes());

    let error = walk_structure(&mut Cursor::new(bytes), |handle| {
        Ok(if handle.is_supported_type() {
            let _ = handle.read_payload()?;
            WalkControl::Descend
        } else {
            WalkControl::Continue
        })
    })
    .unwrap_err();

    assert!(matches!(
        error,
        WalkError::Codec(CodecError::Io(ref io_error))
            if io_error.kind() == std::io::ErrorKind::UnexpectedEof
    ));
}

#[test]
fn edit_reader_rejects_children_that_exceed_container_bounds() {
    let mut bytes = BoxInfo::new(fourcc("moov"), 16).encode();
    bytes.extend_from_slice(&BoxInfo::new(fourcc("free"), 12).encode());

    let options = EditOptions {
        base_media_decode_time: Some(1),
        drop_boxes: BTreeSet::new(),
    };
    let error =
        edit_reader(&mut Cursor::new(bytes), Cursor::new(Vec::new()), &options).unwrap_err();

    assert!(matches!(
        error,
        EditError::TooLargeBoxSize {
            box_type,
            size: 12,
            available_size: 8
        } if box_type == fourcc("free")
    ));
}

#[test]
fn extract_box_with_payload_rejects_truncated_supported_payloads() {
    let mut bytes = BoxInfo::new(fourcc("mvhd"), 12).encode();
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    let error = match extract_box_with_payload(
        &mut Cursor::new(bytes),
        None,
        BoxPath::from([fourcc("mvhd")]),
    ) {
        Ok(_) => panic!("expected truncated payload error"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ExtractError::PayloadDecode {
            path,
            box_type,
            offset: 0,
            source: CodecError::Io(ref io_error)
        }
            if path.as_slice() == [fourcc("mvhd")]
                && box_type == fourcc("mvhd")
                && io_error.kind() == std::io::ErrorKind::UnexpectedEof
    ));
}

fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).unwrap()
}
