//! What kind of failure a library error is, and what would fix it, said
//! without a frontend's vocabulary (ADR-0054). Every error enum implements
//! [`HasKind`]; a frontend decides what a kind becomes (`gage-cli` maps it
//! to an exit code, ADR-0037) and how a [`Remedy`] is offered (`gage-cli`
//! names the command to run; a GUI might show a button).

/// The kinds of failure a frontend treats differently. There is no
/// success kind: a kind describes an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// A state a human must resolve: a sync divergence, a recipient-file
    /// mismatch, a dirty `entries/` working tree, and so on.
    Conflict,
    /// The request itself was invalid: an unknown type or method, a
    /// mutually exclusive combination, a name that can't be used.
    Usage,
    /// A query resolved to no entry, or a name to no vault.
    NotFound,
    /// A query resolved to more than one candidate and there was nobody to
    /// ask.
    Ambiguous,
    /// An unlock failed, or a remote refused access: a wrong passphrase, no
    /// local identity for this device, an expired auth token.
    LockedOrAuth,
    /// An unexpected error that fits none of the other kinds.
    Internal,
    /// A remote could not be reached at all: DNS failure, connection
    /// refused, a timeout. Deliberately not `Conflict`: nothing about
    /// either side's state is wrong, so the same request is worth retrying
    /// unchanged once the network is back.
    Unreachable,
}

impl ErrorKind {
    /// Every kind, in a fixed order.
    pub const ALL: [ErrorKind; 7] = [
        ErrorKind::Conflict,
        ErrorKind::Usage,
        ErrorKind::NotFound,
        ErrorKind::Ambiguous,
        ErrorKind::LockedOrAuth,
        ErrorKind::Internal,
        ErrorKind::Unreachable,
    ];
}

/// What the person can do next about an error or a warning: an action, as
/// data. The library never spells it as a command line; the frontend does.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Remedy {
    /// Sync the vault with its remote, resolving what diverged.
    Sync,
    /// Run the sync again: some conflicts were left unresolved.
    SyncAgain,
    /// Check the vault's two recipient files against each other.
    RecipientVerify,
    /// Rewrite `.age-recipients` from `.gage/config.toml`, reporting what
    /// it drops.
    RecipientRepair,
    /// Store a token for `host`: an existing one may have expired or been
    /// revoked.
    AuthLogin { host: String },
    /// Store `host`'s token again, or fix the token file's permissions.
    AuthLoginAgain { host: String },
    /// Use the remote's HTTPS URL with a stored token instead of SSH.
    UseHttps,
    /// Forget the registration `name` and clone the vault again.
    ReRegister { name: String },
    /// Ask to join the vault from this device, or enroll with the
    /// recovery key.
    EnrollOrRecover,
    /// Configure a remote for the vault.
    SetRemote,
    /// Forget `vault` here, clone it again, then ask to join it.
    Reclone { vault: String },
    /// Ask to join the vault again; after storing a token first when the
    /// remote refused access, for `host` if it is known.
    EnrollAgain {
        auth_refused: bool,
        host: Option<String>,
    },
    /// Push what was committed locally.
    Push,
    /// Ask again, allowing a duplicate title.
    Force,
    /// Choose another device name.
    ChooseDevice,
    /// Approve under another device name.
    ApproveWithDevice,
    /// Name the vault explicitly; `plain` when the given one had to be a
    /// plain name.
    PassName { plain: bool },
    /// Run where someone can answer, or pre-approve the answer.
    Yes,
}

/// An error that knows its [`ErrorKind`], and the [`Remedy`] for it if
/// there is one. An error that wraps another passes both through, unless
/// it knows better.
pub trait HasKind {
    fn kind(&self) -> ErrorKind;

    fn remedy(&self) -> Option<Remedy> {
        None
    }
}
