//! Page-locks buffers holding key material so the OS can't write them to
//! swap, and releases those locks again: `mlock(2)`/`munlock(2)` on Linux
//! and macOS, `VirtualLock`/`VirtualUnlock` on Windows, behind one
//! signature. See "Session model" in the design doc.
//!
//! Locking is best-effort by design, not by accident. It fails routinely
//! under a restrictive `RLIMIT_MEMLOCK` (`ulimit -l`), in containers, and
//! under locked-down Windows policy, and the design's answer there is to
//! warn once and proceed rather than refuse to unlock: "a weaker guarantee
//! beats an unusable tool." This module therefore reports failures
//! faithfully and leaves the warn-or-fail decision to its caller (see
//! `locker` and `Vault::unlock`).
//!
//! The one module in the library allowed `unsafe`: an allocator with a
//! page-sized alignment and two syscalls.

#![allow(unsafe_code)]

use std::alloc::{self, Layout};
use std::io;
use std::ptr::NonNull;

use zeroize::Zeroize;

/// Pins `buf`'s pages in physical memory. An empty slice is a no-op
/// success: there is nothing to pin, and the platform calls disagree about
/// whether a zero-length range is an error.
pub fn lock(buf: &[u8]) -> io::Result<()> {
    if buf.is_empty() {
        return Ok(());
    }
    sys::lock(buf)
}

/// Releases a lock taken by [`lock`]. Like `lock`, an empty slice is a
/// no-op success. Unlocking a buffer that was never locked is the caller's
/// error to avoid: this module doesn't track lock state, because the thing
/// that owns that state (an `Identity`) is where it can be reasoned about.
pub fn unlock(buf: &[u8]) -> io::Result<()> {
    if buf.is_empty() {
        return Ok(());
    }
    sys::unlock(buf)
}

/// The OS page size: the unit both lock calls work in.
pub fn page_size() -> usize {
    sys::page_size()
}

/// A fixed-length byte buffer that occupies whole pages no other
/// allocation shares, for a caller that intends to [`lock`] it. Zeroed
/// before it is freed.
///
/// This exists because neither `mlock` nor `VirtualLock` is
/// reference-counted and both operate on whole pages. Locking a 74-byte
/// key pins the entire page it sits on, and unlocking it releases that
/// page for everything else living there, so two keys sharing a page share
/// one lock and the first unlock drops the other's protection. The general
/// allocator puts two small allocations on one page as a matter of course.
/// `munlock` reports success while losing the second key's protection in
/// silence; `VirtualUnlock` fails the second call with `ERROR_NOT_LOCKED`,
/// a confusing error on the innocent caller.
///
/// The length is fixed at construction and there is no way to grow it, so
/// key material can't spill into memory that was never locked.
pub struct LockedBuf {
    ptr: NonNull<u8>,
    len: usize,
    layout: Layout,
}

// SAFETY: LockedBuf owns its allocation exclusively, like a Box<[u8]>.
unsafe impl Send for LockedBuf {}
// SAFETY: shared access is read-only (&self hands out &[u8]).
unsafe impl Sync for LockedBuf {}

impl LockedBuf {
    /// A zero-filled buffer of `len` bytes, starting on a page boundary
    /// and rounded out to whole pages (at least one).
    pub fn zeroed(len: usize) -> LockedBuf {
        let page = page_size();
        let size = len.div_ceil(page).max(1) * page;
        let layout = Layout::from_size_align(size, page).expect("page size is a power of two");
        // SAFETY: layout has a non-zero size.
        let raw = unsafe { alloc::alloc_zeroed(layout) };
        let ptr = NonNull::new(raw).unwrap_or_else(|| alloc::handle_alloc_error(layout));
        LockedBuf { ptr, len, layout }
    }

