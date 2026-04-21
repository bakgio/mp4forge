use mp4forge::cli;

#[test]
fn dispatch_prints_usage_for_empty_or_unknown_commands() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(cli::dispatch(&[], &mut stdout, &mut stderr), 1);
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "USAGE: mp4forge COMMAND [ARGS]\n",
            "\n",
            "COMMAND:\n",
            "  divide       split a fragmented MP4 into track playlists\n",
            "  dump         display the MP4 box tree\n",
            "  edit         rewrite selected boxes\n",
            "  extract      extract raw boxes by type\n",
            "  psshdump     summarize pssh boxes\n",
            "  probe        summarize an MP4 file\n"
        )
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        cli::dispatch(&["unknown".to_string()], &mut stdout, &mut stderr),
        1
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), "");
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "USAGE: mp4forge COMMAND [ARGS]\n",
            "\n",
            "COMMAND:\n",
            "  divide       split a fragmented MP4 into track playlists\n",
            "  dump         display the MP4 box tree\n",
            "  edit         rewrite selected boxes\n",
            "  extract      extract raw boxes by type\n",
            "  psshdump     summarize pssh boxes\n",
            "  probe        summarize an MP4 file\n"
        )
    );
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
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        concat!(
            "USAGE: mp4forge COMMAND [ARGS]\n",
            "\n",
            "COMMAND:\n",
            "  divide       split a fragmented MP4 into track playlists\n",
            "  dump         display the MP4 box tree\n",
            "  edit         rewrite selected boxes\n",
            "  extract      extract raw boxes by type\n",
            "  psshdump     summarize pssh boxes\n",
            "  probe        summarize an MP4 file\n"
        )
    );
}
