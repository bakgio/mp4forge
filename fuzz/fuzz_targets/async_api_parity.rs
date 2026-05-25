#![no_main]

mod support;

use std::io::Cursor;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use mp4forge::FourCc;
use mp4forge::boxes::iso14496_12::{Ftyp, Tfdt};
use mp4forge::extract::{extract_box_async, extract_boxes_async};
use mp4forge::probe::{
    ProbeOptions, probe_async, probe_detailed_async, probe_fra_async, probe_with_options_async,
};
use mp4forge::rewrite::rewrite_box_as_bytes_async;
use mp4forge::sidx::{
    TopLevelSidxPlanOptions, analyze_top_level_sidx_update_async, apply_top_level_sidx_plan_async,
    plan_top_level_sidx_update_async,
};
use mp4forge::walk::BoxPath;
use tokio::runtime::{Builder, Runtime};

use support::{FuzzInput, seeded_small_mp4_bytes};

const FTYP: FourCc = FourCc::from_bytes(*b"ftyp");
const MOOF: FourCc = FourCc::from_bytes(*b"moof");
const MOOV: FourCc = FourCc::from_bytes(*b"moov");
const TFDT: FourCc = FourCc::from_bytes(*b"tfdt");
const TRAF: FourCc = FourCc::from_bytes(*b"traf");

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let bytes = seeded_small_mp4_bytes(&mut input);
    let options = TopLevelSidxPlanOptions {
        add_if_not_exists: input.take_bool(),
        non_zero_ept: input.take_bool(),
    };

    runtime().block_on(async {
        let _ = probe_async(&mut Cursor::new(bytes.clone())).await;
        let _ = probe_with_options_async(
            &mut Cursor::new(bytes.clone()),
            ProbeOptions {
                expand_samples: input.take_bool(),
                expand_chunks: input.take_bool(),
                include_segments: input.take_bool(),
            },
        )
        .await;
        let _ = probe_detailed_async(&mut Cursor::new(bytes.clone())).await;
        let _ = probe_fra_async(&mut Cursor::new(bytes.clone())).await;

        let path = take_path(&mut input);
        let _ = extract_box_async(&mut Cursor::new(bytes.clone()), None, path.clone()).await;
        let _ = extract_boxes_async(&mut Cursor::new(bytes.clone()), None, &[path]).await;

        let _ = rewrite_box_as_bytes_async::<Ftyp, _>(&bytes, BoxPath::from([FTYP]), |ftyp| {
            ftyp.minor_version ^= input.take_u32();
        })
        .await;
        let _ = rewrite_box_as_bytes_async::<Tfdt, _>(
            &bytes,
            BoxPath::from([MOOF, TRAF, TFDT]),
            |tfdt| {
                tfdt.base_media_decode_time_v0 ^= input.take_u32();
                tfdt.base_media_decode_time_v1 ^= input.take_u64();
            },
        )
        .await;

        let _ = analyze_top_level_sidx_update_async(&mut Cursor::new(bytes.clone())).await;
        if let Ok(Some(plan)) =
            plan_top_level_sidx_update_async(&mut Cursor::new(bytes.clone()), options).await
        {
            let mut output = Cursor::new(Vec::new());
            let _ =
                apply_top_level_sidx_plan_async(&mut Cursor::new(bytes), &mut output, &plan).await;
        }
    });
});

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build fuzz Tokio runtime")
    })
}

fn take_path(input: &mut FuzzInput<'_>) -> BoxPath {
    match input.take_u8() % 5 {
        0 => BoxPath::empty(),
        1 => BoxPath::from([FTYP]),
        2 => BoxPath::from([MOOV]),
        3 => BoxPath::from([MOOF, TRAF]),
        _ => BoxPath::from([MOOF, TRAF, TFDT]),
    }
}
