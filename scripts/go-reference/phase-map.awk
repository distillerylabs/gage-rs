# Assigns each Go gage test file to the gage-rs port phase that owns it.
# Input: "<file>\t<test>" lines; output: "<phase>\t<file>\t<test>".
# A file matching no rule is tagged UNMAPPED, which capture.sh rejects.
function phase(f,  b) {
  b=f; sub(/.*\//,"",b); sub(/_test\.go$/,"",b)
  if (f ~ /internal\/gage\/(exitcode|atomicfile|vaultlock|xdgpaths|config|devicename|vaultconfig|gittest)\//) return "P1"
  if (f ~ /internal\/gage\/(agekey|memlock)\//) return "P2"
  # gitrepo.Log (the single-path history walk) shipped with Go M12, for
  # `log`/`history`, not with sync (re-tagged in P4 planning).
  if (f ~ /internal\/gage\/gitrepo\/history_test\.go$/) return "P6"
  if (f ~ /internal\/gage\/(gitrepo|remoteauth|syncerr)\//) return "P4"
  if (f ~ /internal\/gage\/recipients\//) return "P5"
  # recipient_batch tests E1b's N-recipient form of AddRecipient, whose
  # only consumer is E4's batch approval (re-tagged in P5 planning).
  if (b ~ /^(enroll|approve)/ || b=="clone_enroll" || b=="recipient_batch") return "P7"
  if (b ~ /^recovery/ || b=="init_recoverykey") return "P8"
  if (b ~ /^(paths|registry|help|usageerror|errorprefix|version|main_integration|vault_lifecycle|vaultid_keying|vault_create|init)$/) return "P1"
  # vault_test.go tests Identity close and page locking (M2), not the
  # vault lifecycle its name suggests.
  if (b ~ /^(vault|crypt|unlock|identityfile_marker|init_identity|prompter|prompter_pty|coredump|testsupport|entry|field|entry_crud|insert_field|insert_multiline_pty)$/) return "P2"
  # The $EDITOR round trip, insert -e, rename and the entry-update path are
  # Go M5's, not M4's (re-tagged in P2 planning).
  if (b ~ /^(entry_edit|insert_edit|editor|scratchdir|scratchdir_linux|scratchdir_other)$/) return "P3"
  if (b ~ /^(resolve|resolve_cli|generate|rename|edit|show|session|shell|repl|session_pty|sessionhelp|index|search|complete|complete_pty)$/) return "P3"
  # cat_test.go holds only #55's cat display tests, which are M12's.
  if (b == "cat") return "P6"
  if (b ~ /^(remotesync|sync_index|syncresolve|sync_cli|syncresolve_cli|clone|auth|init_remote_auth)$/) return "P4"
  if (b ~ /^(recipient|reencrypt|recipient_cli|identity_add|identity_cli|trustcache|trustcache_cli|move)$/) return "P5"
  if (b ~ /^(clip|qr|log|history|script)$/) return "P6"
  return "UNMAPPED"
}
{ print phase($1) "\t" $1 "\t" $2 }
