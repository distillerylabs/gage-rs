//! A cross-platform, per-vault advisory file lock (`std::fs::File::lock`,
//! which is `flock(2)` on Unix and `LockFileEx` on Windows). It is a
//! correctness mechanism between cooperating gage processes, not a
//! security one: see "Concurrent processes and the vault lock" in the
//! design doc and Q-FILE-LOCK. Writes take it across their whole
//! read-modify-commit sequence; reads never do. M4 is the first caller.
//!
//! The OS releases the lock the moment the holder's file handle closes,
//! including when the holding process is killed, so there is never a stale
//! lock file to clean up by hand.

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::exitcode::{Code, HasCode};

/// How often [`Lock::acquire`] retries a contended lock. There is no
/// portable "wake me when free" call, so waiting is a short poll loop,
/// which is fine at gage's contention levels (a handful of local
/// processes).
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Who holds a lock, as recorded by the holder. Best-effort: a record that
/// is missing, malformed, or unreadable (Windows locks the bytes it lives
/// in) leaves both fields empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HolderInfo {
    pub pid: Option<u32>,
    pub since: Option<SystemTime>,
}

impl fmt::Display for HolderInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.pid, self.since) {
            (Some(pid), Some(since)) => write!(
                f,
                "pid {pid} (since {})",
                humantime::format_rfc3339_seconds(since)
            ),
            (Some(pid), None) => write!(f, "pid {pid}"),
            (None, _) => f.write_str("another process"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("vaultlock: opening {path}: {source}")]
    Open {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("vaultlock: locking {path}: {source}")]
    Lock {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The timeout elapsed while someone else held the lock.
    #[error("vaultlock: {path} is locked by {holder}")]
    Contended { path: PathBuf, holder: HolderInfo },
    #[error("vaultlock: unlocking {path}: {source}")]
    Unlock {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl HasCode for Error {
    fn code(&self) -> Code {
        match self {
            // Someone else is mid-write: a state to retry or resolve, not
            // a fault.
            Error::Contended { .. } => Code::Conflict,
            _ => Code::Internal,
        }
    }
}

/// A held advisory lock. Dropping it releases the lock;
/// [`release`](Lock::release) does the same and reports an unlock error.
#[derive(Debug)]
pub struct Lock {
    file: Option<File>,
    path: PathBuf,
}

impl Lock {
    /// Takes the advisory lock at `path`, creating the file if needed.
    ///
    /// A contended lock is retried every 20 ms until `timeout` elapses,
    /// since whoever holds it is almost always about to finish, but it
    /// still gives up rather than hang a terminal forever. A zero timeout
    /// tries exactly once.
    pub fn acquire(path: &Path, timeout: Duration) -> Result<Lock, Error> {
        let mut file = open(path)?;
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Lock::held(file, path)),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(Error::Contended {
                        path: path.to_path_buf(),
                        holder: read_holder(&mut file),
                    });
                }
                Err(TryLockError::Error(source)) => {
                    return Err(Error::Lock {
                        path: path.to_path_buf(),
                        source,
                    });
                }
            }
        }
    }

    /// Takes the lock at `path` only if it is free right now: `None` when
    /// someone else holds it, after one attempt and no waiting. For a
    /// caller with something better to do than queue: pull on unlock skips
    /// its catch-up rather than make a read wait behind a write.
    pub fn try_acquire(path: &Path) -> Result<Option<Lock>, Error> {
        let file = open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Lock::held(file, path))),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(source)) => Err(Error::Lock {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// A lock just taken on `file`, with this process recorded as its
    /// holder. Best-effort, like reading it back: the record only ever
    /// feeds a human-facing message.
    fn held(mut file: File, path: &Path) -> Lock {
        let _ = write_holder(&mut file);
        Lock {
            file: Some(file),
            path: path.to_path_buf(),
        }
    }

    /// Releases the lock now, reporting a failure to unlock. Dropping the
    /// lock afterwards is a no-op.
    pub fn release(mut self) -> Result<(), Error> {
        match self.file.take() {
            Some(file) => file.unlock().map_err(|source| Error::Unlock {
                path: self.path.clone(),
                source,
            }),
            None => Ok(()),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            // Closing the handle releases the lock regardless.
            let _ = file.unlock();
        }
    }
}

/// Opens (creating if needed) the lock file at `path`.
fn open(path: &Path) -> Result<File, Error> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|source| Error::Open {
            path: path.to_path_buf(),
            source,
        })
}

fn write_holder(file: &mut File) -> std::io::Result<()> {
    let record = format!(
        "pid={}\nsince={}\n",
        std::process::id(),
        humantime::format_rfc3339_nanos(SystemTime::now())
    );
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(record.as_bytes())
}

