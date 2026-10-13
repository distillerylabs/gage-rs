//! A vault's committed `.gage/config.toml`. M0b brings only the vault id;
//! the file's schema arrives with vault lifecycle in M1.

mod id;

pub use id::{new_id, valid_id};
