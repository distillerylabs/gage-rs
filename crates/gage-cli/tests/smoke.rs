//! Real-binary smoke tests. Everything else drives `gage_cli::run`
//! in-process; this file exists only for what needs an actual process: the
//! real `main`, its real stdin, and the stamped build info.

use assert_cmd::cargo::cargo_bin_cmd;

// Go: TestRealBinaryVersionFlag
#[test]
fn real_binary_version_flag() {
    let out = cargo_bin_cmd!("gage").arg("--version").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.starts_with("gage "), "{stdout:?}");
    assert!(stdout.contains("(commit "), "{stdout:?}");
}

// Go: TestRealBinaryWithRedirectedStdinPrintsHelp. assert_cmd's stdin is
// not a terminal, so bare `gage` must print help and exit 0 rather than
// start a session.
#[test]
fn real_binary_with_redirected_stdin_prints_help() {
    let out = cargo_bin_cmd!("gage").write_stdin("").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Usage:\n  gage [command]"), "{stdout}");
}

// Go: TestPromptsGoToTheErrorStreamNotStdout. A property of how main wires
// the prompter, so it needs main: with stdin redirected, init's passphrase
// prompts land on stderr and stdout carries only init's report. (The
// in-process half, init through the real prompter, is in
// init_identity_tests.) This is also the one run of the shipped binary's
// real KDF cost: no test hook reaches it.
#[test]
fn prompts_go_to_the_error_stream_not_stdout() {
    let root = tempfile::tempdir().unwrap();
    let out = cargo_bin_cmd!("gage")
        .args(["init", "personal", "--device", "laptop-1"])
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("XDG_STATE_HOME", root.path().join("state"))
        .write_stdin("s3cret\ns3cret\n")
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("Choose a passphrase"), "{stderr}");
    assert!(stderr.contains("Confirm passphrase"), "{stderr}");
    assert!(
        !stdout.contains("passphrase"),
        "a prompt reached stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("initialized vault \"personal\""),
        "{stdout}"
    );
    assert!(!stdout.contains("s3cret") && !stderr.contains("s3cret"));
}

#[test]
fn real_binary_exits_with_the_usage_code() {
    cargo_bin_cmd!("gage").arg("frobnicate").assert().code(2);
}

// Go: TestMain built the binary; cargo builds it for integration tests,
// so it is excused.
// Go: TestLinkTimeVersionInjection is `make stamp-check`, run in CI (a
// stamped rebuild inside `make test` would cost more than it proves on
// every run).
