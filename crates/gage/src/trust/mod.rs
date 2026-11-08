//! Who a vault trusts: the recipient list, the trust cache that notices it
//! changing, and orphan assessment.

pub mod keycontinuity;
pub mod orphan;
pub mod recipient;
pub mod trustcache;

#[cfg(test)]
mod keycontinuity_tests;
#[cfg(test)]
pub(crate) mod trustcache_tests;
