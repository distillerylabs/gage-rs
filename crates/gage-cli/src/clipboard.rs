//! The system clipboard behind `-c/--clip`, and the keeper that makes sure
//! a copied secret doesn't stay on it.
//!
//! Every part that touches the OS sits behind a seam (the clipboard port,
//! the timer and the wait), so every test
//! exercises the copy and the clear against fakes. No test touches the
//! developer's real clipboard or needs one to exist, which on a headless
//! CI runner it does not (ADR-0035).

use std::io::Write;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use crate::exitcode::ExitCode;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::error::{CliError, prefixed};

/// The system clipboard, behind one signature. Its only production
/// implementation is [`SystemClipboard`]. Writing `""` clears it.
pub trait ClipboardPort: Send {
    fn write(&mut self, text: &str) -> anyhow::Result<()>;
    fn read(&mut self) -> anyhow::Result<String>;
}

/// The production [`ClipboardPort`]: `arboard`, which writes in-process on
/// every platform, so the value never passes through a child process.
///
/// Opened on first use, not at startup, so a command that never copies
/// never needs a clipboard to exist. Held open after that: on Linux the
/// contents last only while a `Clipboard` lives, which is by design:
/// the clear happens in the process that wrote the value.
#[derive(Default)]
pub struct SystemClipboard {
    clipboard: Option<arboard::Clipboard>,
}

impl SystemClipboard {
    fn open(&mut self) -> Result<&mut arboard::Clipboard, arboard::Error> {
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new()?);
        }
        Ok(self.clipboard.as_mut().expect("opened above"))
    }
}

impl ClipboardPort for SystemClipboard {
    fn write(&mut self, text: &str) -> anyhow::Result<()> {
        let clipboard = self.open()?;
        if text.is_empty() {
            clipboard.clear()?;
            return Ok(());
        }
        // Each platform's "this is a password" hint, so a
        // clipboard-history manager doesn't keep its own copy of the value
        // long after gage has cleared the clipboard (ADR-0035).
        let set = clipboard.set();
        #[cfg(target_os = "macos")]
        let set = arboard::SetExtApple::exclude_from_history(set);
        #[cfg(windows)]
        let set = arboard::SetExtWindows::exclude_from_monitoring(set);
        #[cfg(all(
            unix,
            not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
        ))]
        let set = arboard::SetExtLinux::exclude_from_history(set);
        set.text(text)?;
        Ok(())
    }

    fn read(&mut self) -> anyhow::Result<String> {
        Ok(self.open()?.get_text()?)
    }
}

/// One pending auto-clear. Stopping it means it will never run.
pub trait TimerHandle: Send {
    fn stop(&mut self);
}

/// Schedules a clear, behind a seam so a test fires it on demand rather
/// than by sleeping.
pub trait ClipboardTimer: Send + Sync {
    fn schedule(&self, after: Duration, f: Box<dyn FnOnce() + Send>) -> Box<dyn TimerHandle>;
}

/// The production [`ClipboardTimer`]: a thread that waits out the timeout
/// on a channel, so stopping the handle (or dropping it) ends the thread
/// without running anything.
pub struct ThreadTimer;

impl ClipboardTimer for ThreadTimer {
    fn schedule(&self, after: Duration, f: Box<dyn FnOnce() + Send>) -> Box<dyn TimerHandle> {
        let (stop, stopped) = mpsc::channel::<()>();
        // A thread that can't be started leaves the clear to the
        // session's exit, which runs whatever happens here.
        let _ = std::thread::Builder::new()
            .name("gage-clipboard-clear".into())
            .spawn(move || {
                if let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(after) {
                    f();
                }
            });
        Box::new(ThreadTimerHandle(Some(stop)))
    }
}

struct ThreadTimerHandle(Option<mpsc::Sender<()>>);

impl TimerHandle for ThreadTimerHandle {
    fn stop(&mut self) {
        // Dropping the sender disconnects the channel, which wakes the
        // thread with no timeout to act on.
        self.0.take();
    }
}

/// Where a *scheduled* clear's failure goes: the process's stderr in
/// production. A timer firing has no caller to return the error to, and
/// "your secret is still on the clipboard" is the one thing here a human
/// most needs telling. `Send`, unlike `Io::stderr`, because it is written
/// from the timer's thread.
pub type ReportSink = Box<dyn Write + Send>;

