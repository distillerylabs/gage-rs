//! Fetch, push, compare, fast-forward and clone against ephemeral bare
//! repositories, and auth observed on the wire through the loopback
//! recorder.

use std::path::Path;
use std::time::Duration;

use super::worktree_tests::{head_id, read, write};
use super::*;
use crate::gittest::{self, Answer, BareRemote, Device, HttpRecorder};
use crate::syncerr::Kind;
use crate::xdgpaths::Dirs;

fn opts() -> RemoteOptions {
    gittest::remote_options()
}

fn url(remote: &BareRemote) -> &str {
    remote.path().to_str().unwrap()
}

/// A repository with one commit, published to a fresh bare remote, with
/// the remote-tracking ref populated: where every sync starts.
pub(super) fn published_repo() -> (tempfile::TempDir, BareRemote) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "seed.txt", "seed");
    init_and_commit(dir.path(), "initial commit").unwrap();
    let remote = gittest::new_bare_remote();
    set_remote(dir.path(), url(&remote)).unwrap();
    assert!(push(dir.path(), &opts()).unwrap());
    (dir, remote)
}

pub(super) fn commit_file(dir: &Path, rel: &str, content: &str) {
    write(dir, rel, content);
    commit_all(dir, &format!("commit {rel}")).unwrap();
}

/// A real divergence: the other device publishes `theirs`, this side
/// commits `ours`, and the tracking ref is refreshed.
pub(super) fn diverge_from(dir: &Path, remote: &BareRemote, theirs: &str, ours: &str) {
    Device::clone(remote.path()).write_commit_push(theirs, "theirs", "their commit");
    commit_file(dir, ours, "ours");
    fetch(dir, &opts()).unwrap();
}

fn sync_kind(err: &Error) -> Kind {
    match err {
        Error::Sync(e) => e.kind(),
        other => panic!("not a sync error: {other}"),
    }
}

#[test]
fn has_remote_on_a_directory_that_is_not_a_repo() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!has_remote(dir.path()).unwrap());
}

// Its `state(99)`
// half is struck: a Rust enum can't hold an out-of-range value.
#[test]
fn remote_state_display_covers_all_states() {
    let words: Vec<String> = [
        RemoteState::None,
        RemoteState::InSync,
        RemoteState::Ahead,
        RemoteState::Behind,
        RemoteState::Diverged,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        words,
        ["no-remote", "in-sync", "ahead", "behind", "diverged"]
    );
}

#[test]
fn compare_reports_the_four_states() {
    let (dir, remote) = published_repo();
    assert_eq!(compare(dir.path()).unwrap(), RemoteState::InSync);

    commit_file(dir.path(), "local.txt", "local");
    assert_eq!(compare(dir.path()).unwrap(), RemoteState::Ahead);
    assert_eq!(ahead_count(dir.path()).unwrap(), 1);

    Device::clone(remote.path()).write_commit_push("remote.txt", "remote", "theirs");
    assert!(fetch(dir.path(), &opts()).unwrap());
    assert_eq!(compare(dir.path()).unwrap(), RemoteState::Diverged);
}

#[test]
fn compare_reports_remote_behind() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("remote.txt", "remote", "theirs");
    fetch(dir.path(), &opts()).unwrap();
    assert_eq!(compare(dir.path()).unwrap(), RemoteState::Behind);
}

#[test]
fn compare_with_no_remote_tracking_ref_is_remote_none() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "seed.txt", "seed");
    init_and_commit(dir.path(), "initial commit").unwrap();
    assert_eq!(compare(dir.path()).unwrap(), RemoteState::None);
    assert_eq!(ahead_count(dir.path()).unwrap(), 0);
}

#[test]
fn ahead_count_counts_a_plain_run_of_commits() {
    let (dir, _remote) = published_repo();
    assert_eq!(ahead_count(dir.path()).unwrap(), 0);
    for (i, name) in ["one.txt", "two.txt", "three.txt"].iter().enumerate() {
        commit_file(dir.path(), name, name);
        assert_eq!(ahead_count(dir.path()).unwrap(), i + 1);
    }
}

