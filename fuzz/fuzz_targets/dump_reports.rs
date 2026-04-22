#![no_main]

mod support;

use std::collections::BTreeSet;
use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mp4forge::FourCc;
use mp4forge::cli::dump::{
    DumpOptions, StructuredDumpFormat, build_field_structured_report_paths,
    build_structured_report_paths, dump_reader_field_structured_paths,
    dump_reader_structured_paths, write_field_structured_report, write_structured_report,
};
use mp4forge::walk::BoxPath;

use support::{FuzzInput, seeded_small_mp4_bytes};

const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const FREE: FourCc = FourCc::from_bytes(*b"free");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const MDHD: FourCc = FourCc::from_bytes(*b"mdhd");
const MDIA: FourCc = FourCc::from_bytes(*b"mdia");
const MINF: FourCc = FourCc::from_bytes(*b"minf");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const MVHD: FourCc = FourCc::from_bytes(*b"mvhd");
const PSSH: FourCc = FourCc::from_bytes(*b"pssh");
const SKIP: FourCc = FourCc::from_bytes(*b"skip");
const STBL: FourCc = FourCc::from_bytes(*b"stbl");
const STSD: FourCc = FourCc::from_bytes(*b"stsd");
const TFDT: FourCc = FourCc::from_bytes(*b"tfdt");
const TRAF: FourCc = FourCc::from_bytes(*b"traf");
const TRAK: FourCc = FourCc::from_bytes(*b"trak");
const TRUN: FourCc = FourCc::from_bytes(*b"trun");

const FULL_BOX_TYPES: [FourCc; 10] = [FTYP, FREE, MDAT, MDHD, MVHD, PSSH, SKIP, STSD, TFDT, TRUN];

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let bytes = seeded_small_mp4_bytes(&mut input);
    let options = take_dump_options(&mut input);
    let paths = take_dump_paths(&mut input);
    let format = take_dump_format(&mut input);

    if let Ok(report) =
        build_structured_report_paths(&mut Cursor::new(bytes.as_slice()), &options, &paths)
    {
        let mut rendered = Vec::new();
        let _ = write_structured_report(&mut rendered, &report, format);
    }

    if let Ok(report) =
        build_field_structured_report_paths(&mut Cursor::new(bytes.as_slice()), &options, &paths)
    {
        let mut rendered = Vec::new();
        let _ = write_field_structured_report(&mut rendered, &report, format);
    }

    let mut structured_output = Vec::new();
    let _ = dump_reader_structured_paths(
        &mut Cursor::new(bytes.as_slice()),
        &options,
        &paths,
        format,
        &mut structured_output,
    );

    let mut field_output = Vec::new();
    let _ = dump_reader_field_structured_paths(
        &mut Cursor::new(bytes.as_slice()),
        &options,
        &paths,
        format,
        &mut field_output,
    );
});

fn take_dump_options(input: &mut FuzzInput<'_>) -> DumpOptions {
    let mut full_box_types = BTreeSet::new();
    for _ in 0..input.take_usize(4) {
        full_box_types.insert(input.choose_fourcc(&FULL_BOX_TYPES));
    }

    DumpOptions {
        full_box_types,
        show_all: input.take_bool(),
        show_offset: input.take_bool(),
        hex: input.take_bool(),
        terminal_width: input.take_usize(240).max(16),
    }
}

fn take_dump_paths(input: &mut FuzzInput<'_>) -> Vec<BoxPath> {
    let known_paths = vec![
        BoxPath::empty(),
        BoxPath::from([FTYP]),
        BoxPath::from([MOOV]),
        BoxPath::from([MOOV, MVHD]),
        BoxPath::from([MOOV, TRAK]),
        BoxPath::from([MOOV, TRAK, MDIA]),
        BoxPath::from([MOOV, FourCc::ANY, MDIA]),
        BoxPath::from([MOOV, TRAK, MDIA, MINF, STBL, STSD]),
        BoxPath::from([MOOV, TRAK, MDIA, MINF, STBL, FourCc::ANY]),
        BoxPath::from([MOOF]),
        BoxPath::from([MOOF, TRAF]),
        BoxPath::from([MOOF, TRAF, TFDT]),
        BoxPath::from([MOOF, TRAF, TRUN]),
    ];
    input.take_paths_from_table(&known_paths, 4)
}

fn take_dump_format(input: &mut FuzzInput<'_>) -> StructuredDumpFormat {
    if input.take_bool() {
        StructuredDumpFormat::Json
    } else {
        StructuredDumpFormat::Yaml
    }
}
