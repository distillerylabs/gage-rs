//! The access check (#232) through its two entry points: by URL, before a
//! vault exists here, and by vault. The recorder stands in for a host that
//! checks a token; a local bare remote for one that checks nothing.

use super::*;
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::gittest::{self, Answer, HttpRecorder};
use crate::sync::gitrepo;
use crate::sync::remotesync::tests::SyncVault;
use crate::sync::syncerr::Kind;

const TOKEN: &str = "tok_do_not_print_me";
const HOST: &str = "127.0.0.1";

fn grant(push: bool) -> HttpRecorder {
    HttpRecorder::start(Answer::Grant {
        token: TOKEN.into(),
        push,
    })
}

fn dirs() -> (tempfile::TempDir, Dirs) {
    let root = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        config: root.path().join("config"),
        data: root.path().join("data"),
        state: root.path().join("state"),
    };
    (root, dirs)
}

/// A vault whose origin is `url`.
fn vault_at(url: &str) -> SyncVault {
    let v = SyncVault::new();
    gitrepo::set_remote(&v.vault.path, url).unwrap();
    v
}

fn remote_kind(err: &CheckError) -> Option<Kind> {
    match err {
        CheckError::Remote(gitrepo::Error::Sync(e)) => Some(e.kind()),
        _ => None,
    }
}

#[test]
fn a_candidate_token_is_tested_and_not_stored() {
    let (_root, dirs) = dirs();
    let recorder = grant(true);

    let access = check_remote(&dirs, &recorder.url("me/vault.git"), Some(TOKEN)).unwrap();

    assert_eq!(access, Access::ReadWrite);
    assert!(
        recorder
            .authorizations()
            .contains(&gittest::basic("gage", TOKEN))
    );
    assert!(
        remoteauth::hosts(&dirs).unwrap().is_empty(),
        "it was stored"
    );
}

// A pasted newline is not part of a token, as `remoteauth::save` has it.
#[test]
fn a_candidate_token_is_trimmed() {
    let (_root, dirs) = dirs();
    let recorder = grant(true);
    let pasted = format!("{TOKEN}\n");
    let access = check_remote(&dirs, &recorder.url("me/vault.git"), Some(&pasted)).unwrap();
    assert_eq!(access, Access::ReadWrite);
}

#[test]
fn a_candidate_replaces_the_stored_token_for_the_check_only() {
    let (_root, dirs) = dirs();
    remoteauth::save(&dirs, HOST, "tok_stored_and_wrong").unwrap();
    let recorder = grant(false);

    let access = check_remote(&dirs, &recorder.url("me/vault.git"), Some(TOKEN)).unwrap();

    assert_eq!(access, Access::ReadOnly);
    assert!(
        !recorder
            .authorizations()
            .contains(&gittest::basic("gage", "tok_stored_and_wrong")),
        "the stored token was offered"
    );
    assert_eq!(
        *remoteauth::load(&dirs, HOST).unwrap(),
        "tok_stored_and_wrong"
    );
}

#[test]
fn with_no_candidate_the_stored_token_is_tested() {
    let (_root, dirs) = dirs();
    remoteauth::save(&dirs, HOST, TOKEN).unwrap();
    let recorder = grant(false);
    let access = check_remote(&dirs, &recorder.url("me/vault.git"), None).unwrap();
    assert_eq!(access, Access::ReadOnly);
}

#[test]
fn with_no_candidate_and_none_stored_the_check_is_anonymous() {
    let (_root, dirs) = dirs();
    let recorder = grant(true);
    let err = check_remote(&dirs, &recorder.url("me/vault.git"), None).unwrap_err();
    assert_eq!(remote_kind(&err), Some(Kind::Auth), "{err}");
    assert_eq!(
        err.remedy(),
        Some(Remedy::AuthLogin { host: HOST.into() }),
        "{err}"
    );
    assert!(recorder.authorizations().is_empty());
}

