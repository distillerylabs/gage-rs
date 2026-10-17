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
[Q-CROCKFORD](../open-questions.md#q-crockford) settle the shape. What is
left to pin down before code:

- **Module layout.** A new `crates/gage/src/enrollment/` module: `code`,
  `seal`, `name` (filename build and parse), and `mod.rs` (the `Vault`
  methods, the types, `EnrollmentError`). Tests in `enrollment/*_tests.rs`,
  following the crate's `*_tests.rs` convention.
- **Types and signatures**, Go's names in Rust form:
  - `PendingRequest`: id, expiry and path, from the filename.
  - `OpenedRequest`: device, pubkey, method, created, expires and the
    `PendingRequest`. It has no vault-id field: every one that exists is
    for this vault.
  - `EnrollmentRequest`: E3's return value; only the sealing half here.
  - `fn pending_enrollments(&self) -> Result<Vec<PendingRequest>, EnrollmentError>`
  - `fn open_enrollment(&self, requests: &[PendingRequest], codes: &[Zeroizing<String>]) -> Result<Vec<OpenedRequest>, EnrollmentError>`
  - `fn resolve_enrollment(&self, id: &str) -> Result<PendingRequest, EnrollmentError>`

  None of the three takes an `Identity`. That is the contract E4's late
  unlock rests on. Decide whether codes travel as `Zeroizing<String>`
  (recommended, as passphrases do) and whether the sealing entry point is
  `pub(crate)` (E3 is its only caller).
- **`EnrollmentError`**, one `thiserror` enum whose variants follow the
  TDD's table: `CodeWrong` (`LockedOrAuth`, also returned for a malformed
  code), `Expired`, `ClockSkew`, `IdMismatch`, `MalformedRequest`,
  `WrongVault { vault }`, `TooManyPending` (all `Conflict`), `NoSuchRequest`
  (`NotFound`), `Ambiguous(RequestCandidates)` (`Ambiguous`,
  [Q-ENROLL-AMBIGUOUS](../open-questions.md#q-enroll-ambiguous)) and an
  invalid TTL (`Usage`). E3 and E4 add their own variants. `WrongVault`
  carries the local name of the vault the request is for, looked up from
  global config, falling back to the id. Reading global config from the
  library is precedented (`unlock` does it).
- **Scrypt construction under P2's guards.**
  - Sealing goes through `crypt::Recipient::passphrase_at(code,
    enrollment_work_factor())`.
  - Opening goes through the one scrypt identity constructor, which sets
    `max_work_factor(SCRYPT_MAX_WORK_FACTOR)`.
  - `ENROLLMENT_SCRYPT_WORK_FACTOR: u8 = 14` is its own const, with
    D-ENROLL-SEAL-COST's reasoning in its comment and an instruction not
    to merge it with `SHIPPED_SCRYPT_WORK_FACTOR`. It gets a live atomic
    and `set_enrollment_work_factor_for_tests` (`cfg(test)` /
    `test-support`).
  - `testsupport::fast_scrypt()` lowers both factors.
  - Check whether 22 is the right cap for a seal written at 14. Go uses
    the identity file's cap for both.
- **Decrypt-count instrumentation.** The "rejected before any decryption"
  and "never opened" rows need a count of decryption attempts. Use a
  test-only hook in the `reencrypt_hook` style (`vault.rs`), not a new
  public seam.
- **The payload size bound.** `MAX_PENDING_REQUEST_BYTES = 16 * 1024`.
  It is checked from the directory entry's metadata before any read, and
  again on the decrypted payload by reading at most limit+1 bytes from
  age's reader.
- **The clock.** The expiry, ceiling and skew rows need "now". Decide
  whether the library reads `jiff::Timestamp::now()` and tests build
  payloads relative to it (Go's approach), or takes an injected clock.
  Prefer Go's approach unless a row can't be written honestly without
  one.

## Tasks

Write each task's tests first, at task granularity.

### 1. The code — 7 rows

- [ ] Generation, normalization and validation (`enrollment/code.rs`).
- [ ] Rows:
  - `TestGeneratedCodeHasTheDocumentedShape`: `generated_code_has_the_documented_shape`
  - `TestGeneratedCodesAvoidTheAmbiguousLetters`: `generated_codes_avoid_the_ambiguous_letters`. Over many generations: never `I`, `L`, `O` or `U`.
  - `TestGeneratedCodesNormalizeToThemselves`: `generated_codes_normalize_to_themselves`
  - `TestSuccessiveCodesDiffer`: `successive_codes_differ`
  - `TestCodeNormalizationAcceptsHowPeopleRetypeCodes`: `code_normalization_accepts_how_people_retype_codes`. Lowercase, spaces for hyphens, a missing `GAGE-`.
  - `TestCodeNormalizationMapsCrockfordSubstitutions`: `code_normalization_maps_crockford_substitutions`. `I`/`L` to `1`, `O` to `0`.
  - `TestCodeValidationRejectsWrongLengthAndAlphabet`: `code_validation_rejects_wrong_length_and_alphabet`

### 2. The work factor — 4 rows + 2 inbound ⚑

- [ ] The const, the live atomic, the setter, and `fast_scrypt` lowering both.
- [ ] Rows:
  - `TestEnrollmentScryptWorkFactorIsDeliberate` ⚑: `enrollment_scrypt_work_factor_is_deliberate`. Asserts the shipped const is 14.
  - `TestEnrollmentAndIdentityWorkFactorsAreIndependent` ⚑: `enrollment_and_identity_work_factors_are_independent`. Both directions.
  - `TestShippedEnrollmentWorkFactorReachesARealAgeFile` ⚑: `shipped_enrollment_work_factor_reaches_a_real_age_file`. Read the factor back out of a fresh seal's own scrypt stanza.
  - `TestHostileWorkFactorCostsABoundedWait` ⚑: `hostile_work_factor_costs_a_bounded_wait`. A hand-built blob claiming 2^30 is refused promptly.
- [ ] Inbound (in `crypt.rs`):
  - `only_the_test_hooks_write_the_work_factors`, extended. The enrollment atomic is stored only in its setter.
  - `every_scrypt_identity_caps_its_work_factor`, extended. The open path's identity comes from the one capped constructor.

  Both comments drop their "enrollment's is P7's".

### 3. Seal and open — 10 rows ⚑

- [ ] The payload (serde TOML), the sealing entry point that validates the TTL, and `open_enrollment` with the bound and the scope.
- [ ] Rows (`enrollment/seal_tests.rs`):
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

- [ ] Field validation before `OpenedRequest` exists:
  - `devicename::valid` on the device
  - `agekey` on the pubkey
  - the one-value `method` allowlist (`passphrase`)
  - UUID on `request_id` and `vault_id`
  - RFC 3339 on both timestamps

  Then the `vault_id` comparison. A malformed file is skipped, not fatal
  to the run.
- [ ] Rows (`enrollpayload_test.go` and the rest of `enrollseal_test.go`):
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

- [ ] Filename build and parse, with the UUID and epoch halves validated independently. The TTL ceiling (7 days) and default (24h). The sealed-expiry clamp, compared against now, not `created`. The clock-skew rule (`expires` before `created`).
- [ ] Rows:
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

- [ ] `pending_enrollments`, `resolve_enrollment`, and pruning (crate-private: E4 wires it into every recipient write).
- [ ] Rows (`enrollment_test.go`, `enrollprune_test.go`):
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

- [ ] Rows (`enrollinert_test.go`, the rest of `enrollseal_test.go`):
  - `TestPendingRequestGrantsNoDecryption` ⚑: `pending_request_grants_no_decryption`. An entry written while a valid request for key K is pending is not decryptable by K.
  - `TestPendingRequestsAreNotRecipients` ⚑: `pending_requests_are_not_recipients`. `recipient list` omits them, and `recipient verify` is in sync with ten pending.
  - `TestPendingDirectoryNeedsNoIdentity`: `pending_directory_needs_no_identity`
  - `TestGarbageInPendingIsNeverFatal`: `garbage_in_pending_is_never_fatal`
  - `TestWrongCodeAgainstAFullDirectoryIsFast`: `wrong_code_against_a_full_directory_is_fast`. At the shipped factor, restored for this one test, with a generous ceiling. This is the plan's only wall-clock assertion; see the [index](index.md#process-notes).

(Rows: 7 + 4 + 10 + 9 + 10 + 10 + 5 = 55, plus 2 inbound.)

### 8. Opus review

- [ ] Opus review of the ⚑ rows, by mutation. Apply each mutation, run
      the suite, and record what killed it.

| Mutation | Killed by |
|---|---|
| The seal reads the identity work factor | |
| The open path doesn't cap the claimed factor | |
| Code validation runs after the first decryption | |
| A malformed code returns a different error from a wrong one | |
| `open_enrollment` reads `pending/` instead of its argument | |
| The 32 bound counts expired files | |
| `vault_id` not compared, or compared after `OpenedRequest` is built | |
| Device validation skipped | |
| Size bound only on the ciphertext | |
| Sealed expiry not clamped to the ceiling | |
| Expiry compared against `created` | |
| The code from a different RNG (fixed seed) | |

- [ ] An Opus read of the code and the ⚑ rows against the TDD's
      "reframe", "untrusted input" and `D-ENROLL-SEAL-COST` sections.
      Record what it found here.

### 9. Parity and ledger

- [ ] No command exists yet, and none should. There is no fixture check
      in this milestone.
- [ ] Parity ledger: §2 `798ed47` and `e52828a` (now fully `[x]`) and §3
      E2, marked `[x]` after CI.
- [ ] Mark E2 `[x]` in the [P7 index](index.md) after CI.

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