/// Whatever gage has put on the clipboard that hasn't been cleared yet.
/// There is at most one such thing: the clipboard holds one value, so a
/// second copy supersedes the first rather than queueing behind it.
///
/// What it remembers is a SHA-256 of the copied value, never the value. A
/// pending clear must not be a second plaintext copy of the secret sitting
/// in this process for the whole timeout, and a hash answers the only
/// question the clear asks: is what's on the clipboard still what gage put
/// there?
pub struct ClipboardKeeper {
    inner: Arc<Inner>,
}

struct Inner {
    timer: Box<dyn ClipboardTimer>,
    report: Mutex<Option<ReportSink>>,
    state: Mutex<State>,
}

struct State {
    port: Box<dyn ClipboardPort>,
    /// The digest of what gage wrote, while it may still be there.
    pending: Option<[u8; 32]>,
    /// Bumped by every copy. A scheduled clear carries the generation it
    /// was armed for and does nothing under a later one: a timer that had
    /// already fired, and was waiting on the lock while a second copy
    /// landed, must not wipe the second copy ahead of its own timeout.
    generation: u64,
    scheduled: Option<Box<dyn TimerHandle>>,
}

impl ClipboardKeeper {
    pub fn new(
        port: Box<dyn ClipboardPort>,
        timer: Box<dyn ClipboardTimer>,
        report: Option<ReportSink>,
    ) -> ClipboardKeeper {
        ClipboardKeeper {
            inner: Arc::new(Inner {
                timer,
                report: Mutex::new(report),
                state: Mutex::new(State {
                    port,
                    pending: None,
                    generation: 0,
                    scheduled: None,
                }),
            }),
        }
    }

    /// The keeper `main` runs with: the real clipboard, a real timer, and
    /// scheduled-clear failures reported to `stderr`.
    pub fn system(stderr: ReportSink) -> ClipboardKeeper {
        ClipboardKeeper::new(
            Box::new(SystemClipboard::default()),
            Box::new(ThreadTimer),
            Some(stderr),
        )
    }

    /// Puts `value` on the clipboard and records what to look for when
    /// clearing it. A pending clear from an earlier copy is cancelled once
    /// the write has landed: its value is no longer on the clipboard. A
    /// failed write leaves the earlier copy's clear in place, since that
    /// value is still there.
    pub fn copy(&self, value: &str) -> Result<(), CliError> {
        let mut st = self.inner.state();
        st.port.write(value).map_err(|e| {
            CliError::new(ExitCode::Internal, format!("copying to the clipboard: {e}"))
        })?;
        if let Some(mut handle) = st.scheduled.take() {
            handle.stop();
        }
        st.generation += 1;
        st.pending = Some(digest(value));
        Ok(())
    }

    /// Arranges for [`ClipboardKeeper::clear`] to run after `after` without
    /// blocking: a session's half of the clipboard decision, where the
    /// process is already long-lived and the human wants the prompt back.
    /// With nothing pending it arms nothing, so no later firing can race
    /// whatever the human copies next.
    pub fn schedule_clear(&self, after: Duration) {
        let mut st = self.inner.state();
        if st.pending.is_none() {
            return;
        }
        if let Some(mut handle) = st.scheduled.take() {
            handle.stop();
        }
        let generation = st.generation;
        let inner: Weak<Inner> = Arc::downgrade(&self.inner);
        st.scheduled = Some(self.inner.timer.schedule(
            after,
            Box::new(move || {
                if let Some(inner) = inner.upgrade() {
                    inner.scheduled_clear(generation);
                }
            }),
        ));
    }

    /// Wipes the clipboard, but only if it still holds what gage put there.
    /// The clipboard is shared state gage is a guest in: the timeout's job
    /// is to remove gage's own value, not whatever the human copied since.
    /// A clipboard that can't be read is left alone, with an error: with no
    /// way to tell whose value is on it, the safe answer is not to destroy
    /// a stranger's.
    ///
    /// Idempotent, and a no-op with nothing pending, which is what lets a
    /// session's exit path call it unconditionally.
    pub fn clear(&self) -> Result<(), CliError> {
        self.inner.clear(None)
    }

    #[cfg(test)]
    pub(crate) fn pending_digest(&self) -> Option<[u8; 32]> {
        self.inner.state().pending
    }
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn scheduled_clear(&self, generation: u64) {
        if let Err(err) = self.clear(Some(generation)) {
            let mut report = self.report.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(sink) = report.as_mut() {
                let _ = writeln!(sink, "{}", prefixed(&err));
                let _ = sink.flush();
            }
        }
    }

