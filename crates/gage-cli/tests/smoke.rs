//! Real-binary smoke tests. Everything else drives `gage_cli::run`
//! in-process; this file exists only for what needs an actual process: the
//! real `main`, its real stdin, and the stamped build info.

use assert_cmd::cargo::cargo_bin_cmd;

#[test]
fn real_binary_version_flag() {
    let out = cargo_bin_cmd!("gage").arg("--version").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.starts_with("gage "), "{stdout:?}");
    assert!(stdout.contains("(commit "), "{stdout:?}");
}

// Assert_cmd's stdin is
// not a terminal, so bare `gage` must print help and exit 0 rather than
// start a session.
#[test]
fn real_binary_with_redirected_stdin_prints_help() {
    let out = cargo_bin_cmd!("gage").write_stdin("").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Usage:\n  gage [command]"), "{stdout}");
}

// A property of how main wires
// the prompter, so it needs main: with stdin redirected, init's passphrase
// prompts land on stderr and stdout carries only init's report. (The
// in-process half, init through the real prompter, is in
// init_identity_tests.) This is also the one run of the shipped binary's
// real KDF cost: no test hook reaches it.
#[test]
fn prompts_go_to_the_error_stream_not_stdout() {
    let root = tempfile::tempdir().unwrap();
    let out = cargo_bin_cmd!("gage")
        .args([
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--no-recovery-key",
        ])
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

// The both-terminals rule needs main, which is what reads
// whether stdin and stderr are terminals. A piped default init, the run
// that would file a private key into a log, refuses with Usage before
// asking anything, and leaves gage's data dir as it found it.
#[test]
fn real_binary_init_with_piped_stdin_refuses_to_show_a_recovery_key() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let out = cargo_bin_cmd!("gage")
        .args(["init", "personal", "--device", "laptop-1"])
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_DATA_HOME", &data)
        .env("XDG_STATE_HOME", root.path().join("state"))
        .write_stdin("s3cret\ns3cret\n")
        .assert()
        .code(2);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("--recovery-key-out"), "{stderr}");
    assert!(stderr.contains("--no-recovery-key"), "{stderr}");
    assert!(
        !stderr.contains("Choose a passphrase"),
        "it asked first:\n{stderr}"
    );
    assert!(!stderr.contains("AGE-SECRET-KEY-1"), "{stderr}");
    let left: Vec<_> = std::fs::read_dir(&data)
        .map(|d| d.collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn real_binary_exits_with_the_usage_code() {
    cargo_bin_cmd!("gage").arg("frobnicate").assert().code(2);
}

// The link-time version stamp is `make stamp-check`, run in CI (a
// stamped rebuild inside `make test` would cost more than it proves on
// every run).
