//! The pending filename, `<request-id>-<expires-epoch>.age`, and the expiry
//! rule shared by everything that reads one. See ADR-0042.

use jiff::{SignedDuration, Timestamp};

use super::MAX_ENROLLMENT_TTL;
use crate::storage::vaultconfig;

/// The extension every sealed request carries.
pub(crate) const PENDING_FILE_EXT: &str = ".age";

/// A request's filename.
///
/// The device name is deliberately absent: naming the file after the device
/// would hand every reader of the vault a free "someone is setting up a new
/// machine" signal before anyone approved anything. The epoch is readable,
/// the one exception to "a filename never encodes content", because it lets
/// listing show a real expiry and pruning run as a string parse.
pub(crate) fn pending_file_name(id: &str, expires: Timestamp) -> String {
    format!("{id}-{}{PENDING_FILE_EXT}", expires.as_second())
}

/// A pending filename split back into its id and expiry, or `None` for
/// anything gage didn't write.
///
/// The halves are validated independently: `pending/` is a directory any
/// git writer can put files into, so an unparseable name is skipped (and
/// never pruned) rather than guessed at. The epoch starts at the last
/// hyphen, which is unambiguous because a canonical UUID's hyphens all
/// precede it and the epoch is digits only; a signed or padded spelling is
/// refused rather than normalized.
pub(crate) fn parse_pending_file_name(name: &str) -> Option<(String, Timestamp)> {
    let stem = name.strip_suffix(PENDING_FILE_EXT)?;
    let (id, epoch) = stem.rsplit_once('-')?;
    if !vaultconfig::valid_id(id) || epoch.is_empty() || !epoch.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    // Past i64 doesn't parse, and is a stray: never listed or
    // counted, just never pruned either.
    let seconds: i64 = epoch.parse().ok()?;
    // Past what a timestamp can hold is past the ceiling too: a forged
    // epoch is expired, not a stray, or pruning would never remove it.
    let expires = Timestamp::from_second(seconds).unwrap_or(Timestamp::MAX);
    Some((id.to_string(), expires))
}

/// Whether gage will honour `expires` at `now`: in the future, and no
/// further into it than gage could ever have written.
///
/// The two halves are the same rule from opposite ends, which is why they
/// sit in one function. It is applied to the filename's unauthenticated
/// epoch (listing, pruning) and to the sealed copy (opening), and must agree
/// in both or a request would list as live and refuse to open, or the
/// reverse. The ceiling half is what stops a hostile git writer parking
/// `<uuid>-99999999999.age` in the tree forever.
pub(crate) fn expiry_live(expires: Timestamp, now: Timestamp) -> bool {
    let ceiling = SignedDuration::try_from(MAX_ENROLLMENT_TTL).expect("seven days fits");
    let latest = now.checked_add(ceiling).unwrap_or(Timestamp::MAX);
    expires > now && expires <= latest
}
