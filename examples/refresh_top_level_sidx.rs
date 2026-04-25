use std::env;
use std::error::Error;
use std::io::Cursor;

use mp4forge::sidx::{
    TopLevelSidxPlanAction, TopLevelSidxPlanOptions, apply_top_level_sidx_plan,
    plan_top_level_sidx_update_bytes,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let (input_path, output_path, non_zero_ept) = match args.as_slice() {
        [input_path, output_path] => (input_path.as_str(), output_path.as_str(), false),
        [input_path, output_path, flag] if flag == "--non-zero-ept" => {
            (input_path.as_str(), output_path.as_str(), true)
        }
        _ => {
            return Err(
                "usage: cargo run --example refresh_top_level_sidx -- <input.mp4> <output.mp4> [--non-zero-ept]"
                    .into(),
            );
        }
    };

    let input = std::fs::read(input_path)?;
    let Some(plan) = plan_top_level_sidx_update_bytes(
        &input,
        TopLevelSidxPlanOptions {
            add_if_not_exists: true,
            non_zero_ept,
        },
    )?
    else {
        return Err("no top-level sidx change was needed".into());
    };

    let action = match &plan.action {
        TopLevelSidxPlanAction::Insert => "inserted",
        TopLevelSidxPlanAction::Replace { .. } => "updated",
    };

    let mut output = Vec::with_capacity(input.len().saturating_add(plan.encoded_box_size as usize));
    let applied = apply_top_level_sidx_plan(&mut Cursor::new(&input), &mut output, &plan)?;
    std::fs::write(output_path, &output)?;

    println!(
        "{action} top-level sidx at offset {} with {} references",
        applied.info.offset(),
        applied.sidx.reference_count
    );

    Ok(())
}