// Count what
// the remote can't reach, not the walk back to the merge-base commit.
#[test]
fn ahead_count_is_not_fooled_by_a_merge_commit() {
    let (dir, remote) = published_repo();
    for n in ["a", "b", "c", "d"] {
        Device::clone(remote.path()).write_commit_push(&format!("{n}.txt"), n, n);
    }
    fetch(dir.path(), &opts()).unwrap();
    assert!(fast_forward(dir.path()).unwrap());

    diverge_from(dir.path(), &remote, "theirs.txt", "ours.txt");
    let merged = merge_remote(dir.path(), "gage: merge origin").unwrap();
    assert!(matches!(merged, MergeResult::Merged { .. }), "{merged:?}");
    // Our commit and the merge commit; a larger number means the walk
    // crossed the merge into shared history.
    assert_eq!(ahead_count(dir.path()).unwrap(), 2);

    Device::clone(remote.path()).write_commit_push("later.txt", "later", "later");
    fetch(dir.path(), &opts()).unwrap();
    assert_eq!(compare(dir.path()).unwrap(), RemoteState::Diverged);
    assert_eq!(ahead_count(dir.path()).unwrap(), 2);
}

#[test]
fn a_push_to_an_up_to_date_remote_sends_nothing() {
    let (dir, _remote) = published_repo();
    assert!(!push(dir.path(), &opts()).unwrap());
    commit_file(dir.path(), "x.txt", "x");
    assert!(push(dir.path(), &opts()).unwrap());
    assert_eq!(compare(dir.path()).unwrap(), RemoteState::InSync);
}

#[test]
fn a_fetch_that_brings_nothing_reports_no_change() {
    let (dir, remote) = published_repo();
    fetch(dir.path(), &opts()).unwrap();
    assert!(!fetch(dir.path(), &opts()).unwrap());
    Device::clone(remote.path()).write_commit_push("r.txt", "r", "r");
    assert!(fetch(dir.path(), &opts()).unwrap());
    assert!(!fetch(dir.path(), &opts()).unwrap());
}

// ADR-0018: a deadline of zero, never real time. A fetch with
// something to transfer and a push with something to send both reach a
// callback that checks it.
#[test]
fn a_passed_deadline_aborts_fetch_and_push_as_unreachable() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("r.txt", "r", "r");
    let expired = opts().with_timeout(Duration::ZERO);

    let err = fetch(dir.path(), &expired).unwrap_err();
    assert_eq!(sync_kind(&err), Kind::Unreachable, "{err}");
    assert_eq!(err.code(), Code::Unreachable);
    assert_eq!(
        compare(dir.path()).unwrap(),
        RemoteState::InSync,
        "nothing fetched"
    );

    let (dir, _remote) = published_repo();
    commit_file(dir.path(), "x.txt", "x");
    let err = push(dir.path(), &expired).unwrap_err();
    assert_eq!(sync_kind(&err), Kind::Unreachable, "{err}");
    assert_eq!(
        compare(dir.path()).unwrap(),
        RemoteState::Ahead,
        "nothing sent"
    );
}

#[test]
fn a_non_fast_forward_push_is_diverged() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("theirs.txt", "t", "t");
    commit_file(dir.path(), "ours.txt", "o");
    let err = push(dir.path(), &opts()).unwrap_err();
    assert_eq!(sync_kind(&err), Kind::Diverged, "{err}");
    assert_eq!(err.code(), Code::Conflict);
    assert!(!err.to_string().contains("gage auth login"), "{err}");
}

// ADR-0017: a ref the remote refuses arrives through
// `push_update_reference`, and `push()` itself can succeed. A locked ref
// on the remote makes the local transport refuse the update.
#[test]
fn a_ref_the_remote_refuses_is_diverged_never_success() {
    let (dir, remote) = published_repo();
    commit_file(dir.path(), "x.txt", "x");
    std::fs::write(remote.path().join("refs/heads/main.lock"), "").unwrap();

    let err = push(dir.path(), &opts()).unwrap_err();
    assert_eq!(sync_kind(&err), Kind::Diverged, "{err}");
    assert!(err.to_string().contains("refs/heads/main"), "{err}");
}

