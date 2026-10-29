use super::*;

/// A fresh machine's directories.
fn machine() -> (tempfile::TempDir, Dirs) {
    let root = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        config: root.path().join("config").join("gage"),
        data: root.path().join("data").join("gage"),
        state: root.path().join("state").join("gage"),
    };
    (root, dirs)
}

#[test]
fn store_writes_the_token_under_state_at_0600() {
    let (_root, dirs) = machine();
    store(&dirs, "github.com", "ghp_example").unwrap();

    let path = paths::token_file_path(&dirs, "github.com").unwrap();
    // Under the state root, not config: dotfile managers sync config by
    // convention, and a token is disposable and machine-local.
    assert_eq!(path, dirs.state.join("tokens").join("github.com"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o777, 0o700);
    }
    assert_eq!(&*load(&dirs, "github.com").unwrap(), "ghp_example");
}

// Windows doesn't model the mode
// bits (see `load`), so this runs on Unix only.
#[cfg(unix)]
#[test]
fn load_refuses_a_token_others_can_read() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, dirs) = machine();
    store(&dirs, "github.com", "ghp_example").unwrap();
    let path = paths::token_file_path(&dirs, "github.com").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let err = load(&dirs, "github.com").unwrap_err();
    assert!(
        matches!(err, Error::InsecureToken { mode: 0o644, .. }),
        "{err}"
    );
    assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
    assert!(!err.to_string().contains("ghp_example"), "{err}");
    assert_eq!(
        err.remedy(),
        Some(crate::error::Remedy::AuthLoginAgain {
            host: "github.com".into()
        }),
        "{err}"
    );
}

#[test]
fn logout_forgets_the_token_and_is_safe_twice() {
    let (_root, dirs) = machine();
    store(&dirs, "github.com", "ghp_example").unwrap();
    remove(&dirs, "github.com").unwrap();
    assert!(matches!(
        load(&dirs, "github.com"),
        Err(Error::NoToken { .. })
    ));
    remove(&dirs, "github.com").unwrap();
}

#[test]
fn two_vaults_on_one_host_share_one_token() {
    let (_root, dirs) = machine();
    store(&dirs, "github.com", "ghp_shared").unwrap();

    for url in [
        "https://github.com/me/work-vault.git",
        "https://github.com/me/personal-vault.git",
    ] {
        match method(&dirs, url).unwrap() {
            Method::Token { host, token } => {
                assert_eq!(host, "github.com");
                assert_eq!(&*token, "ghp_shared");
            }
            other => panic!("{url}: {other:?}"),
        }
    }
    // A different host is a different token, and doesn't borrow this one.
    assert_eq!(
        method(&dirs, "https://gitlab.com/me/other.git").unwrap(),
        Method::Anonymous {
            host: "gitlab.com".into()
        }
    );
}

#[test]
fn host_reads_both_remote_spellings() {
    for (url, want) in [
        ("https://github.com/me/vault.git", "github.com"),
        (
            "https://git.example.internal:8443/me/vault.git",
            "git.example.internal",
        ),
        ("git@github.com:me/vault.git", "github.com"),
        (
            "ssh://git@git.example.internal/me/vault.git",
            "git.example.internal",
        ),
        // A local path has no host; tests and USB-drive vaults use these.
        ("/tmp/remote.git", ""),
        ("file:///tmp/remote.git", ""),
        (r"C:\vaults\remote.git", ""),
        ("C:/vaults/remote.git", ""),
        ("./relative/remote.git", ""),
    ] {
        assert_eq!(host(url).unwrap(), want, "{url}");
    }
    let err = host("https://").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Usage, "{err}");
}

#[test]
fn token_host_only_names_http_hosts() {
    for (url, want) in [
        ("https://github.com/me/vault.git", "github.com"),
        (
            "http://git.example.internal:8080/me/vault.git",
            "git.example.internal",
        ),
        ("git@github.com:me/vault.git", ""),
        ("ssh://git@git.example.internal/me/vault.git", ""),
        ("/tmp/remote.git", ""),
    ] {
        assert_eq!(token_host(url).unwrap(), want, "{url}");
    }
}

