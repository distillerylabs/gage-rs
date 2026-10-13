//! `insert`, `cat`, `rm` and `ls` through the real `gage` binary: what only
//! a real process can show. Two processes contending for a vault, and a
//! real terminal feeding `insert -m`. Everything else about these commands
//! is tested in-process (`entry_tests` in the crate).
//!
//! These run the shipped binary, so every unlock pays the real scrypt cost
//! (a second or so): no test hook reaches another process.

use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;

const PASS: &str = "correct horse battery staple";

/// One isolated machine: its own config, data and state roots, handed to
/// every `gage` process through the XDG variables.
struct Machine {
    root: tempfile::TempDir,
}

impl Machine {
    /// A machine with one initialized vault named `personal`.
    fn with_vault() -> Machine {
        let m = Machine {
            root: tempfile::tempdir().unwrap(),
        };
        let out = m
            .gage(&["init", "personal", "--device", "laptop-1"])
            .write_stdin(format!("{PASS}\n{PASS}\n"))
            .output()
            .unwrap();
        assert!(out.status.success(), "init: {}", text(&out.stderr));
        m
    }

    fn gage(&self, args: &[&str]) -> assert_cmd::Command {
        let mut cmd = cargo_bin_cmd!("gage");
        cmd.args(args)
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env("XDG_STATE_HOME", self.root.path().join("state"));
        cmd
    }

    /// Runs `args` with `stdin` after the passphrase line, and returns
    /// stdout, requiring success.
    fn ok(&self, args: &[&str], stdin_after_pass: &str) -> String {
        let out = self
            .gage(args)
            .write_stdin(format!("{PASS}\n{stdin_after_pass}"))
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {}", text(&out.stderr));
        text(&out.stdout)
    }

    fn vault_dir(&self) -> PathBuf {
        self.root.path().join("data/gage/vaults/personal")
    }

    fn vault_id(&self) -> String {
        let out = self.gage(&["vault", "info"]).output().unwrap();
        text(&out.stdout)
            .lines()
            .find_map(|l| l.strip_prefix("id: "))
            .expect("vault info prints the id")
            .to_string()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn commits(vault: &Path) -> usize {
    gage::gitrepo::commit_count(vault).unwrap()
}

// Go: TestConcurrentInsertsThroughTheCLIBothSucceed. Two real gage
// processes insert at the same moment (each pays an unlock of about a
// second, so they overlap): both succeed, each leaves exactly one commit,
// and both entries are readable afterwards.
#[test]
fn concurrent_inserts_through_the_cli_both_succeed() {
    let m = Machine::with_vault();
    let before = commits(&m.vault_dir());
    let barrier = std::sync::Barrier::new(2);

    std::thread::scope(|scope| {
        let run = |title: &'static str, value: &'static str| {
            let (m, barrier) = (&m, &barrier);
            scope.spawn(move || {
                let mut cmd = m.gage(&["insert", title]);
                cmd.write_stdin(format!("{PASS}\n{value}\n"));
                barrier.wait();
                cmd.output().unwrap()
            })
        };
        let (a, b) = (run("first", "one"), run("second", "two"));
        for out in [a.join().unwrap(), b.join().unwrap()] {
            assert!(out.status.success(), "{}", text(&out.stderr));
        }
    });

    assert_eq!(commits(&m.vault_dir()), before + 2, "no commit was lost");
    let ls = m.ok(&["ls"], "");
    assert!(ls.contains("first") && ls.contains("second"), "{ls}");
    for (title, value) in [("first", "one"), ("second", "two")] {
        let yaml = m.ok(&["cat", title], "");
        let e = gage::entry::unmarshal(yaml.as_bytes()).unwrap();
        assert_eq!(e.value, value);
    }
    assert!(gage::gitrepo::is_clean(&m.vault_dir()).unwrap());
}

// Go: TestConcurrentReadsDoNotBlockEachOther. A reader takes no lock, so
// with another process holding the vault's write lock for the whole test,
// two concurrent reads (`ls` and `cat`) still complete.
#[test]
fn concurrent_reads_do_not_block_each_other() {
    let m = Machine::with_vault();
    m.ok(&["insert", "readable"], "v\n");
    let lock_path = m
        .root
        .path()
        .join("state/gage/locks")
        .join(format!("{}.lock", m.vault_id()));
    let held = gage::vaultlock::Lock::acquire(&lock_path, std::time::Duration::ZERO)
        .expect("the write lock is free before the readers start");

    std::thread::scope(|scope| {
        let ls = scope.spawn(|| m.ok(&["ls"], ""));
        let cat = scope.spawn(|| m.ok(&["cat", "readable"], ""));
        assert!(ls.join().unwrap().contains("readable"));
        assert!(cat.join().unwrap().contains("readable"));
    });
    drop(held);
}

// `printf 'pass\nvalue\n' | gage insert x --value-stdin`: with stdin
// redirected, the passphrase and the value arrive on one stream. The
// prompter takes the first line and the command must see the rest; a
// prompter that read ahead would swallow the value and the entry would be
// stored empty, with exit 0 and no warning. Only the real binary shares a
// real stdin between the two readers.
#[test]
fn value_stdin_after_a_piped_passphrase_stores_the_value() {
    let m = Machine::with_vault();

    m.ok(&["insert", "piped", "--value-stdin"], "the-value\n");

    let yaml = m.ok(&["cat", "piped"], "");
    let e = gage::entry::unmarshal(yaml.as_bytes()).unwrap();
    assert_eq!(e.value, "the-value");
}

// Go: TestInsertMultilineCapturesFromARealTerminalUntilEOF. A pty is a
// POSIX object, so Unix only, as Go's (`linux || darwin`).
//
// The real binary runs with the pty as its controlling terminal: the
// passphrase is read masked from it, then `-m` reads the terminal until
// Ctrl-D, and what lands in the entry is exactly what was typed.
#[cfg(unix)]
#[test]
fn insert_multiline_captures_from_a_real_terminal_until_eof() {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let m = Machine::with_vault();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(assert_cmd::cargo::cargo_bin!("gage"));
    cmd.args(["insert", "Notes", "-m"]);
    for (k, sub) in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
    ] {
        cmd.env(k, m.root.path().join(sub));
    }
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);

    // Everything the terminal shows, accumulated by a reader thread.
    let screen = Arc::new(Mutex::new(String::new()));
    let mut reader = pair.master.try_clone_reader().unwrap();
    let sink = Arc::clone(&screen);
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    let wait_for = |needle: &str| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if screen.lock().unwrap().contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "never saw {needle:?} on the terminal:\n{}",
            screen.lock().unwrap()
        );
    };
    let mut writer = pair.master.take_writer().unwrap();

    wait_for("Enter passphrase");
    writer.write_all(format!("{PASS}\n").as_bytes()).unwrap();
    wait_for("Ctrl-D");
    // Typed lines, then Ctrl-D at the start of a line: EOF.
    let typed = "first line\n  indented second\n\tthird, with a tab\n";
    writer.write_all(typed.as_bytes()).unwrap();
    writer.write_all(&[0x04]).unwrap();
    writer.flush().unwrap();

    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "insert failed:\n{}",
        screen.lock().unwrap()
    );
    assert!(
        !screen.lock().unwrap().contains(PASS),
        "the passphrase was echoed"
    );

    let yaml = m.ok(&["cat", "Notes"], "");
    let e = gage::entry::unmarshal(yaml.as_bytes()).unwrap();
    assert_eq!(e.value, typed, "byte for byte what was typed");
}
