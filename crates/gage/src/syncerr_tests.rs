//! Every case is triggered for real, against libgit2, never from guessed
//! class and code values: the table in the module doc is what these
//! observed, and they fail if libgit2 starts answering differently.

use std::path::Path;

use super::*;
use crate::error::HasKind;
use crate::gitrepo;
use crate::gittest::{self, Answer, HttpRecorder};

/// The error libgit2 gives connecting to `url` to fetch, with no
/// credentials callback.
fn connect_error(url: &str) -> git2::Error {
    gitrepo::isolate();
    let mut remote = git2::Remote::create_detached(url).unwrap();
    remote.connect(git2::Direction::Fetch).unwrap_err()
}

/// A loopback port nothing listens on.
fn refused_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/me/vault.git")
}

/// The error libgit2 gives pushing an unrelated history over a published
/// one: a real non-fast-forward push.
fn non_fast_forward_error() -> git2::Error {
    let remote = gittest::new_bare_remote();
    let url = remote.path().to_str().unwrap();
    let push = |dir: &Path| {
        std::fs::write(dir.join("f.txt"), dir.to_string_lossy().as_bytes()).unwrap();
        gitrepo::init_and_commit(dir, "seed").unwrap();
        gitrepo::set_remote(dir, url).unwrap();
        let repo = git2::Repository::open(dir).unwrap();
        let mut origin = repo.find_remote("origin").unwrap();
        origin.push(&["refs/heads/main:refs/heads/main"], None)
    };
    let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    push(first.path()).unwrap();
    push(second.path()).unwrap_err()
}

fn assert_class(err: &git2::Error, classes: &[git2::ErrorClass], code: git2::ErrorCode) {
    assert!(
        classes.contains(&err.class()),
        "class {:?}, want one of {classes:?}: {err}",
        err.class()
    );
    assert_eq!(err.code(), code, "{err}");
}

#[test]
fn classify_push_sorts_the_three_buckets() {
    use git2::{ErrorClass as Class, ErrorCode as C};
    let unreachable = [Class::Net, Class::Os];

    let refused = connect_error(&refused_url());
    assert_class(&refused, &unreachable, C::GenericError);
    // RFC 6761 reserves .invalid: it fails to resolve without a network.
    let dns = connect_error("http://gage-test.invalid/me/vault.git");
    assert_class(&dns, &unreachable, C::GenericError);

    let recorder = HttpRecorder::start(Answer::Challenge);
    let unauthorized = connect_error(&recorder.url("me/vault.git"));
    // libgit2's own HTTP client types a 401; its WinHTTP backend (Windows)
    // reports it, with no credentials callback installed, as the generic
    // "request failed with status code: 401". Same class, so same bucket.
    let unauthorized_code = if cfg!(windows) {
        C::GenericError
    } else {
        C::Auth
    };
    assert_class(&unauthorized, &[Class::Http], unauthorized_code);
    let mut refusals = Vec::new();
    for status in [403, 404] {
        let recorder = HttpRecorder::start(Answer::Status(status));
        let err = connect_error(&recorder.url("me/vault.git"));
        assert_class(&err, &[Class::Http], C::GenericError);
        refusals.push(err);
    }
    // libgit2 takes a remote for a local path only if the path exists, so
    // a missing one (an unmounted drive) is an unsupported URL to it.
    let scratch = tempfile::tempdir().unwrap();
    let missing = connect_error(&scratch.path().join("gone.git").to_string_lossy());
    assert_class(&missing, &[Class::Net], C::GenericError);
    let not_a_repo = connect_error(&scratch.path().to_string_lossy());
    assert_class(&not_a_repo, &[Class::Repository], C::GenericError);

    let rejected = non_fast_forward_error();
    assert_class(&rejected, &[Class::Reference], C::NotFastForward);

    let cases: Vec<(&str, &git2::Error, Kind, ErrorKind)> = vec![
        (
            "refused",
            &refused,
            Kind::Unreachable,
            ErrorKind::Unreachable,
        ),
        ("dns", &dns, Kind::Unreachable, ErrorKind::Unreachable),
        ("401", &unauthorized, Kind::Auth, ErrorKind::LockedOrAuth),
        ("403", &refusals[0], Kind::Auth, ErrorKind::LockedOrAuth),
        ("404", &refusals[1], Kind::Auth, ErrorKind::LockedOrAuth),
        (
            "missing path",
            &missing,
            Kind::Unreachable,
            ErrorKind::Unreachable,
        ),
        (
            "not a repository",
            &not_a_repo,
            Kind::Auth,
            ErrorKind::LockedOrAuth,
        ),
        ("non-ff", &rejected, Kind::Diverged, ErrorKind::Conflict),
    ];
    for (name, err, kind, code) in cases {
        let got = classify_push(err);
        assert_eq!(got.kind(), kind, "{name}: {got}");
        assert_eq!(HasKind::kind(&got), code, "{name}");
        assert!(got.to_string().contains(err.message()), "{name}: {got}");
    }

    // gage's own deadline abort is built directly, never inferred.
    let deadline = Error::new(Kind::Unreachable, "timed out after 30s");
    assert_eq!(HasKind::kind(&deadline), ErrorKind::Unreachable);
}