fn read_holder(file: &mut File) -> HolderInfo {
    let mut buf = Vec::with_capacity(256);
    let read = file
        .seek(SeekFrom::Start(0))
        .and_then(|_| file.take(256).read_to_end(&mut buf));
    if read.is_err() {
        return HolderInfo::default();
    }
    let mut holder = HolderInfo::default();
    for line in String::from_utf8_lossy(&buf).lines() {
        match line.split_once('=') {
            Some(("pid", v)) => holder.pid = v.parse().ok(),
            Some(("since", v)) => holder.since = humantime::parse_rfc3339(v).ok(),
            _ => {}
        }
    }
    holder
}

// These tests hold the lock from real, separate OS processes, never threads.
// File locks belong to the open file description, so two threads that each
// open the path contend just like two processes would: a thread-based test
// passes, but it would also pass if locking were scoped to something
// in-process (a static mutex standing in for the syscall), which is
// exactly the bug that matters. A session in one terminal and a one-shot
// command in another are separate processes, so the tests use separate
// processes.
//
// The holder is this test binary re-executed with HOLDER_ENV set, running
// only `lock_holder_trampoline` (Go's TestMain trampoline).
#[cfg(test)]
#[allow(clippy::disallowed_macros, clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::mpsc;

    const HOLDER_ENV: &str = "GAGE_VAULTLOCK_TEST_HOLDER";

    /// The child side (Go: TestMain + runLockHolder). Acquires the lock,
    /// announces it on stderr (libtest owns stdout), then holds it until
    /// stdin closes (graceful release) or the process is killed.
    #[test]
    fn lock_holder_trampoline() {
        let Some(path) = std::env::var_os(HOLDER_ENV) else {
            return;
        };
        let lock = Lock::acquire(Path::new(&path), Duration::from_secs(5)).unwrap();
        eprintln!("ACQUIRED");
        let _ = std::io::stdin().read(&mut [0u8]);
        lock.release().unwrap();
    }

    struct Holder {
        child: Child,
        stdin: Option<ChildStdin>,
    }

    impl Holder {
        /// Starts a holder process and returns once it has the lock, so no
        /// test races the child still opening the file.
        fn spawn(path: &Path) -> Holder {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "vaultlock::tests::lock_holder_trampoline",
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(HOLDER_ENV, path)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let stderr = BufReader::new(child.stderr.take().unwrap());
            for line in stderr.lines() {
                let line = line.unwrap();
                if line == "ACQUIRED" {
                    let stdin = child.stdin.take();
                    return Holder { child, stdin };
                }
                eprintln!("holder: {line}");
            }
            let _ = child.kill();
            let _ = child.wait();
            panic!("the holder exited without acquiring the lock");
        }

        fn pid(&self) -> u32 {
            self.child.id()
        }

        /// Graceful release: close the holder's stdin.
        fn release(&mut self) {
            drop(self.stdin.take());
        }

        fn kill(mut self) {
            self.child.kill().unwrap();
            let _ = self.child.wait();
        }
    }

    impl Drop for Holder {
        fn drop(&mut self) {
            self.release();
            let _ = self.child.wait();
        }
    }

    fn lock_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.lock");
        (dir, path)
    }

    // Go: TestAcquireRoundTripsOnThisPlatform
    #[test]
    fn acquire_round_trips_on_this_platform() {
        let (_dir, path) = lock_path();
        Lock::acquire(&path, Duration::ZERO)
            .unwrap()
            .release()
            .unwrap();
        // Once released, a zero-timeout acquire succeeds at once.
        Lock::acquire(&path, Duration::ZERO)
            .unwrap()
            .release()
            .unwrap();
    }

    // Go: TestAcquireSerializesAcrossProcesses
    #[test]
    fn acquire_serializes_across_processes() {
        let (_dir, path) = lock_path();
        let holder = Holder::spawn(&path);

        let err = Lock::acquire(&path, Duration::from_millis(200)).unwrap_err();

        let Error::Contended { path: p, holder: h } = &err else {
            panic!("want Contended, got {err:?}");
        };
        assert_eq!(p, &path);
        assert_eq!(err.code(), Code::Conflict);
        // Q-FILE-LOCK: Windows locks the bytes the record lives in, so a
        // contender can't read it there; the record is best-effort.
        if cfg!(unix) {
            assert_eq!(h.pid, Some(holder.pid()), "the holder's real pid");
            assert!(h.since.is_some());
            assert!(err.to_string().contains(&format!("pid {}", holder.pid())));
        }
    }

    // Go: TestReleaseLetsWaiterProceed
    #[test]
    fn release_lets_waiter_proceed() {
        let (_dir, path) = lock_path();
        let mut holder = Holder::spawn(&path);

        let (tx, rx) = mpsc::channel();
        let waiter_path = path.clone();
        std::thread::spawn(move || {
            let result = Lock::acquire(&waiter_path, Duration::from_secs(5)).map(|l| l.release());
            let _ = tx.send(result.is_ok());
        });

        // The waiter must still be blocked before the release. Without
        // this, a lock that was never held would pass the test too.
        assert!(
            rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "acquire returned while the holder still had the lock"
        );
        holder.release();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)),
            Ok(true),
            "the waiter never got the lock after the holder released it"
        );
    }

    // Go: TestKilledHolderReleasesLock. No stale lock survives a crash.
    #[test]
    fn killed_holder_releases_lock() {
        let (_dir, path) = lock_path();
        Holder::spawn(&path).kill();

        Lock::acquire(&path, Duration::from_secs(2))
            .unwrap()
            .release()
            .unwrap();
    }

    // Go: TestAcquireZeroTimeoutFailsFastWhenContended
    #[test]
    fn acquire_zero_timeout_fails_fast_when_contended() {
        let (_dir, path) = lock_path();
        let _holder = Holder::spawn(&path);

        let start = Instant::now();
        let err = Lock::acquire(&path, Duration::ZERO).unwrap_err();

        assert!(matches!(err, Error::Contended { .. }), "{err:?}");
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    // The other half of the timeout: a non-zero one is waited out before
    // giving up, rather than treated as a single try. Without this, an
    // acquire that ignored its timeout would pass every test above (each
    // contended case either expects failure or is released mid-wait).
    #[test]
    fn a_contended_acquire_waits_out_its_timeout_before_failing() {
        let (_dir, path) = lock_path();
        let _holder = Holder::spawn(&path);
        let timeout = Duration::from_millis(300);

        let start = Instant::now();
        let err = Lock::acquire(&path, timeout).unwrap_err();
        let waited = start.elapsed();

        assert!(matches!(err, Error::Contended { .. }), "{err:?}");
        assert!(waited >= timeout, "gave up after {waited:?} of {timeout:?}");
        assert!(
            waited < timeout + Duration::from_secs(2),
            "overshot a {timeout:?} timeout by {:?}",
            waited - timeout
        );
    }

    // Go: TestReleaseOnANilLockIsANoOp. Rust has no nil lock: release is
    // Drop plus an explicit release() that reports the unlock error. What
    // carries over is that both together are safe: release consumes the
    // lock, and the Drop that follows does nothing.
    #[test]
    fn dropping_after_an_explicit_release_is_a_no_op() {
        let (_dir, path) = lock_path();
        let lock = Lock::acquire(&path, Duration::ZERO).unwrap();
        lock.release().unwrap();
        let _again = Lock::acquire(&path, Duration::ZERO).unwrap();
    }

    // try_acquire answers "busy" at once while another process holds the
    // lock (pull on unlock skips rather than queueing behind a write), and
    // takes the lock once it is free.
    #[test]
    fn try_acquire_answers_busy_without_waiting() {
        let (_dir, path) = lock_path();
        let mut holder = Holder::spawn(&path);

        let start = Instant::now();
        assert!(
            Lock::try_acquire(&path).unwrap().is_none(),
            "held elsewhere"
        );
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "try_acquire waited {:?}",
            start.elapsed()
        );

        holder.release();
        let deadline = Instant::now() + Duration::from_secs(5);
        let lock = loop {
            if let Some(lock) = Lock::try_acquire(&path).unwrap() {
                break lock;
            }
            assert!(Instant::now() < deadline, "never free after release");
            std::thread::sleep(POLL_INTERVAL);
        };
        // Held for real: a second try from here is busy too.
        assert!(Lock::try_acquire(&path).unwrap().is_none());
        lock.release().unwrap();
    }

    #[test]
    fn try_acquire_open_failure_is_an_error_not_busy() {
        let (dir, _) = lock_path();
        let err = Lock::try_acquire(&dir.path().join("missing/vault.lock")).unwrap_err();
        assert!(matches!(err, Error::Open { .. }), "{err:?}");
    }

    #[test]
    fn dropping_a_lock_releases_it() {
        let (_dir, path) = lock_path();
        drop(Lock::acquire(&path, Duration::ZERO).unwrap());
        Lock::acquire(&path, Duration::ZERO).unwrap();
    }

    #[test]
    fn open_failure_is_internal_not_contended() {
        let (dir, _) = lock_path();
        let err =
            Lock::acquire(&dir.path().join("missing/vault.lock"), Duration::ZERO).unwrap_err();
        assert!(matches!(err, Error::Open { .. }), "{err:?}");
        assert_eq!(err.code(), Code::Internal);
    }

    #[test]
    fn holder_info_renders_for_humans() {
        assert_eq!(HolderInfo::default().to_string(), "another process");
        let since = humantime::parse_rfc3339("2026-01-02T03:04:05Z").unwrap();
        let h = HolderInfo {
            pid: Some(42),
            since: Some(since),
        };
        assert_eq!(h.to_string(), "pid 42 (since 2026-01-02T03:04:05Z)");
    }
}
