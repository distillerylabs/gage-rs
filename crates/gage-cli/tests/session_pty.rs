//! Session mode over a real terminal: the one M6 test that needs one.
//! Everything else about the REPL runs in-process through the plain line
//! reader (`repl_tests` in the crate), so without this the `rustyline`
//! half would ship untested: raw mode, and the prompter reading a masked
//! passphrase from the same terminal between two lines.
//!
//! A pty is a POSIX object, so Unix only, as Go's is (`linux || darwin`).
#![cfg(unix)]

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin_cmd;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const PASS: &str = "correct horse battery staple";
const VALUE: &str = "correcthorsebatterystaple-value";

/// The XDG roots every `gage` process in this test sees.
fn xdg(root: &std::path::Path) -> [(&'static str, PathBuf); 3] {
    [
        ("XDG_CONFIG_HOME", root.join("config")),
        ("XDG_DATA_HOME", root.join("data")),
        ("XDG_STATE_HOME", root.join("state")),
    ]
}

fn gage(root: &std::path::Path, args: &[&str], stdin: String) {
    let out = cargo_bin_cmd!("gage")
        .args(args)
        .envs(xdg(root))
        .write_stdin(stdin)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Whether the terminal at `tty` has echo on.
#[allow(unsafe_code)]
fn echo_on(tty: &std::fs::File) -> bool {
    // SAFETY: termios is plain data, filled in by tcgetattr on an fd this
    // test holds open.
    let lflag = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(tty.as_raw_fd(), &mut t), 0);
        t.c_lflag
    };
    lflag & libc::ECHO != 0
}

// Go: TestSessionOverARealTerminalDrivesTheLineEditor. Bare `gage` on a
// terminal reaches the line editor; a passphrase asked between two lines
// is read masked from the same terminal; Up recalls a line and Ctrl-C
// abandons one, which only the editor's raw mode does (cooked, the first
// is junk and the second kills the process); `exit` ends it 0.
#[test]
fn session_over_a_real_terminal_drives_the_line_editor() {
    let root = tempfile::tempdir().unwrap();
    gage(
        root.path(),
        &[
            "init",
            "personal",
            "--device",
            "laptop-1",
            "--no-recovery-key",
        ],
        format!("{PASS}\n{PASS}\n"),
    );
    gage(
        root.path(),
        &["insert", "ProtonMail", "--value-stdin"],
        format!("{PASS}\n{VALUE}"),
    );

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let tty_path = pair.master.tty_name().expect("a unix pty has a path");
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
    // Waits for the `n`th appearance of `needle`. Each line is typed only
    // once the editor is reading: a line typed between two reads is lost
    // with the first read's buffer, as type-ahead past Ctrl-C is in any
    // readline.
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
    // The editor draws a fresh, empty prompt as the prompt followed by a
    // carriage return; a redraw (a recalled line) has text after it, so
    // counting these counts prompts, not redraws.
    let locked = "personal🔒 gage> ";
    let unlocked = "personal🔓 gage> ";
    let fresh = |prompt: &str| format!("{prompt}\r");

    // init made personal current, so it shows, locked, from the start.
    wait_for(&fresh(locked), 1);
    type_raw(b"use personal\r");

    // Between lines the terminal is the prompter's: wait for it to turn
    // echo off before typing, rather than sleeping and hoping.
    wait_for("Enter passphrase", 1);
    let probe = std::fs::File::open(&tty_path).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while echo_on(&probe) {
        assert!(Instant::now() < deadline, "echo never went off");
        std::thread::sleep(Duration::from_millis(2));
    }
    type_raw(format!("{PASS}\n").as_bytes());

    wait_for(&fresh(unlocked), 1);
    type_raw(b"show ProtonMail\r");
    wait_for(VALUE, 1);

    // Up recalls the last line onto the prompt; Enter runs it again.
    wait_for(&fresh(unlocked), 2);
    type_raw(b"\x1b[A");
    wait_for(&format!("{unlocked}show ProtonMail"), 1);
    type_raw(b"\r");
    wait_for(VALUE, 2);

    // Ctrl-C mid-line drops the line and keeps the session.
    wait_for(&fresh(unlocked), 3);
    type_raw(b"show half-typed");
    wait_for("half-typed", 1);
    type_raw(b"\x03");
    wait_for(&fresh(unlocked), 4);
    type_raw(b"status\r");
    wait_for("last used", 1);

    wait_for(&fresh(unlocked), 5);
    type_raw(b"exit\r");

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "never exited:\n{}", seen());
        std::thread::sleep(Duration::from_millis(10));
    };
    let screen = seen();
    assert!(status.success(), "session exit {status:?}:\n{screen}");
    assert!(
        !screen.contains(PASS),
        "the passphrase was echoed:\n{screen}"
    );
    assert!(
        !screen.contains("unknown command"),
        "a line reached the dispatcher mangled:\n{screen}"
    );

    // And the history file holds the typed lines, the recalled one again,
    // and neither the passphrase nor the abandoned line.
    let roots = xdg(root.path());
    let env = |k: &str| {
        roots
            .iter()
            .find(|(name, _)| *name == k)
            .map(|(_, p)| p.to_string_lossy().into_owned())
    };
    let dirs =
        gage::xdgpaths::Dirs::resolve(gage::xdgpaths::Os::current(), &env, root.path()).unwrap();
    let history = std::fs::read_to_string(dirs.state.join("history")).unwrap();
    assert_eq!(
        history,
        "use personal\nshow ProtonMail\nshow ProtonMail\nstatus\nexit\n"
    );
}
