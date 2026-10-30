//! Keys and ciphertext: age keys, encryption, the in-memory identity, page
//! locking and value generation.

#[cfg(any(test, feature = "test-support"))]
pub mod agekey;
#[cfg(not(any(test, feature = "test-support")))]
pub(crate) mod agekey;
pub mod crypt;
pub mod generate;
pub mod identity;
#[cfg(any(test, feature = "test-support"))]
pub mod locker;
#[cfg(not(any(test, feature = "test-support")))]
pub(crate) mod locker;
pub(crate) mod memlock;
