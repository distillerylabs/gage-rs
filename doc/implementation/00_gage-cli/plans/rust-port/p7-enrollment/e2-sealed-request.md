# E2 — The sealed request

[← E1b](e1b-shared-recipient-write.md) · [P7 plan](index.md) · [#65](https://github.com/distillerylabs/gage-rs/issues/65) · next: [E3 — Joining a vault](e3-joining-side.md)

> **Recommended model: Opus ⚑.** The code is an offline-attackable secret,
> and the seal is what makes the whole feature mean anything. Entropy, the
> generation source, the AEAD semantics and the work-factor bounds are
> where being subtly wrong is invisible to a passing test. Run an **Opus
> review of the seal, code, bound and inertness rows** (task 8) before
> calling it done, in the same spirit as M2.

## Goal

The enrollment primitives, at the library level, with **no command wired
up**. Generate a code, seal a request to it, name the file, open it again,
decide whether it has expired, resolve an id, and prune. Bytes and files
in, typed values out.

- **The code:** 16 Crockford base32 characters (80 bits) from `OsRng`,
  shown as `GAGE-XXXX-XXXX-XXXX-XXXX`. It is normalized and validated
  before any decryption.
- **The seal:** a TOML payload (`request_id`, `vault_id`, `device`,
  `pubkey`, `method`, `created`, `expires`), encrypted to one scrypt
  recipient at the enrollment work factor, 14.
- **The file:** `.gage/pending/<request-id>-<expires-epoch>.age`.
- **`Vault::pending_enrollments()`:** live requests only, from filenames
  alone. It needs no identity, reads no git history, and treats a missing
  directory as zero requests.
- **`Vault::open_enrollment(requests, codes)`:** opens only the requests
  it is handed. It refuses more than 32 before any decryption, caps a
  claimed work factor, bounds file and payload size, and validates every
  payload field and the vault id before returning an `OpenedRequest`.
- **`Vault::resolve_enrollment(id)`:** exact UUID, then substring, then a
  candidate list, over the UUID portion of the filename only.
- **Pruning:** removes files expired by their filename epoch, or claiming
  an epoch beyond the ceiling. A name that doesn't parse is left alone.
- **`pending/` is inert.** Nothing reads it at encryption time.

`Vault::enroll` is E3's. `approve_enrollments` and `deny_enrollment` are
E4's. E4 wires this milestone's three functions into commands and
reimplements none of them.

## Depends on

- **P2 · M2:** `crypt` (the age wrapper, `Recipient::passphrase_at`, the
  single scrypt identity constructor, `SCRYPT_MAX_WORK_FACTOR`, the
  work-factor hook and its guard tests), `agekey`.
- **P1:** `vaultconfig` (`[vault].id`), `devicename::valid`, `atomicfile`,
  `exitcode`, `paths`.
- **P5:** `recipient list` / `recipient verify` (the inertness rows run
  them over a vault with pending requests).

## Design references

Init TDD sections:

- "The reframe: the seal is authentication, not confidentiality"
- `D-ENROLL-CODE-FORMAT`
- `D-ENROLL-SEAL-COST`, including "Seals are written at work factor 14,
  not 19"
- `D-ENROLL-TTL`
- `D-ENROLL-EXPIRY-IN-NAME`
- "The sealed payload is untrusted input"
- "The request names the vault it is for"
- "A sealed expiry beyond the ceiling is expired, not refused"
- "The load-bearing invariant: `pending/` is inert"
- "Expiry, revocation, and pruning"
- "Clock skew, in the direction the ceiling doesn't cover"
- "Library surface": the error list and its exit-code table

## Go sources

- Go plan `gage-cli-init-design/e2-sealed-request.md`: all of it.
- Parity ledger: §2 `798ed47`; §2 `e52828a` (the enrollment halves of the
  scrypt hook); §3 E2.
- Go sources at the pinned SHA: `internal/gage/enrollcode.go`,
  `enrollseal.go`, `enrollment.go`, and the enrollment half of
  `crypt.go`.
- Test inventory, 55 rows:
  - `enrollcode_test.go` (7)
  - `enrollseal_test.go` (18)
  - `enrollpayload_test.go` (6)
  - `enrollexpiry_test.go` (6)
  - `enrollinert_test.go` (4)
  - `enrollname_test.go` (4)
  - `enrollment_test.go` (9)
  - `enrollprune_test.go` `TestPruningLeavesLiveRequestsAlone` (1)
- Inbound, 2 rows from P2: the enrollment halves of `crypt`
  `TestOnlyTheTestHooksWriteTheWorkFactors` and
  `TestEveryScryptIdentityCapsItsWorkFactor`.

## Decisions to make first

[Q-ENROLL-SCRYPT-HOOK](../open-questions.md#q-enroll-scrypt-hook) and
[Q-CROCKFORD](../open-questions.md#q-crockford) settle the shape. The
rest were resolved at the start of the milestone, from Go's
`enrollcode.go`, `enrollseal.go` and `enrollment.go` at the pinned SHA.
All were reviewed before any code. Where an answer departs from Go,
the bullet says so.

- **Module layout [resolved].** A new `crates/gage/src/enrollment/`
  module:
  - `code.rs`: generation, normalization and validation.
  - `seal.rs`: the payload, sealing, one open attempt, and payload
    validation.
  - `name.rs`: filename build and parse, `canonical_uuid`, and
    `expiry_live`.
  - `mod.rs`: the constants, the types, `EnrollmentError`, and the
    `Vault` methods (listing, opening, resolution, pruning).

  The tests follow Go's files, so each inventory row has an obvious home:
  `code_tests.rs`, `seal_tests.rs`, `payload_tests.rs`, `expiry_tests.rs`,
  `inert_tests.rs`, `name_tests.rs` and `listing_tests.rs` (Go's
  `enrollment_test.go` plus the prune row). Their shared fixtures
  (`seal_into`, `hand_seal`, `good_payload`, `write_pending`, `open_all`,
  `assert_enrollment_error`) live in `enrollment/fixtures_tests.rs`.
  The `_tests.rs` suffix keeps them out of `crypt`'s source scans.
- **Types and signatures [resolved].** Go's names in Rust form.
  Timestamps are `jiff::Timestamp`, which the crate already uses for
  entries. The TTL is a `std::time::Duration`, because E3's `--ttl` goes
  through `shell.rs`'s `parse_duration`, and that returns one.
  - `PendingRequest { id: String, path: PathBuf, expires: Timestamp }`:
    built from the filename. `expires` is unauthenticated.
  - `OpenedRequest { id, device, pubkey, method: String, created,
    expires: Timestamp }`. Flat, as in Go and the TDD's library surface.
    It doesn't nest the `PendingRequest`. It has no vault-id field.
  - `EnrollmentRequest { id, device, pubkey, method, created, expires,
    code: Zeroizing<String>, published: bool }`. It has all of Go's
    fields, because the TDD fixes its shape. Nothing in E2 sets
    `published`.
  - `pub fn pending_enrollments(&self) -> Result<Vec<PendingRequest>, EnrollmentError>`
  - `pub fn open_enrollment(&self, requests: &[PendingRequest], codes: &[Zeroizing<String>]) -> Result<Vec<OpenedRequest>, EnrollmentError>`
  - `pub fn resolve_enrollment(&self, id: &str) -> Result<PendingRequest, EnrollmentError>`
  - `pub(crate) fn seal_enrollment(&self, device, pubkey, method, ttl) -> Result<(EnrollmentRequest, Vec<u8>), EnrollmentError>`.
    Crate-private, because E3's `enroll` is its only caller. The
    normalized code it passes to age is `Zeroizing` as well.
  - `pub(crate) fn prune_pending_enrollments(&self) -> Result<Vec<String>, EnrollmentError>`.
    Crate-private, because E4 wires it into the recipient writes.

  Codes travel as `Zeroizing<String>`, as passphrases do. None of the
  three public methods takes an `Identity`, which
  `pending_directory_needs_no_identity` pins.

  **TTL deviation, recorded here:** a negative `std::time::Duration`
  can't be built, so `seal_rejects_ttls_outside_the_ceiling` covers zero
  and the ceiling. A negative `--ttl` is refused when E3 parses it
  (`identity_enroll_refuses_an_unusable_ttl`).
- **`EnrollmentError` [resolved].** One `#[non_exhaustive]` `thiserror`
  enum. Its variants and codes follow the TDD's table and Go's `exitcode`
  wrapping:

  | Variant | Code | Raised by |
  |---|---|---|
  | `CodeWrong` | `LockedOrAuth` | `open_enrollment`. Covers a code that opens nothing and a code that fails validation, deliberately. |
  | `Expired { expires }` | `Conflict` | open (the sealed expiry, after the ceiling clamp) |
  | `ClockSkew { created, expires }` | `Conflict` | open |
  | `IdMismatch { sealed, filed }` | `Conflict` | open |
  | `MalformedRequest { reason }` | `Conflict` | open. Also covers a decrypted payload over the limit. |
  | `WrongVault { vault, this }` | `Conflict` | open. `vault` is `"work" (id …)` from this device's registry, or `the vault with id …`. `this` is the local name of the vault being opened. |
  | `TooManyPending { pending, limit }` | `Conflict` | open, before any decryption. The message names approving or denying by id. |
  | `NoSuchRequest { id }` | `NotFound` | resolve |
  | `Ambiguous(RequestCandidates { id, matches: Vec<PendingRequest> })` | `Ambiguous` | resolve ([Q-ENROLL-AMBIGUOUS](../open-questions.md#q-enroll-ambiguous)) |
  | `InvalidTtl { ttl }` | `Usage` | seal: zero or above the ceiling |
  | `InvalidDevice { device }`, `InvalidPubkey(agekey::Error)`, `UnknownMethod { method }` | `Usage` | seal (Go's `sealEnrollment` checks) |
  | `NoVaultId` | `Internal` | seal, when the vault's id isn't a canonical UUID |
  | `Random(rand_core::Error)`, `CodeMismatch`, `Encode(String)`, `Seal(crypt::Error)`, `Io { path, source }` | `Internal` | the OS RNG, the generator/normalizer disagreeing (Go's unreachable check), TOML encoding, age, and reading or pruning `pending/` |

  E3 and E4 add their own variants. `WrongVault`'s registry lookup reads
  global config through `self.dirs`, as `unlock` does. If the registry is
  missing or unreadable, the message loses the name and nothing else.
- **Scrypt construction under P2's guards [resolved].**
  - `SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR: u8 = 14` lives in `crypt.rs`
    next to `SHIPPED_SCRYPT_WORK_FACTOR`, named as its twin is, so the source-scan guard is extended in
    place. Its comment carries Go's D-ENROLL-SEAL-COST reasoning and the
    coupling to the code's 80 bits.
  - It has a live `AtomicU8` (`ENROLLMENT_SCRYPT_WORK_FACTOR`),
    `enrollment_scrypt_work_factor()`, and
    `set_enrollment_work_factor_for_tests` under `cfg(any(test, feature =
    "test-support"))`. `fast_scrypt()` lowers both factors to
    `TEST_SCRYPT_WORK_FACTOR`.
  - Sealing calls `Recipient::passphrase_at(code,
    enrollment_scrypt_work_factor())`, which becomes `pub(crate)`. That
    keeps the one `scrypt::Recipient::new` site.
  - Opening goes through `crypt::scrypt_identity`, the one capped
    constructor, at the enrollment cap. age-rust's `scrypt::Identity::new` runs a calibration
    pass at 2^10 (about a millisecond), so building one per attempt, as
    Go does, costs nothing worth avoiding.
  - The shipped-factor row runs in a child process that never called the
    hook, as `shipped_work_factor_reaches_a_real_age_file` already does.
    Go lowers and then restores a global, which would race the other
    tests here.
  - `wrong_code_against_a_full_directory_is_fast` and
    `enrollment_and_identity_work_factors_are_independent` run in child
    processes too. The first needs the shipped factor. The second moves
    both live factors, and in-process that would race every test reading
    them, such as `scrypt_work_factor_is_actually_applied`.
  - **The cap [resolved, departs from Go:
    [Q-ENROLL-SCRYPT-CAP](../open-questions.md#q-enroll-scrypt-cap)].**
    The open path caps a claimed factor at
    `ENROLLMENT_SCRYPT_MAX_WORK_FACTOR = 18`, not the identity file's 22.
    Go uses 22 for both. With r=8, scrypt needs 1 KiB × N of memory, so
    one `pending/` file claiming 22 costs 4 GiB and about 8× a factor-19
    unlock, and any git writer can commit 32 of them. 18 is four
    doublings above what gage seals at, the same headroom age's own
    default cap leaves. At 18, a hostile file costs 256 MiB and ~0.35 s.
    `scrypt_identity` takes the cap as a parameter, and the
    every-identity-caps guard accepts only the two named cap constants.
- **Decrypt-count instrumentation [resolved].** `Vault` gets a second
  `DecryptCounter`, `enrollment_decrypts`, reusing the existing
  zero-sized-outside-tests type. Each open attempt bumps it just before
  age runs, and a `cfg(test)` `enrollment_decrypt_count()` reads it. It is
  observation, not a seam: nothing can be faked through it.
- **The size bounds [resolved].** `MAX_PENDING_REQUEST_BYTES = 16 * 1024`
  is checked in three places:
  - **Listing:** from the directory entry's metadata, before any read.
  - **Opening:** by reading at most limit+1 bytes of each file it is
    handed, since the caller may not have listed.
  - **The decrypted payload:** by reading at most limit+1 bytes from
    age's reader.

  The third needs `crypt::decrypt_with_limit`. `decrypt_with` becomes the
  unlimited case of it, so there is still one place ciphertext becomes
  plaintext. Over-limit is reported as its own outcome, not as an error,
  so the caller maps it to `MalformedRequest`. The open-attempt function
  returns Go's three outcomes as an enum: `NotOpened`,
  `Opened(OpenedRequest)`, `Refused(EnrollmentError)`.
- **The clock [resolved]: Go's approach.** The public methods read
  `jiff::Timestamp::now()` once per call. The crate-private validator and
  the open attempt take `now` as a parameter, as Go's do. Tests build
  payloads relative to the real now, and no row needs an injected clock.
- **The code's randomness [resolved].** Ten bytes from `OsRng`
  (`rand_core`, as `generate` uses), split into sixteen 5-bit groups.
  There's no rejection sampling, because 32 divides evenly. Generation
  takes no RNG parameter. The fixed-seed mutation in task 8 is killed by
  `successive_codes_differ`, and if that turns out to be too weak, by a
  source scan like `generate`'s.
- **Payload parsing [resolved].** Every payload field is a `String` with
  `#[serde(default)]`, as in Go. An absent field then reaches its own
  validation check, so an absent `vault_id` is reported as "not a UUID",
  not as "not TOML". UUIDs must be canonical (lowercase 8-4-4-4-12, not
  nil), via the `uuid` crate's parse and re-print. Timestamps are parsed
  as RFC 3339 into `jiff::Timestamp` and written as UTC whole seconds.

## Tasks

Write each task's tests first, at task granularity.

### 1. The code — 7 rows

- [x] Generation, normalization and validation (`enrollment/code.rs`).
- [x] Rows:
  - `TestGeneratedCodeHasTheDocumentedShape`: `generated_code_has_the_documented_shape`
  - `TestGeneratedCodesAvoidTheAmbiguousLetters`: `generated_codes_avoid_the_ambiguous_letters`. Over many generations: never `I`, `L`, `O` or `U`.
  - `TestGeneratedCodesNormalizeToThemselves`: `generated_codes_normalize_to_themselves`
  - `TestSuccessiveCodesDiffer`: `successive_codes_differ`
  - `TestCodeNormalizationAcceptsHowPeopleRetypeCodes`: `code_normalization_accepts_how_people_retype_codes`. Lowercase, spaces for hyphens, a missing `GAGE-`.
  - `TestCodeNormalizationMapsCrockfordSubstitutions`: `code_normalization_maps_crockford_substitutions`. `I`/`L` to `1`, `O` to `0`.
  - `TestCodeValidationRejectsWrongLengthAndAlphabet`: `code_validation_rejects_wrong_length_and_alphabet`

### 2. The work factor — 4 rows + 2 inbound ⚑

- [x] The const, the live atomic, the setter, and `fast_scrypt` lowering both.
- [x] Rows:
  - `TestEnrollmentScryptWorkFactorIsDeliberate` ⚑: `enrollment_scrypt_work_factor_is_deliberate`. Asserts the shipped const is 14.
  - `TestEnrollmentAndIdentityWorkFactorsAreIndependent` ⚑: `enrollment_and_identity_work_factors_are_independent`. Both directions.
  - `TestShippedEnrollmentWorkFactorReachesARealAgeFile` ⚑: `shipped_enrollment_work_factor_reaches_a_real_age_file`. Read the factor back out of a fresh seal's own scrypt stanza.
  - `TestHostileWorkFactorCostsABoundedWait` ⚑: `hostile_work_factor_costs_a_bounded_wait`. A hand-built blob claiming 2^30 is refused promptly.
- [x] Inbound (in `crypt.rs`):
  - `only_the_test_hooks_write_the_work_factors`, extended. The enrollment atomic is stored only in its setter.
  - `every_scrypt_identity_caps_its_work_factor`, extended. The open path's identity comes from the one capped constructor.

  Both comments drop their "enrollment's is P7's".

### 3. Seal and open — 10 rows ⚑

- [x] The payload (serde TOML), the sealing entry point that validates the TTL, and `open_enrollment` with the bound and the scope.
- [x] Rows (`enrollment/seal_tests.rs`):
  - `TestSealRoundTripsThroughItsOwnCode`: `seal_round_trips_through_its_own_code`
  - `TestWrongCodeOpensNothing` ⚑: `wrong_code_opens_nothing`. `CodeWrong`.
  - `TestRetypedCodesOpenTheSameRequest`: `retyped_codes_open_the_same_request`
  - `TestMalformedCodeIsRejectedBeforeAnyDecryption` ⚑: `malformed_code_is_rejected_before_any_decryption`. Decrypt count zero; same error as a wrong code.
  - `TestTamperedSealDoesNotOpen` ⚑: `tampered_seal_does_not_open`
  - `TestSealedIDMustMatchTheFilename`: `sealed_id_must_match_the_filename`. A changed epoch alone is not a mismatch.
  - `TestTooManyPendingIsRefusedBeforeAnyDecryption` ⚑: `too_many_pending_is_refused_before_any_decryption`. More than 32 live requests is `TooManyPending`, its message naming the ID form.
  - `TestTheBoundCountsLiveRequestsOnly`: `the_bound_counts_live_requests_only`. 40 files, most expired by filename, with no write run.
  - `TestIDScopedRunPassesTheBoundByConstruction`: `id_scoped_run_passes_the_bound_by_construction`. One decryption attempt per code.
  - `TestScopeIsHonouredNotTreatedAsAHint` ⚑: `scope_is_honoured_not_treated_as_a_hint`

### 4. The payload is untrusted input — 9 rows ⚑

- [x] Field validation before `OpenedRequest` exists:
  - `devicename::valid` on the device
  - `agekey` on the pubkey
  - the one-value `method` allowlist (`passphrase`)
  - UUID on `request_id` and `vault_id`
  - RFC 3339 on both timestamps

  Then the `vault_id` comparison. A malformed file is skipped, not fatal
  to the run.
- [x] Rows (`enrollpayload_test.go` and the rest of `enrollseal_test.go`):
  - `TestMalformedPayloadsAreRefusedAsMalformed` ⚑: `malformed_payloads_are_refused_as_malformed`. Covers:
    - a device outside the allowlist, including ANSI escapes and a newline
    - a bad pubkey
    - an unknown method
    - a non-UUID `request_id`
    - an absent or non-UUID `vault_id`

    Each is `MalformedRequest`, not `CodeWrong`, at `Conflict`.
  - `TestMalformedPayloadDoesNotPoisonTheRun`: `malformed_payload_does_not_poison_the_run`
  - `TestValidateSealedPayloadRejectsNonTOML`: `validate_sealed_payload_rejects_non_toml`
  - `TestSealEnrollmentRejectsInvalidInput`: `seal_enrollment_rejects_invalid_input`
  - `TestRequestSealedForAnotherVaultIsRefused` ⚑: `request_sealed_for_another_vault_is_refused`. Built as a file copy between two vaults. `WrongVault`, which is neither `CodeWrong` nor `MalformedRequest`, and names the vault it is for.
  - `TestOversizedPendingFileIsNeverOpened` ⚑: `oversized_pending_file_is_never_opened`. Skipped from its size alone; decrypt count zero; not deleted.
  - `TestOversizedFileIsSkippedEvenWhenHandedInDirectly`: `oversized_file_is_skipped_even_when_handed_in_directly`
  - `TestDecryptedPayloadIsBoundedToo` ⚑: `decrypted_payload_is_bounded_too`. A multi-megabyte sealed payload is refused on the limit, not allocated.
  - `TestGeneratedCodeIsReturnedAndNowhereElse`: `generated_code_is_returned_and_nowhere_else`. Not in the vault, any written file, or local state. Scoped to the generating side, as Go's is.

### 5. Filenames and expiry — 10 rows

- [x] Filename build and parse, with the UUID and epoch halves validated independently. The TTL ceiling (7 days) and default (24h). The sealed-expiry clamp, compared against now, not `created`. The clock-skew rule (`expires` before `created`).
- [x] Rows:
  - `TestPendingFileNameRoundTrips`: `pending_file_name_round_trips`
  - `TestPendingFileNameParserSkipsStrays`: `pending_file_name_parser_skips_strays`
  - `TestDefaultTTLIsWithinTheCeiling`: `default_ttl_is_within_the_ceiling`
  - `TestSealRejectsTTLsOutsideTheCeiling`: `seal_rejects_ttls_outside_the_ceiling`. Zero, negative and beyond the ceiling, at `Usage`, before anything is generated.
  - `TestExpiryComesFromTheSealNotTheFilename`: `expiry_comes_from_the_seal_not_the_filename`
  - `TestFilenameClaimingExpiryOnALiveSealGrantsNothing`: `filename_claiming_expiry_on_a_live_seal_grants_nothing`
  - `TestForgedFilenameEpochIsPrunedAsExpired`: `forged_filename_epoch_is_pruned_as_expired`
  - `TestSealedExpiryBeyondTheCeilingIsExpired`: `sealed_expiry_beyond_the_ceiling_is_expired`. `Expired`, not `MalformedRequest`, under an in-range filename epoch.
  - `TestExpiryIsComparedAgainstNowNotCreated`: `expiry_is_compared_against_now_not_created`
  - `TestClockSkewIsItsOwnError`: `clock_skew_is_its_own_error`

### 6. Listing, resolution, pruning — 10 rows

- [x] `pending_enrollments`, `resolve_enrollment`, and pruning (crate-private: E4 wires it into every recipient write).
- [x] Rows (`enrollment_test.go`, `enrollprune_test.go`):
  - `TestPendingEnrollmentsReportsZeroWithoutTheDirectory`: `pending_enrollments_reports_zero_without_the_directory`
  - `TestPendingEnrollmentsSkipsStrays`: `pending_enrollments_skips_strays`
  - `TestPendingEnrollmentsFiltersExpiredWithoutDeleting`: `pending_enrollments_filters_expired_without_deleting`
  - `TestPendingEnrollmentsReadsNoGitHistory`: `pending_enrollments_reads_no_git_history`. Instrument the git layer, or use a long history.
  - `TestFilenameEpochMatchesTheSealedExpiry`: `filename_epoch_matches_the_sealed_expiry`
  - `TestResolveEnrollmentFollowsTheBaseResolutionOrder`: `resolve_enrollment_follows_the_base_resolution_order`. Exact beats substring; an ambiguous substring returns *all* matches; nothing matching is `NoSuchRequest`.
  - `TestResolveEnrollmentMatchesTheUUIDPortionOnly`: `resolve_enrollment_matches_the_uuid_portion_only`
  - `TestPruneRemovesExpiredAndForgedExpiriesOnly`: `prune_removes_expired_and_forged_expiries_only`. Unparseable names are left alone.
  - `TestPruneWithoutAPendingDirectoryIsNotAnError`: `prune_without_a_pending_directory_is_not_an_error`
  - `TestPruningLeavesLiveRequestsAlone`: `pruning_leaves_live_requests_alone`

### 7. Inertness and cost — 5 rows ⚑

- [x] Rows (`enrollinert_test.go`, the rest of `enrollseal_test.go`):
  - `TestPendingRequestGrantsNoDecryption` ⚑: `pending_request_grants_no_decryption`. An entry written while a valid request for key K is pending is not decryptable by K.
  - `TestPendingRequestsAreNotRecipients` ⚑: `pending_requests_are_not_recipients`. `recipient list` omits them, and `recipient verify` is in sync with ten pending.
  - `TestPendingDirectoryNeedsNoIdentity`: `pending_directory_needs_no_identity`
  - `TestGarbageInPendingIsNeverFatal`: `garbage_in_pending_is_never_fatal`
  - `TestWrongCodeAgainstAFullDirectoryIsFast`: `wrong_code_against_a_full_directory_is_fast`. At the shipped factor, restored for this one test, with a generous ceiling. This is the plan's only wall-clock assertion; see the [index](index.md#process-notes).

(Rows: 7 + 4 + 10 + 9 + 10 + 10 + 5 = 55, plus 2 inbound.)

### 8. Opus review

- [x] Opus review of the ⚑ rows, by mutation. Apply each mutation, run
      the suite, and record what killed it.

| Mutation | Killed by |
|---|---|
| The seal reads the identity work factor | `shipped_enrollment_work_factor_reaches_a_real_age_file`, `enrollment_and_identity_work_factors_are_independent` |
| The open path doesn't cap the claimed factor | `every_scrypt_identity_caps_its_work_factor`, `claimed_factor_is_capped_at_the_enrollment_cap`. `hostile_work_factor_costs_a_bounded_wait` alone does not kill it: age-rust's own default cap (its calibrated target + 4) still refuses 2^30. |
| The open path caps at the identity file's 22 (Q-ENROLL-SCRYPT-CAP) | `claimed_factor_is_capped_at_the_enrollment_cap` |
| Code validation runs after the first decryption | `malformed_code_is_rejected_before_any_decryption` |
| A malformed code returns a different error from a wrong one | `malformed_code_is_rejected_before_any_decryption` |
| `open_enrollment` reads `pending/` instead of its argument | `id_scoped_run_passes_the_bound_by_construction`, `scope_is_honoured_not_treated_as_a_hint` |
| The 32 bound counts expired files | `the_bound_counts_live_requests_only`, `id_scoped_run_passes_the_bound_by_construction` |
| `vault_id` not compared | `request_sealed_for_another_vault_is_refused`. "Compared after `OpenedRequest` is built" has no observable form here: the one function returns either the value or the refusal. |
| Device validation skipped | `malformed_payloads_are_refused_as_malformed`, `malformed_payload_does_not_poison_the_run` |
| Size bound only on the ciphertext | `decrypted_payload_is_bounded_too` |
| Sealed expiry not clamped to the ceiling | `sealed_expiry_beyond_the_ceiling_is_expired`, `expiry_is_compared_against_now_not_created` |
| Expiry compared against `created` | `expiry_comes_from_the_seal_not_the_filename`, `clock_skew_is_its_own_error` |
| The code from a different RNG (fixed seed) | `successive_codes_differ`, `wrong_code_opens_nothing`, `wrong_code_against_a_full_directory_is_fast`, and three more |
| *Added:* the read-time size bound dropped | `oversized_file_is_skipped_even_when_handed_in_directly` |
| *Added:* the sealed id not compared to the filename | `sealed_id_must_match_the_filename` |
| *Added:* clock skew folded into expiry | `clock_skew_is_its_own_error` |
| *Added:* a refusal returned even though another request opened | **Survived Go's rows.** Killed by the new `a_refusal_does_not_displace_a_request_that_opened` |
| *Added:* the open path follows symlinks and FIFOs | the new `symlinks_in_pending_are_never_followed` (Unix) |
| *Second review:* half the code's random bytes filled (40 bits) | **Survived every row.** Killed by the new `generated_codes_use_every_character_in_every_position` |
| *Second review:* four bits per character (64 bits, alphabet 0-F) | **Survived every row.** Killed by the same new row |
| *Second review:* every character drawn from one random byte | `successive_codes_differ` |
| *Second review:* the 32 bound refuses exactly 32 (`>=`) | `wrong_code_against_a_full_directory_is_fast`, the only row that hands in exactly 32 |
| *Second review:* expired filenames not filtered at listing | `pending_enrollments_filters_expired_without_deleting`, `the_bound_counts_live_requests_only`, and three more |
| *Second review:* pruning drops the ceiling half, or also prunes oversized live files | `prune_removes_expired_and_forged_expiries_only` |
| *Second review:* pubkey or method not validated | `malformed_payloads_are_refused_as_malformed` |
| *Second review:* the TTL ceiling refuses exactly 7 days | `seal_rejects_ttls_outside_the_ceiling` |
| *Second review:* resolution skips the exact stage | `resolve_enrollment_follows_the_base_resolution_order` |

The mutations were applied one at a time by a script, each followed by
`cargo test -p gage --lib -- enrollment:: crypt::`.

The second review's survivors that were not gaps:
- Stripping `GAGE` at any length over 16 is an equivalent mutant: every
  length other than 20 still fails the final length check.
- Five one-unit boundaries survive and are accepted: skew at
  `expires == created`, liveness at `expires == now` and at exactly
  now + 7 days, and a file of exactly 16 KiB. None is a value gage
  writes: a TTL is over zero, timestamps are whole seconds, and seals are
  a few hundred bytes.

- [x] An Opus read of the code and the ⚑ rows against the TDD's
      "reframe", "untrusted input" and `D-ENROLL-SEAL-COST` sections.
      Record what it found here.

  Findings:
  - **The open path followed symlinks.** Fixed. Listing already skipped
    anything but a regular file, because `DirEntry::metadata` doesn't
    follow links. But `read_pending_file` opened whatever path it was
    handed. A committed symlink to a FIFO would block the open forever.
    That hang is open in Go: its listing keeps symlinks and `os.Open`
    follows them. The open path now refuses anything whose own metadata
    isn't a regular file (`symlinks_in_pending_are_never_followed`).
  - **Go's poisoning row can't see one of the two poisoning orders.**
    Fixed with a new row; see the table.
  - **The cap.** Q-ENROLL-SCRYPT-CAP, settled before code.
  - **Timestamps are parsed by `jiff`, which accepts more than RFC 3339:**
    a space for the `T`, and bracketed annotations. Accepted. The
    timestamps are sealed by a code-holder, the result is a validated
    instant, and nothing about the spelling is shown or stored.
  - **Attacker-supplied strings in refusal messages** (`request_id`,
    `method`, the timestamps) are `Debug`-quoted, so control characters
    reach a terminal escaped. The device name is never echoed. A message
    is at most the 16 KiB payload bound.
  - **Nothing in the open path reads git history, takes the lock, or
    writes a file.** Opening deletes nothing
    (`the_bound_counts_live_requests_only`). Pruning is crate-private and
    unwired until E4.

  Second review (a fresh Opus read and its own mutation pass, after the
  commit):
  - **No row saw the code's entropy.** Fixed. The shape, alphabet and
    successive-codes rows all pass a generator that fills half its bytes
    or reads four bits per character, and both mutations survived. This
    is the bug the recommended-model note warns about, invisible to a
    passing test. The new row requires every character in every
    position over 2000 draws. Go has the same gap.
  - **An epoch past i64 is a stray, not expired.** `parse_pending_file_name`'s
    comment said every forged epoch is pruned; one too long to parse is
    skipped, never listed or counted, and never pruned. The behavior
    matches Go and costs nothing, so only the comment changed.
  - **For E4: one request id can sit under two filenames.** A git writer
    can copy a seal to a second epoch, because the id check compares only
    the UUID half (as designed). Then a broad open returns the request
    twice, and `resolve_enrollment` on the full id is `Ambiguous` between
    two candidates the human can't tell apart. Go behaves the same.
    Approval should dedupe by id, and deny-by-id should act on every
    file with that id, rather than leave a request no id can name.
  - **For E4: `agekey::validate_recipient` is shape-only**, so a sealed
    pubkey can be an all-uppercase spelling or a plugin recipient. Both
    are valid in `.age-recipients`, and both are Go's behavior. E4's
    already-a-recipient check should compare keys case-insensitively,
    or the uppercase spelling of an existing key gets added twice.
  - Confirmed, no change: the seal reads only its own factor; the open
    path's one capped constructor at 18; the bound and code validation
    before any KDF run; payload limit enforced through age's reader; every
    field validated before an `OpenedRequest` exists; the vault-id,
    request-id, skew and ceiling ordering as the TDD gives it; `Zeroizing`
    capacity in `generate_code` exact, so the code is never reallocated.

  Rows beyond Go's 55, each named in its file:
  - `claimed_factor_is_capped_at_the_enrollment_cap` (Q-ENROLL-SCRYPT-CAP)
  - `absent_payload_fields_are_malformed_not_unparseable`
  - `a_refusal_does_not_displace_a_request_that_opened`
  - `symlinks_in_pending_are_never_followed`
  - `generated_codes_use_every_character_in_every_position` (second review)

### 9. Parity and ledger

- [x] No command exists yet, and none should. There is no fixture check
      in this milestone.
- [x] Parity ledger: §2 `798ed47` and `e52828a` (now fully `[x]`) and §3
      E2, marked `[x]` after CI. (§3's row covers E1b–E4 together, so it
      stayed `[ ]` until E4 shipped; now `[x]`.)
- [x] Mark E2 `[x]` in the [P7 index](index.md) after CI. (CI green on
      Linux, macOS and Windows at `0d12cea`.)

## Definition of done

Every box above is checked. All 55 claimed rows and the 2 inbound rows are
ported, and the review table is filled in. `make lint` and `make test` are
clean locally and on Linux, macOS and Windows CI.

The milestone is done when:
- a request sealed to a code opens with that code, as typed by a person,
  and with nothing else
- no file in `pending/`, however hostile, can hang the open path, exhaust
  memory, take down the run, or put an unvalidated string in front of a
  human
- `pending/` grants nothing
