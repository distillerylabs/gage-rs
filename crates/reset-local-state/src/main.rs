//! Deletes every directory gage owns on this machine ($GAGE_CONFIG,
//! $GAGE_DATA and $GAGE_STATE) so a developer can re-test from a
//! fresh-install state. A `make reset-local-state` convenience, never a
//! `gage` subcommand: destroying a user's vaults and identities has no
//! place in the CLI's own command surface.
//!
//! It refuses to run unannounced: it prints exactly what it resolved and
//! deletes nothing unless the answer is `yes`.

use std::io::{BufRead, Write};
use std::path::Path;

use gage::xdgpaths::Dirs;

fn main() {
    let dirs = match Dirs::from_process() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("reset-local-state: {e}");
            std::process::exit(1);
        }
    };
    let result = reset(&dirs, &mut std::io::stdin().lock(), &mut std::io::stdout());
    if let Err(e) = result {
        eprintln!("reset-local-state: {e}");
        std::process::exit(1);
    }
}

/// Shows what would be deleted, asks, and deletes only on `yes`. Returns
/// whether anything was deleted.
fn reset(dirs: &Dirs, input: &mut dyn BufRead, out: &mut dyn Write) -> std::io::Result<bool> {
    let row =
        |var: &str, dir: &Path, what: &str| format!("  {var:<12}  {}   ({what})", dir.display());
    writeln!(
        out,
        "This will permanently delete ALL local gage state on this machine:\n"
    )?;
    writeln!(
        out,
        "{}",
        row(
            "$GAGE_CONFIG",
            &dirs.config,
            "global config: registered vaults, defaults"
        )
    )?;
    writeln!(
        out,
        "{}",
        row(
            "$GAGE_DATA",
            &dirs.data,
            "every vault's git checkout + identity files"
        )
    )?;
    writeln!(
        out,
        "{}",
        row(
            "$GAGE_STATE",
            &dirs.state,
            "locks, trust cache, remote auth tokens, history"
        )
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "Any vault whose only local identity lives here becomes unrecoverable"
    )?;
    writeln!(
        out,
        "on this device unless its recipients or a synced remote still exist.\n"
    )?;
    write!(
        out,
        r#"Type "yes" to permanently delete all of the above: "#
    )?;
    out.flush()?;

    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if answer.trim() != "yes" {
        writeln!(out, "aborted, nothing deleted")?;
        return Ok(false);
    }
    for dir in [&dirs.config, &dirs.data, &dirs.state] {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(std::io::Error::new(
                    e.kind(),
                    format!("removing {}: {e}", dir.display()),
                ));
            }
        }
    }
    writeln!(out, "done: all local gage state deleted")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populated() -> (tempfile::TempDir, Dirs) {
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config/gage"),
            data: root.path().join("data/gage"),
            state: root.path().join("state/gage"),
        };
        for d in [&dirs.config, &dirs.data, &dirs.state] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("file"), b"x").unwrap();
        }
        (root, dirs)
    }

    #[test]
    fn anything_but_yes_deletes_nothing() {
        for answer in ["", "\n", "no\n", "y\n", "YES\n", "yes please\n"] {
            let (_root, dirs) = populated();
            let mut out = Vec::new();

            let deleted = reset(&dirs, &mut answer.as_bytes(), &mut out).unwrap();

            assert!(!deleted, "{answer:?}");
            for d in [&dirs.config, &dirs.data, &dirs.state] {
                assert!(
                    d.join("file").exists(),
                    "{answer:?} deleted {}",
                    d.display()
                );
            }
            assert!(
                String::from_utf8(out)
                    .unwrap()
                    .ends_with("aborted, nothing deleted\n")
            );
        }
    }

    #[test]
    fn yes_deletes_all_three_and_says_what_it_resolved() {
        let (_root, dirs) = populated();
        let mut out = Vec::new();

        assert!(reset(&dirs, &mut "yes\n".as_bytes(), &mut out).unwrap());

        let out = String::from_utf8(out).unwrap();
        for d in [&dirs.config, &dirs.data, &dirs.state] {
            assert!(!d.exists(), "{} survived", d.display());
            assert!(out.contains(&d.display().to_string()), "{out}");
        }
    }

    #[test]
    fn a_directory_that_never_existed_is_not_an_error() {
        let (_root, dirs) = populated();
        std::fs::remove_dir_all(&dirs.state).unwrap();
        assert!(reset(&dirs, &mut "yes\n".as_bytes(), &mut Vec::new()).unwrap());
    }
}