#[test]
fn ssh_remote_needing_ssh_config_fails_with_the_real_reason() {
    let (_root, dirs) = machine();
    let err = method(&dirs, "git@internal:secrets/work-vault.git").unwrap_err();
    assert!(matches!(err, Error::SshConfigAlias { .. }), "{err}");
    assert_eq!(err.kind(), ErrorKind::Usage);
    let msg = err.to_string();
    assert!(msg.contains("~/.ssh/config"), "{msg}");
    assert_eq!(err.remedy(), Some(crate::error::Remedy::UseHttps), "{msg}");
}

// Whether an agent is running
// differs per machine and CI runner, so this pins which path resolving
// took, never the outcome: an agent credential, or an error naming
// ssh-agent. Never the alias refusal, never "unsupported".
#[test]
fn ssh_remote_with_a_real_host_is_attempted() {
    let (_root, dirs) = machine();
    match method(&dirs, "git@github.com:me/vault.git") {
        Ok(Method::SshAgent { host, user }) => {
            assert_eq!((host.as_str(), user.as_str()), ("github.com", "git"));
            // What the credentials callback will hand libgit2.
            git2::Cred::ssh_key_from_agent(&user).unwrap();
        }
        Ok(other) => panic!("an SSH remote resolved to {other:?}"),
        Err(e) => {
            assert!(matches!(e, Error::SshAgent { .. }), "{e}");
            assert!(e.to_string().contains("ssh-agent"), "{e}");
        }
    }
}

#[test]
fn store_refuses_an_empty_token() {
    let (_root, dirs) = machine();
    for token in ["", "   ", "\n"] {
        let err = store(&dirs, "github.com", token).unwrap_err();
        assert!(matches!(err, Error::EmptyToken), "{token:?}");
        assert_eq!(err.kind(), ErrorKind::Usage);
    }
    assert!(!paths::tokens_dir(&dirs).join("github.com").exists());
}

#[test]
fn host_cannot_escape_the_tokens_directory() {
    let (root, dirs) = machine();
    for host in ["../../etc/cron.d/x", "..", "", "a/b", "."] {
        assert!(store(&dirs, host, "tok").is_err(), "{host:?}");
        assert!(load(&dirs, host).is_err(), "{host:?}");
        assert!(remove(&dirs, host).is_err(), "{host:?}");
    }
    assert!(!root.path().join("etc").exists());
}

#[test]
fn an_auth_failure_offers_a_login_for_its_host() {
    use crate::error::{HasKind, Remedy};
    use crate::syncerr;
    let refused = || syncerr::Error::new(syncerr::Kind::Auth, "401");
    assert_eq!(
        refused().annotate_auth("github.com").remedy(),
        Some(Remedy::AuthLogin {
            host: "github.com".into()
        })
    );
    // A local path has no token to be wrong.
    let local = refused().annotate_auth("");
    assert_eq!(local.remedy(), None);
    assert!(local.to_string().contains("path"), "{local}");
}

#[test]
fn hosts_lists_every_stored_host_sorted() {
    let (_root, dirs) = machine();
    assert!(hosts(&dirs).unwrap().is_empty());
    store(&dirs, "gitlab.com", "a").unwrap();
    store(&dirs, "github.com", "b").unwrap();
    assert_eq!(hosts(&dirs).unwrap(), ["github.com", "gitlab.com"]);
}

#[test]
fn a_local_remote_offers_nothing() {
    let (_root, dirs) = machine();
    assert_eq!(method(&dirs, "/tmp/remote.git").unwrap(), Method::Local);
}

#[test]
fn a_methods_debug_never_shows_the_token() {
    let m = Method::Token {
        host: "h".into(),
        token: Zeroizing::new("tok_secret".into()),
    };
    assert!(!format!("{m:?}").contains("tok_secret"));
}