#[test]
fn fast_forward_only_moves_when_it_is_a_fast_forward() {
    let (dir, remote) = published_repo();
    let other = Device::clone(remote.path());
    other.write_commit_push("remote.txt", "remote", "theirs");
    fetch(dir.path(), &opts()).unwrap();
    assert!(fast_forward(dir.path()).unwrap());
    assert_eq!(read(dir.path(), "remote.txt"), "remote");
    assert_eq!(compare(dir.path()).unwrap(), RemoteState::InSync);

    // Both sides moved: ff-only catches up cleanly or does nothing.
    commit_file(dir.path(), "local.txt", "local");
    other.pull();
    other.write_commit_push("remote2.txt", "remote2", "theirs again");
    fetch(dir.path(), &opts()).unwrap();
    let before = head_id(dir.path());
    assert!(!fast_forward(dir.path()).unwrap());
    assert_eq!(head_id(dir.path()), before);
    assert!(!dir.path().join("remote2.txt").exists());
}

#[test]
fn fast_forward_refuses_over_uncommitted_changes() {
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("remote.txt", "remote", "theirs");
    fetch(dir.path(), &opts()).unwrap();
    write(dir.path(), "seed.txt", "edited outside gage");

    let err = fast_forward(dir.path()).unwrap_err();
    assert!(matches!(err, Error::DirtyWorkTree { .. }), "{err}");
    assert_eq!(read(dir.path(), "seed.txt"), "edited outside gage");
}

#[test]
fn clone_produces_a_working_copy() {
    let (_dir, remote) = published_repo();
    let dest = tempfile::tempdir().unwrap();
    let dest = dest.path().join("clone");
    clone(url(&remote), &dest, &opts()).unwrap();
    assert_eq!(read(&dest, "seed.txt"), "seed");
    assert_eq!(remote_url(&dest).unwrap().as_deref(), Some(url(&remote)));
    assert_eq!(compare(&dest).unwrap(), RemoteState::InSync);
}

// A bare repository
// whose HEAD names a branch that will never exist, as `git init --bare`
// can leave one.
#[test]
fn clone_works_when_the_remote_head_points_nowhere() {
    let (dir, _) = published_repo();
    let bare = gittest::new_bare_remote();
    git2::Repository::open_bare(bare.path())
        .unwrap()
        .set_head("refs/heads/trunk")
        .unwrap();
    set_remote(dir.path(), url(&bare)).unwrap();
    push(dir.path(), &opts()).unwrap();

    let dest = tempfile::tempdir().unwrap();
    let dest = dest.path().join("clone");
    clone(url(&bare), &dest, &opts()).unwrap();
    assert_eq!(read(&dest, "seed.txt"), "seed");
}

/// Pushes one commit on each named branch to `remote`, and nothing else.
fn push_branches(remote: &BareRemote, names: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "f.txt", "root");
    init_and_commit(dir.path(), "root").unwrap();
    set_remote(dir.path(), url(remote)).unwrap();
    let repo = git2::Repository::open(dir.path()).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let specs: Vec<String> = names
        .iter()
        .map(|name| {
            if repo.find_branch(name, git2::BranchType::Local).is_err() {
                repo.branch(name, &head, false).unwrap();
            }
            format!("refs/heads/{name}:refs/heads/{name}")
        })
        .collect();
    repo.find_remote(ORIGIN)
        .unwrap()
        .push(&specs, None)
        .unwrap();
}

