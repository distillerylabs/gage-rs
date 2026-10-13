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

#[test]
fn real_binary_exits_with_the_usage_code() {
    cargo_bin_cmd!("gage").arg("frobnicate").assert().code(2);
}

// Go: TestMain built the binary; cargo builds it for integration tests,
// so it is excused.
// Go: TestLinkTimeVersionInjection is `make stamp-check`, run in CI (a
// stamped rebuild inside `make test` would cost more than it proves on
// every run).
