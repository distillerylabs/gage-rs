//! The seam between `Vault::unlock`/`Identity`'s `Drop` and the platform's
//! page-locking syscalls. Production and every realistic test use the
//! real, `memlock`-backed [`MemLocker`]; a fake returning a deterministic
//! failure exists for the tests proving a page-lock failure warns and
//! proceeds rather than aborting the unlock, and a counting one for the
//! tests proving `Drop` releases exactly what was locked.

use std::any::Any;
use std::io;

use crate::memlock;

/// Pins and releases the pages holding key material.
pub trait Locker: Any + Send + Sync {
    fn lock(&self, buf: &[u8]) -> io::Result<()>;
    fn unlock(&self, buf: &[u8]) -> io::Result<()>;
}

/// The real implementation: a trivial adapter over `memlock`, which is
/// where the platform-specific code lives.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemLocker;

impl Locker for MemLocker {
    fn lock(&self, buf: &[u8]) -> io::Result<()> {
        memlock::lock(buf)
    }

    fn unlock(&self, buf: &[u8]) -> io::Result<()> {
        memlock::unlock(buf)
    }
}