#[test]
fn remote_branch_picks_main_or_master_and_refuses_ambiguity() {
    let pick = |names: &[&str]| {
        let remote = gittest::new_bare_remote();
        push_branches(&remote, names);
        let r = remote::remote_branch(url(&remote), &remoteauth::Method::Local, &opts());
        (remote, r)
    };
    assert_eq!(pick(&["main", "feature"]).1.unwrap(), "main");
    assert_eq!(pick(&["master", "feature"]).1.unwrap(), "master");
    assert_eq!(pick(&["main", "master"]).1.unwrap(), "main");
    assert_eq!(pick(&["only"]).1.unwrap(), "only");

    let err = pick(&["feature-a", "feature-b"]).1.unwrap_err();
    assert!(matches!(err, Error::AmbiguousBranch { .. }), "{err}");
    assert!(err.to_string().contains("several branches"), "{err}");

    // No branches at all (only a tag): not a vault.
    let remote = gittest::new_bare_remote();
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "f.txt", "x");
    init_and_commit(dir.path(), "c").unwrap();
    set_remote(dir.path(), url(&remote)).unwrap();
    let repo = git2::Repository::open(dir.path()).unwrap();
    let head = repo.head().unwrap().peel(git2::ObjectType::Commit).unwrap();
    repo.tag_lightweight("v1", &head, false).unwrap();
    repo.find_remote(ORIGIN)
        .unwrap()
        .push(&["refs/tags/v1:refs/tags/v1"], None)
        .unwrap();
    let err = remote::remote_branch(url(&remote), &remoteauth::Method::Local, &opts()).unwrap_err();
    assert!(matches!(err, Error::NoBranches { .. }), "{err}");
    assert!(
        err.to_string().contains("empty repository, not a vault"),
        "{err}"
    );

    let empty = gittest::new_bare_remote();
    let err = remote::remote_branch(url(&empty), &remoteauth::Method::Local, &opts()).unwrap_err();
    assert!(matches!(err, Error::NoBranches { .. }), "{err}");
}

// --- auth on the wire ---------------------------------------------------

/// A machine with its own token store.
struct Machine {
    _root: tempfile::TempDir,
    dirs: Dirs,
}

fn machine() -> (Machine, RemoteOptions) {
    let root = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        config: root.path().join("config"),
        data: root.path().join("data"),
        state: root.path().join("state"),
    };
    let opts = RemoteOptions::new(&dirs);
    (Machine { _root: root, dirs }, opts)
}

/// A repository with one commit whose origin is `url`.
fn repo_with_origin(url: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "seed.txt", "seed");
    init_and_commit(dir.path(), "seed").unwrap();
    set_remote(dir.path(), url).unwrap();
    dir
}

const TOKEN: &str = "tok_do_not_print_me";

// Fetch and push carry the stored token: the token is observed on the wire, as `Basic` credentials on
// the retry after a 401, for fetch (upload-pack) and push (receive-pack).
// Without this test, dropping the credentials from both operations
// left the suite green.
#[test]
fn fetch_and_push_carry_the_stored_token_to_libgit2() {
    let (m, opts) = machine();
    let recorder = HttpRecorder::start(Answer::Challenge);
    remoteauth::store(&m.dirs, "127.0.0.1", TOKEN).unwrap();
    let dir = repo_with_origin(&recorder.url("me/vault.git"));

    let fetch_err = fetch(dir.path(), &opts).unwrap_err();
    let push_err = push(dir.path(), &opts).unwrap_err();
    for err in [&fetch_err, &push_err] {
        assert_eq!(sync_kind(err), Kind::Auth, "{err}");
    }

    let want = gittest::basic("gage", TOKEN);
    for service in ["git-upload-pack", "git-receive-pack"] {
        let sent: Vec<_> = recorder
            .requests()
            .into_iter()
            .filter(|r| r.target.contains(service))
            .collect();
        assert!(!sent.is_empty(), "{service} never reached the server");
        assert_eq!(sent[0].authorization, None, "{service}: anonymous first");
        let carried: Vec<_> = sent
            .iter()
            .filter_map(|r| r.authorization.clone())
            .collect();
        // Exactly one retry with the token: a rejected token doesn't loop.
        assert_eq!(carried, std::slice::from_ref(&want), "{service}");
        assert!(sent.iter().all(|r| r.host.starts_with("127.0.0.1")));
    }
}

#[test]
fn fetch_and_push_go_anonymous_with_no_stored_token() {
    let (_m, opts) = machine();
    let recorder = HttpRecorder::start(Answer::Challenge);
    let dir = repo_with_origin(&recorder.url("me/vault.git"));

    let fetch_err = fetch(dir.path(), &opts).unwrap_err();
    let push_err = push(dir.path(), &opts).unwrap_err();
    for err in [&fetch_err, &push_err] {
        assert_eq!(sync_kind(err), Kind::Auth, "{err}");
    }
    assert!(recorder.authorizations().is_empty());
    assert_eq!(recorder.requests().len(), 2, "one attempt each, anonymous");
}