#[test]
fn a_refused_candidate_is_an_auth_error_that_never_shows_it() {
    let (_root, dirs) = dirs();
    let recorder = grant(true);
    let err = check_remote(&dirs, &recorder.url("me/vault.git"), Some("tok_wrong")).unwrap_err();
    assert_eq!(remote_kind(&err), Some(Kind::Auth), "{err}");
    assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
    assert!(!format!("{err} {err:?}").contains("tok_wrong"), "{err:?}");
}

#[test]
fn an_empty_candidate_is_refused_before_connecting() {
    let (_root, dirs) = dirs();
    let recorder = grant(true);
    let err = check_remote(&dirs, &recorder.url("me/vault.git"), Some(" \n")).unwrap_err();
    assert!(
        matches!(
            err,
            CheckError::Remote(gitrepo::Error::RemoteAuth(remoteauth::Error::EmptyToken))
        ),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Usage);
    assert!(recorder.requests().is_empty());
}

// A token goes only where `token_host` says one may (ADR-0075): never to a
// local path, an SSH remote, or cleartext `http://` off this machine.
#[test]
fn a_candidate_for_a_remote_never_offered_a_token_is_refused() {
    let (_root, dirs) = dirs();
    let bare = gittest::new_bare_remote();
    for url in [
        bare.path().to_str().unwrap().to_string(),
        "git@github.com:me/vault.git".to_string(),
        "http://git.example.com/me/vault.git".to_string(),
    ] {
        let err = check_remote(&dirs, &url, Some(TOKEN)).unwrap_err();
        assert!(
            matches!(err, CheckError::NotTokenRemote { .. }),
            "{url}: {err:?}"
        );
        assert_eq!(err.kind(), ErrorKind::Usage, "{url}");
        assert!(!err.to_string().contains(TOKEN), "{err}");
    }
}

#[test]
fn a_url_with_credentials_in_it_is_refused() {
    let (_root, dirs) = dirs();
    let err = check_remote(&dirs, "https://me:secret@git.example.com/v.git", None).unwrap_err();
    assert!(
        matches!(
            err,
            CheckError::Remote(gitrepo::Error::RemoteAuth(
                remoteauth::Error::CredentialsInUrl { .. }
            ))
        ),
        "{err:?}"
    );
    assert!(!err.to_string().contains("secret"), "{err}");
}

#[test]
fn an_unreachable_remote_is_unreachable() {
    let (_root, dirs) = dirs();
    let err = check_remote(&dirs, &gittest::refused_url(), Some(TOKEN)).unwrap_err();
    assert_eq!(remote_kind(&err), Some(Kind::Unreachable), "{err}");
    assert_eq!(err.kind(), ErrorKind::Unreachable);
}

#[test]
fn a_vault_checks_its_own_origin() {
    let recorder = grant(true);
    let v = vault_at(&recorder.url("me/vault.git"));
    assert_eq!(
        v.vault.check_remote(Some(TOKEN)).unwrap(),
        Access::ReadWrite
    );
}

#[test]
fn a_vault_checks_the_token_stored_for_its_origin() {
    let recorder = grant(false);
    let v = vault_at(&recorder.url("me/vault.git"));
    remoteauth::save(v.vault.dirs(), HOST, TOKEN).unwrap();
    assert_eq!(v.vault.check_remote(None).unwrap(), Access::ReadOnly);
}

#[test]
fn a_vault_with_a_local_origin_can_push_and_nothing_moves() {
    let v = SyncVault::new();
    let before = gittest::Device::clone(v.remote.path()).head();
    assert_eq!(v.vault.check_remote(None).unwrap(), Access::ReadWrite);
    assert_eq!(gittest::Device::clone(v.remote.path()).head(), before);
    assert!(gitrepo::is_clean(&v.vault.path).unwrap());
}

#[test]
fn a_vault_without_a_remote_says_to_set_one() {
    let v = SyncVault::new();
    git2::Repository::open(&v.vault.path)
        .unwrap()
        .remote_delete(gitrepo::ORIGIN)
        .unwrap();
    let err = v.vault.check_remote(Some(TOKEN)).unwrap_err();
    assert!(matches!(err, CheckError::NoRemote { .. }), "{err:?}");
    assert_eq!(err.kind(), ErrorKind::Usage);
    assert_eq!(err.remedy(), Some(Remedy::SetRemote));
}
