#![no_main]

mod support;

use std::collections::BTreeSet;
use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::{Ftyp, Mvhd, Tfdt};
use mp4forge::cli::dump::{DumpOptions, build_field_structured_report};
use mp4forge::cli::edit::{EditOptions, edit_reader};
use mp4forge::codec::ImmutableBox;
use mp4forge::probe::{ProbeOptions, probe_with_options};
use mp4forge::rewrite::{rewrite_box_as_bytes, rewrite_boxes_as_bytes};
use mp4forge::walk::{BoxPath, WalkControl, walk_structure};

use support::{FuzzInput, seeded_rewrite_mp4_bytes};

const FREE: FourCc = FourCc::from_bytes(*b"free");
const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const MDAT: FourCc = FourCc::from_bytes(*b"mdat");
const MDHD: FourCc = FourCc::from_bytes(*b"mdhd");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const MVHD: FourCc = FourCc::from_bytes(*b"mvhd");
const PSSH: FourCc = FourCc::from_bytes(*b"pssh");
const SKIP: FourCc = FourCc::from_bytes(*b"skip");
const TFDT: FourCc = FourCc::from_bytes(*b"tfdt");
const TRAF: FourCc = FourCc::from_bytes(*b"traf");
const TRAK: FourCc = FourCc::from_bytes(*b"trak");
const TRUN: FourCc = FourCc::from_bytes(*b"trun");

const DROP_BOX_TYPES: [FourCc; 6] = [FREE, MDAT, MDHD, PSSH, SKIP, TRUN];

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let bytes = seeded_rewrite_mp4_bytes(&mut input);

    match input.take_u8() % 3 {
        0 => exercise_ftyp_rewrite(&mut input, &bytes),
        1 => exercise_mvhd_rewrite(&mut input, &bytes),
        _ => exercise_tfdt_rewrite(&mut input, &bytes),
    }

    exercise_edit_flow(&mut input, &bytes);
});

fn exercise_ftyp_rewrite(input: &mut FuzzInput<'_>, bytes: &[u8]) {
    let known_paths = vec![
        BoxPath::empty(),
        BoxPath::from([FTYP]),
        BoxPath::from([MOOV]),
        BoxPath::from([MOOV, MVHD]),
        BoxPath::from([MOOF, TRAF, TFDT]),
    ];
    let path = input.take_path_from_table(&known_paths);
    if let Ok(rewritten) = rewrite_box_as_bytes::<Ftyp, _>(bytes, path, |ftyp| {
        ftyp.major_brand = input.take_fourcc();
        ftyp.minor_version ^= input.take_u32();
        ftyp.add_compatible_brand(input.take_fourcc());
        if input.take_bool() && !ftyp.compatible_brands.is_empty() {
            let brand = ftyp.compatible_brands[input.take_usize(ftyp.compatible_brands.len() - 1)];
            ftyp.remove_compatible_brand(brand);
        }
    }) {
        exercise_rewritten_bytes(&rewritten);
    }
}

fn exercise_mvhd_rewrite(input: &mut FuzzInput<'_>, bytes: &[u8]) {
    let known_paths = vec![
        BoxPath::empty(),
        BoxPath::from([FTYP]),
        BoxPath::from([MOOV]),
        BoxPath::from([MOOV, MVHD]),
        BoxPath::from([MOOV, TRAK]),
    ];
    let paths = input.take_paths_from_table(&known_paths, 3);
    if let Ok(rewritten) = rewrite_boxes_as_bytes::<Mvhd, _>(bytes, &paths, |mvhd| {
        mvhd.timescale = input.take_u32().max(1);
        if mvhd.version() == 0 {
            mvhd.duration_v0 = input.take_u32();
        } else {
            mvhd.duration_v1 = input.take_u64();
        }
        mvhd.next_track_id = input.take_u32();
    }) {
        exercise_rewritten_bytes(&rewritten);
    }
}

fn exercise_tfdt_rewrite(input: &mut FuzzInput<'_>, bytes: &[u8]) {
    let known_paths = vec![
        BoxPath::empty(),
        BoxPath::from([MOOV, MVHD]),
        BoxPath::from([MOOF]),
        BoxPath::from([MOOF, TRAF]),
        BoxPath::from([MOOF, TRAF, TFDT]),
        BoxPath::from([MOOF, FourCc::ANY, TFDT]),
    ];
    let paths = input.take_paths_from_table(&known_paths, 4);
    let decode_time = input.take_u64();
    if let Ok(rewritten) = rewrite_boxes_as_bytes::<Tfdt, _>(bytes, &paths, |tfdt| {
        if tfdt.version() == 0 {
            tfdt.base_media_decode_time_v0 = decode_time as u32;
        } else {
            tfdt.base_media_decode_time_v1 = decode_time;
        }
    }) {
        exercise_rewritten_bytes(&rewritten);
    }
}

fn exercise_edit_flow(input: &mut FuzzInput<'_>, bytes: &[u8]) {
    let mut drop_boxes = BTreeSet::new();
    for _ in 0..input.take_usize(4) {
        drop_boxes.insert(input.choose_fourcc(&DROP_BOX_TYPES));
    }

    let options = EditOptions {
        base_media_decode_time: if input.take_bool() {
            Some(input.take_u64())
        } else {
            None
        },
        drop_boxes,
    };

    let mut rewritten = Cursor::new(Vec::new());
    if edit_reader(&mut Cursor::new(bytes), &mut rewritten, &options).is_ok() {
        exercise_rewritten_bytes(rewritten.get_ref().as_slice());
    }
}

fn exercise_rewritten_bytes(bytes: &[u8]) {
    let _ = probe_with_options(&mut Cursor::new(bytes), ProbeOptions::lightweight());
    let _ = build_field_structured_report(&mut Cursor::new(bytes), &DumpOptions::default());
    let _ = walk_structure(&mut Cursor::new(bytes), |handle| {
        Ok(if handle.is_supported_type() {
            WalkControl::Descend
        } else {
            WalkControl::Continue
        })
    });
}