#[test]
fn push_after_logout_names_the_host_and_the_command() {
    let (m, opts) = machine();
    let recorder = HttpRecorder::start(Answer::Challenge);
    remoteauth::store(&m.dirs, "127.0.0.1", TOKEN).unwrap();
    remoteauth::remove(&m.dirs, "127.0.0.1").unwrap();
    let dir = repo_with_origin(&recorder.url("me/vault.git"));

    let err = push(dir.path(), &opts).unwrap_err();
    assert_eq!(sync_kind(&err), Kind::Auth, "{err}");
    assert_eq!(err.code(), Code::LockedOrAuth);
    for want in ["127.0.0.1", "gage auth login"] {
        assert!(err.to_string().contains(want), "{err}");
    }
}

// The token is a bearer
// credential: nothing printed, logged or kept in session history may carry
// it. Each refusal shape the recorder can produce is checked.
#[test]
fn no_token_value_leaks_into_a_sync_error() {
    for answer in [
        Answer::Challenge,
        Answer::Status(403),
        Answer::Status(404),
        Answer::RedirectAfterAuth("http://localhost:1/elsewhere".into()),
    ] {
        let (m, opts) = machine();
        let recorder = HttpRecorder::start(answer.clone());
        remoteauth::store(&m.dirs, "127.0.0.1", TOKEN).unwrap();
        let dir = repo_with_origin(&recorder.url("me/vault.git"));
        for err in [
            fetch(dir.path(), &opts).unwrap_err(),
            push(dir.path(), &opts).unwrap_err(),
        ] {
            let shown = format!("{err} {err:?}");
            assert!(!shown.contains(TOKEN), "{answer:?}: {shown}");
        }
    }
}

// Auth failures name the host and the fix. An expired or
// revoked token comes back naming the host and `gage auth login`, not as a
// bare 403; a divergence never acquires that hint.
#[test]
fn auth_failures_name_the_host_and_the_fix() {
    let (_m, opts) = machine();
    for status in [403, 404] {
        let recorder = HttpRecorder::start(Answer::Status(status));
        let dir = repo_with_origin(&recorder.url("me/vault.git"));
        let err = push(dir.path(), &opts).unwrap_err();
        assert_eq!(sync_kind(&err), Kind::Auth, "{status}: {err}");
        for want in ["127.0.0.1", "gage auth login"] {
            assert!(err.to_string().contains(want), "{status}: {err}");
        }
    }
    let (dir, remote) = published_repo();
    Device::clone(remote.path()).write_commit_push("t.txt", "t", "t");
    commit_file(dir.path(), "o.txt", "o");
    let diverged = push(dir.path(), &opts).unwrap_err();
    assert!(
        !diverged.to_string().contains("gage auth login"),
        "{diverged}"
    );
}

// ADR-0020: the token is offered only to the host it was
// stored for. Here the server redirects the first, anonymous request to a
// different host name (the same listener under `localhost`), which then
// asks for credentials.
#[test]
fn a_token_is_not_offered_to_a_host_it_was_not_stored_for() {
    let (m, opts) = machine();
    let recorder = HttpRecorder::start(Answer::Challenge);
    let redirector = HttpRecorder::start(Answer::RedirectFirst(
        recorder.localhost_url("me/vault.git/info/refs?service=git-upload-pack"),
    ));
    remoteauth::store(&m.dirs, "127.0.0.1", TOKEN).unwrap();
    let dir = repo_with_origin(&redirector.url("me/vault.git"));

    let err = fetch(dir.path(), &opts).unwrap_err();
    assert_eq!(sync_kind(&err), Kind::Auth, "{err}");
    let reached_localhost = recorder
        .requests()
        .iter()
        .any(|r| r.host.starts_with("localhost"));
    if cfg!(windows) {
        // WinHTTP follows a redirect to another host only to https://, and
        // REDIRECTS turns even that off; a plain-http one is replayed to
        // the same server until libgit2 gives up.
        assert!(!reached_localhost, "WinHTTP followed an offsite redirect");
    } else {
        assert!(
            reached_localhost,
            "the redirect was never followed, so nothing was tested"
        );
    }
    let leaked: Vec<_> = recorder
        .authorizations()
        .into_iter()
        .chain(redirector.authorizations())
        .collect();
    assert!(leaked.is_empty(), "{leaked:?}");
}