    fn clear(&self, only_generation: Option<u64>) -> Result<(), CliError> {
        let mut st = self.state();
        if only_generation.is_some_and(|g| g != st.generation) {
            return Ok(());
        }
        if let Some(mut handle) = st.scheduled.take() {
            handle.stop();
        }
        // Off gage's books either way. A failure below means the value may
        // still be there, which the human is told; what must not happen is
        // a later retry wiping a clipboard someone else has since taken
        // over.
        let Some(wrote) = st.pending.take() else {
            return Ok(());
        };
        let current = Zeroizing::new(st.port.read().map_err(|e| {
            CliError::new(
                ExitCode::Internal,
                format!("reading the clipboard back: {e}"),
            )
        })?);
        // Constant-time not because a timing attack is plausible here, but
        // because comparing secret-derived bytes any other way is the habit
        // that eventually gets used somewhere it does matter.
        if digest(&current).ct_eq(&wrote).unwrap_u8() != 1 {
            return Ok(());
        }
        st.port
            .write("")
            .map_err(|e| CliError::new(ExitCode::Internal, format!("clearing the clipboard: {e}")))
    }
}

fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// Blocks until `timeout` passes or the human interrupts, whichever comes
/// first. Both mean the same thing, clear now, which is why it reports
/// neither: Ctrl-C here is "I've finished pasting", not an abort, and a
/// one-shot `show -c` exits 0 either way. The channel is a parameter so
/// this is testable without a real signal.
pub fn wait_or_interrupt(interrupt: &mpsc::Receiver<()>, timeout: Duration) {
    let _ = interrupt.recv_timeout(timeout);
}

/// A one-shot `-c`'s wait: [`wait_or_interrupt`] on a real Ctrl-C. The
/// handler is process-wide and can be set once, which fits: this runs
/// only on the one-shot `-c` path, and the process clears and exits right
/// after, so the handler never outlives its use. Should it fail to
/// install, the wait still runs; only the early clear is lost.
pub fn wait_for_clipboard(timeout: Duration) {
    let (tx, rx) = mpsc::sync_channel(1);
    if ctrlc::set_handler(move || {
        let _ = tx.try_send(());
    })
    .is_err()
    {
        std::thread::sleep(timeout);
        return;
    }
    wait_or_interrupt(&rx, timeout);
}

#[cfg(test)]
pub(crate) mod fake {
    //! The clipboard seams' test doubles. Shared handles, so a test keeps
    //! one while a keeper
    //! owns the other.

    use super::*;

    #[derive(Default)]
    struct ClipState {
        content: String,
        writes: Vec<String>,
        write_err: Option<String>,
        clear_err: Option<String>,
        read_err: Option<String>,
    }

    /// An in-memory clipboard with failure switches.
    #[derive(Clone, Default)]
    pub(crate) struct FakeClipboard(Arc<Mutex<ClipState>>);

    impl FakeClipboard {
        pub(crate) fn holding(content: &str) -> FakeClipboard {
            let cb = FakeClipboard::default();
            cb.set(content);
            cb
        }

        fn lock(&self) -> MutexGuard<'_, ClipState> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner)
        }

        pub(crate) fn get(&self) -> String {
            self.lock().content.clone()
        }

        /// Someone else copying something after gage did.
        pub(crate) fn set(&self, content: &str) {
            self.lock().content = content.to_string();
        }

        /// Every successful write, the clear's `""` included.
        pub(crate) fn writes(&self) -> Vec<String> {
            self.lock().writes.clone()
        }

        pub(crate) fn fail_writes(&self, msg: &str) {
            self.lock().write_err = Some(msg.to_string());
        }

        /// Fails only the clear (a write of `""`), so a copy still lands.
        pub(crate) fn fail_clears(&self, msg: &str) {
            self.lock().clear_err = Some(msg.to_string());
        }

