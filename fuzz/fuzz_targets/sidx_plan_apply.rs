#![no_main]

mod support;

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mp4forge::probe::{ProbeOptions, probe_with_options};
use mp4forge::sidx::{
    TopLevelSidxPlanOptions, analyze_top_level_sidx_update_bytes, apply_top_level_sidx_plan_bytes,
    plan_top_level_sidx_update_bytes,
};
use mp4forge::walk::{WalkControl, walk_structure};

use support::{FuzzInput, seeded_any_mp4_bytes};

fuzz_target!(|data: &[u8]| {
    let mut input = FuzzInput::new(data);
    let bytes = seeded_any_mp4_bytes(&mut input);
    let options = TopLevelSidxPlanOptions {
        add_if_not_exists: input.take_bool(),
        non_zero_ept: input.take_bool(),
    };

    let _ = analyze_top_level_sidx_update_bytes(&bytes);
    if let Ok(Some(plan)) = plan_top_level_sidx_update_bytes(&bytes, options)
        && let Ok(rewritten) = apply_top_level_sidx_plan_bytes(&bytes, &plan)
    {
        let _ = analyze_top_level_sidx_update_bytes(&rewritten);
        let _ = probe_with_options(&mut Cursor::new(&rewritten), ProbeOptions::lightweight());
        let _ = walk_structure(&mut Cursor::new(&rewritten), |handle| {
            Ok(if handle.is_supported_type() {
                WalkControl::Descend
            } else {
                WalkControl::Continue
            })
        });
    }
});
