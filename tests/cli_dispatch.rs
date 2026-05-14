use mp4forge::cli;

#[test]
fn dispatch_prints_usage_for_empty_or_unknown_commands() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(cli::dispatch(&[], &mut stdout, &mut stderr), 1);
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), top_level_usage());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        cli::dispatch(&["unknown".to_string()], &mut stdout, &mut stderr),
        1
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), top_level_usage());
}

#[test]
fn dispatch_handles_help() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        cli::dispatch(&["help".to_string()], &mut stdout, &mut stderr),
        0
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), top_level_usage());
}

#[cfg(not(feature = "decrypt"))]
#[test]
fn dispatch_keeps_decrypt_unavailable_without_feature() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        cli::dispatch(&["decrypt".to_string()], &mut stdout, &mut stderr),
        1
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), top_level_usage());
}

#[cfg(not(feature = "mux"))]
#[test]
fn dispatch_keeps_mux_unavailable_without_feature() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        cli::dispatch(&["mux".to_string()], &mut stdout, &mut stderr),
        1
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(String::from_utf8(stderr).unwrap(), top_level_usage());
}

fn top_level_usage() -> String {
    let mut usage = String::from("USAGE: mp4forge COMMAND [ARGS]\n\nCOMMAND:\n");
    usage.push_str("  divide       split a fragmented MP4 into track playlists\n");
    #[cfg(feature = "decrypt")]
    usage.push_str("  decrypt      decrypt protected MP4-family content\n");
    usage.push_str("  dump         display the MP4 box tree\n");
    usage.push_str("  edit         rewrite selected boxes\n");
    usage.push_str("  extract      extract raw boxes by type or path\n");
    #[cfg(feature = "mux")]
    usage.push_str("  inspect      inspect one direct-ingest input without writing an MP4\n");
    #[cfg(feature = "mux")]
    usage.push_str("  mux          merge one video track plus audio tracks into one MP4\n");
    usage.push_str("  psshdump     summarize pssh boxes\n");
    usage.push_str("  probe        summarize an MP4 file\n");
    usage
}
