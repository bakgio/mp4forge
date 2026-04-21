use std::io::{self, Cursor, Seek, SeekFrom, Write};

use mp4forge::boxes::iso14496_12::{Ftyp, Tkhd};
use mp4forge::codec::marshal;
use mp4forge::writer::{Writer, WriterError};
use mp4forge::{BoxInfo, FourCc};

#[test]
fn writer_backfills_sizes_and_copies_boxes() {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    let info = writer.start_box_type(fourcc("ftyp")).unwrap();
    assert_eq!(info.offset(), 0);
    assert_eq!(info.size(), 8);

    let mut ftyp = Ftyp {
        major_brand: fourcc("abem"),
        minor_version: 0x1234_5678,
        compatible_brands: vec![fourcc("abcd"), fourcc("efgh")],
    };
    marshal(&mut writer, &ftyp, None).unwrap();

    let info = writer.end_box().unwrap();
    assert_eq!(info.offset(), 0);
    assert_eq!(info.size(), 24);

    let info = writer.start_box_type(fourcc("moov")).unwrap();
    assert_eq!(info.offset(), 24);
    assert_eq!(info.size(), 8);

    writer
        .copy_box(
            &mut Cursor::new(vec![
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0a, b'u', b'd', b't', b'a',
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            ]),
            &BoxInfo::new(fourcc("udta"), 15).with_offset(6),
        )
        .unwrap();

    let info = writer.start_box_type(fourcc("trak")).unwrap();
    assert_eq!(info.offset(), 47);
    assert_eq!(info.size(), 8);

    let info = writer.start_box_type(fourcc("tkhd")).unwrap();
    assert_eq!(info.offset(), 55);
    assert_eq!(info.size(), 8);

    let tkhd = sample_tkhd();
    marshal(&mut writer, &tkhd, None).unwrap();

    let info = writer.end_box().unwrap();
    assert_eq!(info.offset(), 55);
    assert_eq!(info.size(), 92);

    let info = writer.end_box().unwrap();
    assert_eq!(info.offset(), 47);
    assert_eq!(info.size(), 100);

    let info = writer.end_box().unwrap();
    assert_eq!(info.offset(), 24);
    assert_eq!(info.size(), 123);

    writer.seek(SeekFrom::Start(8)).unwrap();
    ftyp.compatible_brands[1] = fourcc("EFGH");
    marshal(&mut writer, &ftyp, None).unwrap();

    let mut expected = vec![
        0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p', b'a', b'b', b'e', b'm', 0x12, 0x34, 0x56,
        0x78, b'a', b'b', b'c', b'd', b'E', b'F', b'G', b'H', 0x00, 0x00, 0x00, 0x7b, b'm', b'o',
        b'o', b'v', 0x00, 0x00, 0x00, 0x0a, b'u', b'd', b't', b'a', 0x01, 0x02, 0x03, 0x04, 0x05,
        0x06, 0x07, 0x00, 0x00, 0x00, 0x64, b't', b'r', b'a', b'k', 0x00, 0x00, 0x00, 0x5c, b't',
        b'k', b'h', b'd', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02,
        0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x06, 0x00, 0x07, 0x00, 0x00,
    ];
    expected.extend_from_slice(&[0x00; 36]);
    expected.extend_from_slice(&[0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x09]);

    assert_eq!(writer.into_inner().into_inner(), expected);
}

#[test]
fn end_box_rejects_empty_stack() {
    let error = Writer::new(Cursor::new(Vec::<u8>::new()))
        .end_box()
        .unwrap_err();
    assert!(matches!(error, WriterError::NoOpenBox));
}

#[test]
fn copy_box_rejects_short_source() {
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let error = writer
        .copy_box(
            &mut Cursor::new(vec![0x00, 0x00, 0x00, 0x08, b'f', b'r', b'e', b'e']),
            &BoxInfo::new(fourcc("free"), 12),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        WriterError::IncompleteCopy {
            expected_size: 12,
            actual_size: 8
        }
    ));
}

#[test]
fn end_box_rejects_header_size_changes() {
    let mut writer = Writer::new(SparseBuffer::default());
    writer.start_box_type(fourcc("wide")).unwrap();
    writer
        .seek(SeekFrom::Start(u64::from(u32::MAX) + 1))
        .unwrap();

    let error = writer.end_box().unwrap_err();
    assert!(matches!(
        error,
        WriterError::HeaderSizeChanged {
            box_type,
            original_header_size: 8,
            rewritten_header_size: 16
        } if box_type == fourcc("wide")
    ));
}

fn fourcc(value: &str) -> FourCc {
    FourCc::try_from(value).unwrap()
}

#[allow(clippy::field_reassign_with_default)]
fn sample_tkhd() -> Tkhd {
    let mut tkhd = Tkhd::default();
    tkhd.creation_time_v0 = 1;
    tkhd.modification_time_v0 = 2;
    tkhd.track_id = 3;
    tkhd.duration_v0 = 4;
    tkhd.layer = 5;
    tkhd.alternate_group = 6;
    tkhd.volume = 7;
    tkhd.width = 8;
    tkhd.height = 9;
    tkhd
}

#[derive(Default)]
struct SparseBuffer {
    position: u64,
    len: u64,
}

impl Write for SparseBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.position = self
            .position
            .checked_add(buf.len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "position overflow"))?;
        self.len = self.len.max(self.position);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for SparseBuffer {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.len) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
        };

        if !(0..=i128::from(u64::MAX)).contains(&next) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"));
        }

        let next = next as u64;
        self.position = next;
        self.len = self.len.max(next);
        Ok(next)
    }
}