        pub(crate) fn fail_reads(&self, msg: &str) {
            self.lock().read_err = Some(msg.to_string());
        }
    }

    impl ClipboardPort for FakeClipboard {
        fn write(&mut self, text: &str) -> anyhow::Result<()> {
            let mut st = self.lock();
            if let Some(e) = &st.write_err {
                anyhow::bail!("{e}");
            }
            if let (true, Some(e)) = (text.is_empty(), &st.clear_err) {
                anyhow::bail!("{e}");
            }
            st.content = text.to_string();
            st.writes.push(text.to_string());
            Ok(())
        }

        fn read(&mut self) -> anyhow::Result<String> {
            let st = self.lock();
            if let Some(e) = &st.read_err {
                anyhow::bail!("{e}");
            }
            Ok(st.content.clone())
        }
    }

    #[derive(Default)]
    struct TimerState {
        after: Option<Duration>,
        f: Option<Box<dyn FnOnce() + Send>>,
        scheduled: usize,
        stopped: bool,
    }

    /// Captures a scheduled clear instead of running it.
    #[derive(Clone, Default)]
    pub(crate) struct FakeTimer(Arc<Mutex<TimerState>>);

    impl FakeTimer {
        fn lock(&self) -> MutexGuard<'_, TimerState> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner)
        }

        /// The timeout of the last clear scheduled, if any was.
        pub(crate) fn after(&self) -> Option<Duration> {
            self.lock().after
        }

        pub(crate) fn scheduled(&self) -> usize {
            self.lock().scheduled
        }

        /// Whether the last handle handed out has been stopped.
        pub(crate) fn stopped(&self) -> bool {
            self.lock().stopped
        }

        /// Runs the last scheduled clear, as the real timer would at the
        /// timeout. Outside the timer's lock, since the clear stops its own
        /// handle.
        pub(crate) fn fire(&self) {
            assert!(self.fire_if_scheduled(), "no clipboard clear was scheduled");
        }

        /// [`FakeTimer::fire`], if a clear is armed and hasn't run.
        pub(crate) fn fire_if_scheduled(&self) -> bool {
            let f = self.lock().f.take();
            match f {
                Some(f) => {
                    f();
                    true
                }
                None => false,
            }
        }
    }

    impl ClipboardTimer for FakeTimer {
        fn schedule(&self, after: Duration, f: Box<dyn FnOnce() + Send>) -> Box<dyn TimerHandle> {
            let mut st = self.lock();
            st.after = Some(after);
            st.f = Some(f);
            st.scheduled += 1;
            st.stopped = false;
            Box::new(self.clone())
        }
    }

    impl TimerHandle for FakeTimer {
        fn stop(&mut self) {
            self.lock().stopped = true;
        }
    }

    /// A [`ReportSink`] a test reads back.
    #[derive(Clone, Default)]
    pub(crate) struct SharedSink(pub(crate) Arc<Mutex<Vec<u8>>>);

    impl SharedSink {
        pub(crate) fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// One test machine's clipboard: what every run of an `Env` copies to,
    /// the timer its sessions schedule on, the one-shot waits it was asked
    /// for, and where scheduled-clear failures were reported.
    #[derive(Default)]
    pub(crate) struct Harness {
        pub(crate) cb: FakeClipboard,
        pub(crate) timer: FakeTimer,
        pub(crate) reports: SharedSink,
        waits: Mutex<Vec<Duration>>,
        #[allow(clippy::type_complexity)]
        during_wait: Mutex<Option<Box<dyn Fn(&FakeClipboard) + Send>>>,
    }

    impl Harness {
        /// A fresh keeper per run, over this machine's clipboard.
        pub(crate) fn keeper(&self) -> ClipboardKeeper {
            ClipboardKeeper::new(
                Box::new(self.cb.clone()),
                Box::new(self.timer.clone()),
                Some(Box::new(self.reports.clone())),
            )
        }

        /// The one-shot wait: recorded, never slept.
        pub(crate) fn wait(&self, d: Duration) {
            self.waits.lock().unwrap().push(d);
            if let Some(f) = self.during_wait.lock().unwrap().as_ref() {
                f(&self.cb);
            }
        }

        pub(crate) fn waits(&self) -> Vec<Duration> {
            self.waits.lock().unwrap().clone()
        }

        /// Runs `f` while a one-shot `-c` waits out its timeout: the window
        /// someone else might copy something in.
        pub(crate) fn during_wait(&self, f: impl Fn(&FakeClipboard) + Send + 'static) {
            *self.during_wait.lock().unwrap() = Some(Box::new(f));
        }
    }

    pub(crate) fn keeper(cb: &FakeClipboard, timer: &FakeTimer) -> ClipboardKeeper {
        ClipboardKeeper::new(Box::new(cb.clone()), Box::new(timer.clone()), None)
    }

    /// A keeper for harnesses that never copy: anything that reaches its
    /// clipboard lands in a fake nobody reads.
    pub(crate) fn unused() -> ClipboardKeeper {
        keeper(&FakeClipboard::default(), &FakeTimer::default())
    }
}
