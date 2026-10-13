//! A vault's unlocked private key on this device.
//!
//! `Identity` is a parameter, never internal state: `Vault::unlock` is the
//! only thing that produces one, and every CRUD method takes it explicitly,
//! so `Vault` stays stateless and identity-agnostic. M2 gives it key
//! material, zeroized and page-unlocked in `Drop` (there is no separate
//! close to forget). M0b fixes only the shape.

/// An unlocked identity. It can't be constructed outside the library.
#[derive(Debug)]
pub struct Identity {
    _private: (),
}