// The other redirect shape: a host that accepted the token and then
// redirects. libgit2 follows only the initial request's redirect
// (`RemoteRedirect::Initial`); whatever it does next, the token must not
// reach the other host.
#[test]
fn a_redirect_after_the_token_does_not_carry_it_to_another_host() {
    let (m, opts) = machine();
    let elsewhere = HttpRecorder::start(Answer::Challenge);
    let origin = HttpRecorder::start(Answer::RedirectAfterAuth(
        elsewhere.localhost_url("me/vault.git/info/refs?service=git-upload-pack"),
    ));
    remoteauth::store(&m.dirs, "127.0.0.1", TOKEN).unwrap();
    let dir = repo_with_origin(&origin.url("me/vault.git"));

    let err = fetch(dir.path(), &opts).unwrap_err();
    assert_eq!(sync_kind(&err), Kind::Auth, "{err}");
    let token = gittest::basic("gage", TOKEN);
    let reached_elsewhere = elsewhere
        .requests()
        .iter()
        .any(|r| r.host.starts_with("localhost"));
    if cfg!(windows) {
        // WinHTTP re-sends the accepted token with every replay, but only
        // to the origin it was accepted by: the plain-http redirect is
        // replayed there until libgit2 gives up, and never followed.
        assert!(!reached_elsewhere, "WinHTTP followed an offsite redirect");
        let sent = origin.authorizations();
        assert!(!sent.is_empty(), "the token was never offered");
        assert!(sent.iter().all(|a| *a == token), "{sent:?}");
    } else {
        assert!(err.to_string().contains("localhost asked"), "{err}");
        assert_eq!(origin.authorizations(), [token]);
        assert!(
            reached_elsewhere,
            "the redirect was never followed, so nothing was tested"
        );
    }
    let leaked: Vec<_> = elsewhere
        .requests()
        .into_iter()
        .filter(|r| r.host.starts_with("localhost"))
        .filter_map(|r| r.authorization)
        .collect();
    assert!(leaked.is_empty(), "{leaked:?}");
}

// ADR-0017, for real through gitrepo: a refused port and an
// unresolvable host are unreachable, with no auth hint.
#[test]
fn an_unreachable_remote_is_unreachable() {
    let (_m, opts) = machine();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    for url in [
        format!("http://127.0.0.1:{port}/me/vault.git"),
        "http://gage-test.invalid/me/vault.git".to_string(),
    ] {
        let dir = repo_with_origin(&url);
        for err in [
            fetch(dir.path(), &opts).unwrap_err(),
            push(dir.path(), &opts).unwrap_err(),
        ] {
            assert_eq!(sync_kind(&err), Kind::Unreachable, "{url}: {err}");
            assert_eq!(err.code(), Code::Unreachable);
        }
    }
}

#[test]
fn a_missing_local_remote_is_unreachable_and_named() {
    let gone = tempfile::tempdir().unwrap().path().join("gone.git");
    let dir = repo_with_origin(gone.to_str().unwrap());
    let err = fetch(dir.path(), &opts()).unwrap_err();
    assert_eq!(sync_kind(&err), Kind::Unreachable, "{err}");
    assert!(err.to_string().contains("gone.git"), "{err}");
}

// ADR-0020: under WinHTTP no offsite redirect is followed, since it
// would carry an accepted token with it; libgit2's own client follows the
// initial request's only. The branch listing takes the same policy from
// its scratch repository's config, spelled as git spells it.
#[test]
fn the_redirect_policy_matches_the_http_backend() {
    if cfg!(windows) {
        assert!(matches!(remote::REDIRECTS, git2::RemoteRedirect::None));
        assert_eq!(remote::REDIRECTS_CONFIG, "false");
    } else {
        assert!(matches!(remote::REDIRECTS, git2::RemoteRedirect::Initial));
        assert_eq!(remote::REDIRECTS_CONFIG, "initial");
    }
}