// A real
// non-fast-forward push is typed (`NotFastForward`), so divergence is
// detected positively rather than only by elimination. The rejection a
// server reports through `push_update_reference` is gitrepo's
// (`a_ref_the_remote_refuses_is_diverged_never_success`).
#[test]
fn classify_push_catches_libgit2s_real_rejection() {
    let rejection = non_fast_forward_error();
    assert_eq!(rejection.code(), git2::ErrorCode::NotFastForward);
    assert_eq!(git_kind(&rejection), None, "positive, push-only detection");
    assert_eq!(classify_push(&rejection).kind(), Kind::Diverged);
}

#[test]
fn classify_fetch_never_invents_a_divergence() {
    let repo_dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(repo_dir.path()).unwrap();
    let broken = repo.find_commit(git2::Oid::ZERO_SHA1).unwrap_err();
    assert_eq!(classify_fetch(&broken), None, "passed through: {broken}");

    // A fetch can't be rejected for divergence: even the push-only code
    // is not one when a fetch reports it.
    let rejection = non_fast_forward_error();
    assert_eq!(classify_fetch(&rejection), None);

    // The two buckets a fetch can land in still work.
    let dns = connect_error("http://gage-test.invalid/me/vault.git");
    assert_eq!(classify_fetch(&dns).unwrap().kind(), Kind::Unreachable);
    let recorder = HttpRecorder::start(Answer::Challenge);
    let unauthorized = connect_error(&recorder.url("me/vault.git"));
    assert_eq!(classify_fetch(&unauthorized).unwrap().kind(), Kind::Auth);
}

// Nothing classifies nil: the classifiers take an
// error, not an `Option` of one, so there is no nil to pass; an `Ok` never
// reaches them.

#[test]
fn offline_and_diverged_do_not_share_an_exit_code() {
    let offline = classify_push(&connect_error(&refused_url()));
    let diverged = classify_push(&non_fast_forward_error());
    assert_ne!(HasKind::kind(&offline), HasKind::kind(&diverged));
    assert_eq!(HasKind::kind(&offline), ErrorKind::Unreachable);
    assert_eq!(HasKind::kind(&diverged), ErrorKind::Conflict);
}

#[test]
fn the_message_names_the_bucket_and_the_cause() {
    let err = Error::new(Kind::Unreachable, "failed to connect");
    assert_eq!(
        err.to_string(),
        "could not reach the remote: failed to connect"
    );
    let auth = Error::new(Kind::Auth, "401").annotate_auth("github.com");
    assert_eq!(auth.to_string(), "the remote refused access: 401");
    assert_eq!(
        auth.remedy(),
        Some(Remedy::AuthLogin {
            host: "github.com".into()
        })
    );
    let local = Error::new(Kind::Auth, "401").annotate_auth("");
    assert_eq!(
        local.to_string(),
        "the remote refused access: 401; check that the repository exists at that path and is readable"
    );
    let diverged = Error::new(Kind::Diverged, "rejected").annotate_auth("github.com");
    assert_eq!(
        diverged.to_string(),
        "the remote has diverged from this vault: rejected"
    );
    assert_eq!(diverged.remedy(), None);
}

#[test]
fn credentials_in_a_url_never_survive_into_the_message() {
    let err = Error::new(
        Kind::Auth,
        "failed for 'https://gage:tok_secret@git.example.com/me/v.git' and http://u@h/x",
    );
    let msg = err.to_string();
    assert!(!msg.contains("tok_secret"), "{msg}");
    assert!(!msg.contains("gage:tok"), "{msg}");
    assert!(msg.contains("https://git.example.com/me/v.git"), "{msg}");
    assert!(msg.contains("http://h/x"), "{msg}");
}
