//! Every case is triggered for real, against libgit2, never from guessed
//! class and code values: the table in the module doc is what these
//! observed, and they fail if libgit2 starts answering differently.

use std::path::Path;

use super::*;
use crate::exitcode::code_of;
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

// Go: TestClassifyPushSortsTheThreeBuckets
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
    assert_class(&unauthorized, &[Class::Http], C::Auth);
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

    let cases: Vec<(&str, &git2::Error, Kind, Code)> = vec![
        ("refused", &refused, Kind::Unreachable, Code::Unreachable),
        ("dns", &dns, Kind::Unreachable, Code::Unreachable),
        ("401", &unauthorized, Kind::Auth, Code::LockedOrAuth),
        ("403", &refusals[0], Kind::Auth, Code::LockedOrAuth),
        ("404", &refusals[1], Kind::Auth, Code::LockedOrAuth),
        (
            "missing path",
            &missing,
            Kind::Unreachable,
            Code::Unreachable,
        ),
        (
            "not a repository",
            &not_a_repo,
            Kind::Auth,
            Code::LockedOrAuth,
        ),
        ("non-ff", &rejected, Kind::Diverged, Code::Conflict),
    ];
    for (name, err, kind, code) in cases {
        let got = classify_push(err);
        assert_eq!(got.kind(), kind, "{name}: {got}");
        assert_eq!(code_of::<(), _>(&Err(got.clone())), code, "{name}");
        assert!(got.to_string().contains(err.message()), "{name}: {got}");
    }

    // gage's own deadline abort is built directly, never inferred.
    let deadline = Error::new(Kind::Unreachable, "timed out after 30s");
    assert_eq!(deadline.code(), Code::Unreachable);
}

// Go: TestClassifyPushCatchesGoGitsRealRejection, as libgit2's: a real
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

// Go: TestClassifyFetchNeverInventsADivergence
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

// Go: TestClassifyPassesNilThrough is excused: the classifiers take an
// error, not an `Option` of one, so there is no nil to pass; an `Ok` never
// reaches them.

// Go: TestOfflineAndDivergedDoNotShareAnExitCode
#[test]
fn offline_and_diverged_do_not_share_an_exit_code() {
    let offline = classify_push(&connect_error(&refused_url()));
    let diverged = classify_push(&non_fast_forward_error());
    assert_ne!(offline.code(), diverged.code());
    assert_eq!(offline.code(), Code::Unreachable);
    assert_eq!(diverged.code(), Code::Conflict);
}

/// An error that wraps another, as a caller retrying an operation would.
#[derive(Debug, thiserror::Error)]
#[error("gage: retrying: {0}")]
struct Retrying(#[source] Error);

// Go: TestClassifyIsIdempotentOnAnAlreadyClassifiedUnreachable
#[test]
fn classify_is_idempotent_on_an_already_classified_unreachable() {
    let already = Retrying(Error::new(Kind::Unreachable, "connection refused"));
    for got in [classify_push(&already), classify_fetch(&already).unwrap()] {
        assert_eq!(got.kind(), Kind::Unreachable, "{got}");
        assert_eq!(got.code(), Code::Unreachable);
    }
}

#[test]
fn the_message_names_the_bucket_and_the_cause() {
    let err = Error::new(Kind::Unreachable, "failed to connect");
    assert_eq!(
        err.to_string(),
        "gage: could not reach the remote: failed to connect"
    );
    let auth = Error::new(Kind::Auth, "401").annotate_auth("run x");
    assert_eq!(
        auth.to_string(),
        "gage: the remote refused access: 401; run x"
    );
    let diverged = Error::new(Kind::Diverged, "rejected").annotate_auth("run x");
    assert_eq!(
        diverged.to_string(),
        "gage: the remote has diverged from this vault: rejected"
    );
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
