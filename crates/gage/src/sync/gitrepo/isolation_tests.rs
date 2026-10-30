//! ADR-0016: the user's and the system's git config never reach a
//! vault. Child processes, because libgit2 reads HOME (and its XDG and
//! Windows equivalents) once per process, and because the isolation is a
//! process-wide `Once`: a test in this process would find it already run.

use std::path::Path;
use std::process::Command;

use super::*;
use crate::gittest;

const HOSTILE_HOME_ENV: &str = "GAGE_GITREPO_TEST_HOSTILE_HOME";

/// A path spelled the way git config and remote URLs take it on every OS.
fn slashed(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// The child side. Runs with HOME, USERPROFILE, XDG_CONFIG_HOME and APPDATA
/// pointing at a directory whose global config rewrites `real` to `decoy`
/// and turns on `core.autocrlf`.
#[test]
#[allow(clippy::disallowed_macros)]
fn hostile_git_config_trampoline() {
    let Some(home) = std::env::var_os(HOSTILE_HOME_ENV) else {
        return;
    };
    let home = Path::new(&home);
    let real = home.join("real.git");
    let decoy = home.join("decoy.git");
    let (real_url, decoy_url) = (slashed(&real), slashed(&decoy));

    // Before anything in gitrepo runs: libgit2 sees the hostile config, so
    // the assertions after it can't pass on a config that was never found.
    let config = git2::Config::open_default().unwrap();
    assert_eq!(
        config
            .get_string(&format!("url.{decoy_url}.insteadof"))
            .unwrap(),
        real_url
    );
    assert!(config.get_bool("core.autocrlf").unwrap());
    let raw = home.join("raw");
    let repo = git2::Repository::init(&raw).unwrap();
    repo.remote("origin", &real_url).unwrap();
    assert_eq!(
        repo.find_remote("origin").unwrap().url().unwrap(),
        decoy_url,
        "an unisolated libgit2 rewrites the URL"
    );
    drop(repo);

    // From here on everything goes through gage. The real remote holds a
    // file with LF line endings; the decoy holds something else entirely.
    for (path, file) in [(&real, "real.txt"), (&decoy, "decoy.txt")] {
        git2::Repository::init_opts(
            path,
            git2::RepositoryInitOptions::new()
                .bare(true)
                .initial_head("main"),
        )
        .unwrap();
        let seed = home.join(format!("seed-{file}"));
        std::fs::create_dir(&seed).unwrap();
        std::fs::write(seed.join(file), "one\ntwo\n").unwrap();
        init_and_commit(&seed, "seed").unwrap();
        set_remote(&seed, &slashed(path)).unwrap();
        push(&seed, &gittest::remote_options()).unwrap();
    }

    let vault = home.join("vault");
    clone(&real_url, &vault, &gittest::remote_options()).unwrap();
    assert_eq!(remote_url(&vault).unwrap().as_deref(), Some(&*real_url));
    assert!(vault.join("real.txt").is_file(), "fetched from the decoy");
    assert!(!vault.join("decoy.txt").exists(), "fetched from the decoy");
    assert_eq!(
        std::fs::read(vault.join("real.txt")).unwrap(),
        b"one\ntwo\n",
        "core.autocrlf rewrote a checked-out file"
    );

    // A fetch and a fast-forward, too: the other half of the sync path.
    let other = gittest::Device::clone(Path::new(&real_url));
    other.write_commit_push("later.txt", "three\nfour\n", "later");
    assert!(fetch(&vault, &gittest::remote_options()).unwrap());
    assert!(fast_forward(&vault).unwrap());
    assert_eq!(
        std::fs::read(vault.join("later.txt")).unwrap(),
        b"three\nfour\n"
    );

    // And the config libgit2 would hand any other caller is now empty.
    let config = git2::Config::open_default().unwrap();
    assert!(config.get_string("core.autocrlf").is_err());
    eprintln!("CHECKED");
}

// libgit2 reads the user's git config, and honors `url.<base>.insteadOf` (ADR-0014's spike),
// which would let a global config redirect a vault's pushes.
#[test]
#[allow(clippy::disallowed_methods)]
fn hostile_git_config_is_ignored() {
    let home = tempfile::tempdir().unwrap();
    let real = slashed(&home.path().join("real.git"));
    let decoy = slashed(&home.path().join("decoy.git"));
    let gitconfig = format!(
        "[url \"{decoy}\"]\n\tinsteadOf = {real}\n[core]\n\tautocrlf = true\n\
         [init]\n\tdefaultBranch = hostile\n[user]\n\tname = Hostile\n\temail = hostile@example.com\n"
    );
    for dir in [home.path().to_path_buf(), home.path().join(".config/git")] {
        std::fs::create_dir_all(&dir).unwrap();
        let name = if dir == home.path() {
            ".gitconfig"
        } else {
            "config"
        };
        std::fs::write(dir.join(name), &gitconfig).unwrap();
    }

    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "sync::gitrepo::isolation_tests::hostile_git_config_trampoline",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HOSTILE_HOME_ENV, home.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("APPDATA", home.path().join("AppData"))
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env_remove("GIT_CONFIG_NOSYSTEM")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "child failed:\n{stderr}");
    assert!(stderr.contains("CHECKED"), "child ran no check:\n{stderr}");
}

// The isolation empties every config level gage could otherwise inherit.
#[test]
fn isolation_empties_every_config_search_path() {
    isolate();
    for level in [
        git2::ConfigLevel::System,
        git2::ConfigLevel::XDG,
        git2::ConfigLevel::Global,
        git2::ConfigLevel::ProgramData,
    ] {
        #[allow(unsafe_code)]
        // SAFETY: reads a process-global libgit2 option that only
        // `isolate` (already run above) ever writes.
        let path = unsafe { git2::opts::get_search_path(level) }.unwrap();
        assert!(path.as_bytes().is_empty(), "{level:?}: {path:?}");
    }
}