    /// A buffer holding a copy of `bytes`.
    pub fn copy_from(bytes: &[u8]) -> LockedBuf {
        let mut buf = LockedBuf::zeroed(bytes.len());
        buf.as_mut_slice().copy_from_slice(bytes);
        buf
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: ptr is valid for layout.size() >= len bytes, initialized
        // (alloc_zeroed), and borrowed for &self's lifetime.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as as_slice, with exclusive access through &mut self.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }

    /// The whole allocation, slack included: what is wiped on drop, so
    /// nothing written past `len` by a bug survives either.
    fn allocation_mut(&mut self) -> &mut [u8] {
        // SAFETY: ptr is valid for layout.size() bytes, exclusively owned.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) }
    }
}

impl Drop for LockedBuf {
    fn drop(&mut self) {
        self.allocation_mut().zeroize();
        // SAFETY: ptr came from alloc_zeroed with this exact layout.
        unsafe { alloc::dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

// Never print key material, not even in a test failure.
impl std::fmt::Debug for LockedBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LockedBuf({} bytes, <redacted>)", self.len)
    }
}

#[cfg(unix)]
mod sys {
    use std::io;

    pub fn lock(buf: &[u8]) -> io::Result<()> {
        // SAFETY: the range is a live, non-empty borrow for the call's
        // duration; mlock only reads the address range.
        check(unsafe { libc::mlock(buf.as_ptr().cast(), buf.len()) })
    }

    pub fn unlock(buf: &[u8]) -> io::Result<()> {
        // SAFETY: as lock.
        check(unsafe { libc::munlock(buf.as_ptr().cast(), buf.len()) })
    }

    pub fn page_size() -> usize {
        // SAFETY: sysconf has no preconditions.
        let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        usize::try_from(size).expect("the OS reports a positive page size")
    }

    fn check(rc: libc::c_int) -> io::Result<()> {
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// The platform saying "you may not lock this much", rather than a
    /// wrong call: `ENOMEM` is the `RLIMIT_MEMLOCK` ceiling (containers, a
    /// low `ulimit -l`); `EPERM` is the same refusal on kernels that report
    /// it that way for an unprivileged process.
    #[cfg(test)]
    pub fn resource_limited(err: &io::Error) -> bool {
        matches!(err.raw_os_error(), Some(libc::ENOMEM | libc::EPERM))
    }
}

#[cfg(windows)]
mod sys {
    use std::io;

    use windows_sys::Win32::System::Memory::{VirtualLock, VirtualUnlock};
    use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};

    pub fn lock(buf: &[u8]) -> io::Result<()> {
        // SAFETY: the range is a live, non-empty borrow for the call's
        // duration.
        check(unsafe { VirtualLock(buf.as_ptr().cast(), buf.len()) })
    }

    pub fn unlock(buf: &[u8]) -> io::Result<()> {
        // SAFETY: as lock.
        check(unsafe { VirtualUnlock(buf.as_ptr().cast(), buf.len()) })
    }

    pub fn page_size() -> usize {
        // SAFETY: GetSystemInfo fills the struct it is handed and cannot
        // fail.
        let info = unsafe {
            let mut info: SYSTEM_INFO = std::mem::zeroed();
            GetSystemInfo(&mut info);
            info
        };
        info.dwPageSize as usize
    }

