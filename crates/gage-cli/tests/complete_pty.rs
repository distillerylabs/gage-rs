//! Tab completion over a real terminal: the one completion test that needs one.
//! Which candidates are right at a position is tested in-process
//! (`complete_tests` in the crate); this only proves a literal Tab
//! reaches that logic through `rustyline`, and that completing records
//! nothing in the history.
//!
//! A pty is a POSIX object, so Unix only (`linux || darwin`).
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin_cmd;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const PASS: &str = "correct horse battery staple";

/// The XDG roots every `gage` process in this test sees.
fn xdg(root: &std::path::Path) -> [(&'static str, PathBuf); 3] {
    [
        ("XDG_CONFIG_HOME", root.join("config")),
        ("XDG_DATA_HOME", root.join("data")),
        ("XDG_STATE_HOME", root.join("state")),
    ]
}

// A literal Tab
// expands `stat` to `status`, which then runs. It also re-asserts the
// rule on this path: a Tab, and a line abandoned after one, never reach
// the history; only the submitted lines do.
#[test]
fn tab_completion_at_the_prompt_expands_a_partial_command() {
    let root = tempfile::tempdir().unwrap();
    let init = cargo_bin_cmd!("gage")
        .args([
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--no-recovery-key",
        ])
        .envs(xdg(root.path()))
        .write_stdin(format!("{PASS}\n{PASS}\n"))
        .output()
        .unwrap();
    assert!(init.status.success(), "{init:?}");

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(assert_cmd::cargo::cargo_bin!("gage"));
    for (k, v) in xdg(root.path()) {
        cmd.env(k, v);
    }
    // A terminal the editor supports, whatever CI's TERM is.
    cmd.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);

    let screen = Arc::new(Mutex::new(String::new()));
    let mut reader = pair.master.try_clone_reader().unwrap();
    let sink = Arc::clone(&screen);
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    let seen = || screen.lock().unwrap().clone();
    let wait_for = |needle: &str, n: usize| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if seen().matches(needle).count() >= n {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("never saw {needle:?} x{n} on the terminal:\n{}", seen());
    };
    let mut writer = pair.master.take_writer().unwrap();
    let mut type_raw = |bytes: &[u8]| {
        writer.write_all(bytes).unwrap();
        writer.flush().unwrap();
    };
    // init made personal current, so it shows, locked, from the start. A
    // fresh prompt is the prompt followed by a carriage return.
    let fresh = "personal🔒 gage> \r";

    wait_for(fresh, 1);
    // `stat` is a prefix of exactly one session command, so one Tab
    // completes it in place rather than listing candidates.
    type_raw(b"stat\t");
    wait_for("personal🔒 gage> status", 1);
    type_raw(b"\r");
    wait_for("no vaults have been unlocked", 1);

    // A Tab, then Ctrl-C: nothing submitted, so nothing recorded.
    wait_for(fresh, 2);
    type_raw(b"loc\t");
    wait_for("personal🔒 gage> lock", 1);
    type_raw(b"\x03");
    wait_for(fresh, 3);
    type_raw(b"exit\r");

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "never exited:\n{}", seen());
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "session exit {status:?}:\n{}", seen());

    let history = std::fs::read_to_string(root.path().join("state/gage/history")).unwrap();
    assert_eq!(history, "status\nexit\n");
}