    fn check(ok: windows_sys::core::BOOL) -> io::Result<()> {
        if ok != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Windows saying the process's working-set quota won't allow another
    /// locked page, rather than a wrong call.
    #[cfg(test)]
    pub fn resource_limited(err: &io::Error) -> bool {
        use windows_sys::Win32::Foundation::{
            ERROR_NOT_ENOUGH_MEMORY, ERROR_NOT_ENOUGH_QUOTA, ERROR_WORKING_SET_QUOTA,
        };
        matches!(
            err.raw_os_error().map(|c| c as u32),
            Some(ERROR_WORKING_SET_QUOTA | ERROR_NOT_ENOUGH_MEMORY | ERROR_NOT_ENOUGH_QUOTA)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An age X25519 secret key's Bech32 line: the shape of the buffer
    /// this module actually protects.
    const KEY_SIZED: usize = 74;

    /// Skips (by returning true) only when the platform refused for a
    /// resource reason; any other error fails, so "we called mlock wrong"
    /// can't hide behind the skip "this CI runner won't allow locking"
    /// legitimately needs.
    fn refused_for_resources(result: io::Result<()>, what: &str) -> bool {
        match result {
            Ok(()) => false,
            Err(e) if sys::resource_limited(&e) => {
                #[allow(clippy::disallowed_macros)]
                {
                    eprintln!(
                        "skipping: this platform refuses to lock pages for a resource reason ({e}); \
                         gage's answer to that is to warn once and proceed, tested in Vault::unlock"
                    );
                }
                true
            }
            Err(e) => panic!("{what}: {e}"),
        }
    }

    fn page_of(buf: &[u8]) -> usize {
        buf.as_ptr() as usize & !(page_size() - 1)
    }

    // Known limit: no portable "is this page locked?" query exists, so this proves
    // the calls are reached, accept a key-sized buffer and don't disturb
    // it, not that the kernel pinned anything.
    #[test]
    fn lock_unlock_round_trip() {
        let mut buf = LockedBuf::zeroed(KEY_SIZED);
        for (i, b) in buf.as_mut_slice().iter_mut().enumerate() {
            *b = i as u8;
        }
        let want = buf.as_slice().to_vec();

        if refused_for_resources(lock(buf.as_slice()), "lock") {
            return;
        }
        assert_eq!(buf.as_slice(), want, "lock altered the buffer");
        unlock(buf.as_slice()).expect("unlock after a successful lock");
        assert_eq!(buf.as_slice(), want, "unlock altered the buffer");
    }

    #[test]
    fn empty_slice_is_a_no_op() {
        lock(&[]).unwrap();
        unlock(&[]).unwrap();
        let empty = LockedBuf::zeroed(0);
        assert!(empty.is_empty());
        lock(empty.as_slice()).unwrap();
        unlock(empty.as_slice()).unwrap();
    }

    // Regression: two buffers once shared a page lock. LockedBuf has a
    // fixed length and no way to grow, so there is no capacity cap to check.
    #[test]
    fn alloc_gives_each_buffer_its_own_page() {
        let page = page_size();
        assert!(KEY_SIZED <= page, "this test assumes a key fits in a page");
        let bufs: Vec<LockedBuf> = (0..8).map(|_| LockedBuf::zeroed(KEY_SIZED)).collect();
        let mut pages = std::collections::HashSet::new();
        for (i, buf) in bufs.iter().enumerate() {
            assert_eq!(buf.len(), KEY_SIZED);
            assert_eq!(
                buf.as_slice().as_ptr() as usize % page,
                0,
                "buffer {i} is not page-aligned"
            );
            assert!(
                pages.insert(page_of(buf.as_slice())),
                "buffer {i} shares a page"
            );
        }
        // Each owns a whole page, so its allocation can't reach into a
        // page another buffer holds.
        assert!(bufs.iter().all(|b| b.layout.size() == page));
    }

    // Two identities held
    // at once, where closing the first made closing the second fail on
    // Windows before each key got its own page.
    #[test]
    fn unlocking_one_buffer_leaves_another_locked() {
        let (first, second) = (LockedBuf::zeroed(KEY_SIZED), LockedBuf::zeroed(KEY_SIZED));
        if refused_for_resources(lock(first.as_slice()), "lock(first)") {
            return;
        }
        lock(second.as_slice()).expect("lock(second) with another key locked");
        unlock(first.as_slice()).expect("unlock(first)");
        unlock(second.as_slice())
            .expect("unlock(second) after unlocking an unrelated key: the two shared a page lock");
    }

    #[test]
    fn copy_from_holds_the_bytes_and_debug_redacts_them() {
        let buf = LockedBuf::copy_from(b"AGE-SECRET-KEY-1XYZ");
        assert_eq!(buf.as_slice(), b"AGE-SECRET-KEY-1XYZ");
        assert!(!format!("{buf:?}").contains("SECRET"), "{buf:?}");
    }

    #[test]
    fn a_multi_page_buffer_rounds_out_to_whole_pages() {
        let page = page_size();
        let buf = LockedBuf::zeroed(page + 1);
        assert_eq!(buf.layout.size(), 2 * page);
        assert_eq!(buf.len(), page + 1);
    }
}
