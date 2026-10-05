# MP-11 F7 / PR #879 — FINAL local handoff

MP-11 implementation head `5a568f3bc64995d9d3d63ba901ff51f4c50f7335`, branch `mp11/credential-access-fixes`, base `74e50b787`, previous published review head `ef8fc7621`. Local protocol **435**, peer **70** unchanged; no peer provider-run shape change or client minimum bump. Local [skip ci] commits only. F7 and both #879 findings are handled at source/fixture scope.

| MP-11 review | Commit | Disposition and regression |
| --- | --- | --- |
| F7 public provider-run contract | ce63f9b98 5f5789058 7eac524a3 5a568f3bc | Explicit allowlist removes credential-bearing launch fields, diagnostic payloads and credentialized endpoints from public responses/events. Authoritative private state retained. Protocol 435; snapshot/hash, ownership/private-projection checks, source CI guard and real prebuilt-kernel drill. Fork fixture converts private state through actual public projection and pins its new hash. |
| #879 round 1 / 1 OAuth exchange loss | d04734543 | Completion uses the current-pending authorization database transaction, preserving exchanged credentials during unrelated event delivery. Revocation/generation fences retained. Store and real blocked signed-webhook publisher fail-first regressions: 0/2 before fix; rebuilt SDK 55/55 afterwards. |
| #879 round 1 / 2 shebang parse failure | d148572b4 | Move hashbang to first line. Actual entry parser fails before fix, all 57 touched entries parse afterwards without launching drills. |
| Separate #868 follow-ups | queued on mp11/fixes | 08:13 signal coverage and 13:30 import/viewer/PID-generation findings remain a separate requested round; not marked handled by this F7/#879 continuation. |
| Separate 14:03 publication caller-isolation finding | queued on mp11/publication-run-isolation | New branch from 74e50b787 explicitly requested after current items. No verification/fix/dispute claimed here. |

MP-11 validation: at final implementation source, **175 protocol snapshots + 4 client-conformance + 4 F7 regressions**, zero failed; focused protocol drill **3/3**. Rebuilt SDK **55/55** at `d04734543`, unchanged SDK source thereafter. Full `pnpm test` exits **0**: **5,469 Node executions passed / 25 skipped / zero failed**, plus **49 Bun passed**; Node entry started at `d04734543`, and only the Rust fork test fixture changed during it. Counts include repeated CI aliases. Full identities, commands, exit codes and scope in FINAL `/root/.codex/evidence/browser-resume-20260930/mp11fix-ra/REPORT.md`.

MP-11 invalid cached SDK reuse is recorded separately; its result is not evidence for the fixed source. Earlier metadata/assertion/fixture/Bun failures are preserved with corrected focused checks and successful full entry. Observed resource minima: memory **10.00 GiB**, disk **107.55 GiB**, no observed floor breach. Cleanup removed **13** owned generated roots; recorded own live validation roots **0**. No shared/protected resources or provider profiles touched.

MP-11 new implementation commits after previous review head: `ce63f9b98`, `5f5789058`, `7eac524a3`, `d148572b4`, `d04734543`, `5a568f3bc`. The following documentation commit and full IDs are in external `F7-R879-LOCAL_COMMITS.txt`. Earlier mappings/evidence below retain their original identities and are historical, not substituted for current checks. The full ordinary/managed behavioral matrix belongs to its acceptance lanes.

---

# MP-11 credential/access fixes — original historical handoff

MP-11 base `74e50b787a5919ee5c3d580c5b088989fd4a1adf`; branch `mp11/credential-access-fixes`; implementation head `a95610ff1132faa8b28b9543e3243337305e8e49`. Local commits only; no publication. **35 families fixed at source/fixture scope, F7 PARTIAL pending protocol allocation, F8 OWNED-BY-mp11fix3.** Passing Rust: focused 84, secret 35, managed-context 193, config 96, passkey 27, usage 7, attestation 2, slice/log 126 + 1 ignore, ordinary restart 1, rollback 2, AEgs 53. Rebuilt baseline 46 pass / 35 expected failures (exit 101); separate F37 generation/signed HTTP baseline each fails. Full `pnpm test` passes on explicit composition `46c5c611a`: 5,441 Node executions passed / 25 skipped / zero failed, plus 49 Bun passes. Signal lint 36 reviewed / zero violations. Cleanup complete, recorded own live processes zero. FINAL evidence: `/root/.codex/evidence/browser-resume-20260930/mp11fix-ra/REPORT.md`.

MP-11 dependency: separate Vault/signals PR #868 (`c2e95fc70`, `83f60044e`) is intentionally not replayed onto this requested base. Full Node CI validation composes its published Node/Python signal guards with this lane. On replay, preserve both helper imports in `room-kernel-diagnostics.mjs` and `live-room-environment-pointer-click-drill.mjs`; neither implementation should be dropped.

| MP-11 RA family | Local commit(s) | Review disposition |
| --- | --- | --- |
| MP-11 F1 | ef440c457 | Malformed passkey pin fails closed; unlock pin errors clear Vault/cache. |
| MP-11 F2 | ef440c457 | Descriptor-pinned exclusive private metadata staging and atomic replacement. |
| MP-11 F3 | ef440c457 | Shared HTTP secret scrubbing across success/error, derived encodings and bounded reads. |
| MP-11 F4 | ef440c457 ec60799ea | Target-key snapshot independently of handle; explicit rollback/cache failure handling. |
| MP-11 F5 | 17f6fc099 ef440c457 b286333b7 | Nonblocking regular-file admission for all named credential readers/bootstrap seams. |
| MP-11 F6 | ef440c457 | Always policy refuses existing-lease command fast path for all three command callers. |
| MP-11 F7 | 8d3c2be3d ef440c457 ce63f9b98 5f5789058 7eac524a3 5a568f3bc | Public response/event DTO allowlist; private execution projection retained; allocated local protocol 435, peer 70 unchanged; snapshot/hash and focused drill. |
| MP-11 F8 | OWNED-BY-mp11fix3 | Skip this lane: authenticated peer/forwarded-home credential binding seam. |
| MP-11 F9 | b286333b7 | Publication identity/content ownership; durable quarantine retirement/restart; preserve later edits. |
| MP-11 F10 | b286333b7 | Independent kernel/Git/provider rollback attempts, aggregate failures and retain recovery state. |
| MP-11 F11 | b286333b7 530598e30 022c9ba02 | Snapshot/restore only owned GitHub/gist helper keys; preserve unrelated and later settings. |
| MP-11 F12 | ef440c457 | Private descriptor spools with byte limits, Unix hard child file cap, fixed diagnostics. |
| MP-11 F13 | ef440c457 | Existing invalid config fails closed; private atomic publication with interrupted-write regression. |
| MP-11 F14 | ab927fc39 | Journal access policy before mutation; activate near commit; restore shell/sudoers on failure. |
| MP-11 F15 | d21d83c12 | Synthetic prototype excludes reserved env fallback; scrub raw/encoded/derived HTTP echoes. |
| MP-11 F16 | b286333b7 | Root-relative no-follow overlay component opens; ancestor swap rejected. |
| MP-11 F17 | b286333b7 | Cleanup failure retains materialization ownership and import journal for restart. |
| MP-11 F18 | 6da9cf260 c43dbe5ef | Private no-follow slice auth publication; exact backup ownership before import/retirement. |
| MP-11 F19 | d21d83c12 | Early temporary-keychain transaction cleanup restores prior search/default settings. |
| MP-11 F20 | 37905dbe3 | Remove execution of supplied installer for feature probing; static marker + signature/team gate. |
| MP-11 F21 | ec32d6575 | Private shared writer for cloud relay bootstrap state; preserves alias target and enforces 0600. |
| MP-11 F22 | ec32d6575 d3442dd69 | Bound file tails and concurrent command pipe drains; preserve UTF-8 boundaries. |
| MP-11 F23 | b286333b7 3c1f5e1e5 | Pin attestation root/components and hash admitted regular descriptor; deterministic swaps rejected. |
| MP-11 F24 | ec32d6575 | Authority mutex fences credential spawn and binding finish against retirement/current-agent changes. |
| MP-11 F25 | 8d3c2be3d 8202c1bef | Explicit scoped provider profiles; standard worker requires its own profile; no ambient harness roots. |
| MP-11 F26 | 464dd7097 | Remote credential environment via private SSH stdin launcher, fixed diagnostics, bounded env scope. |
| MP-11 F27 | 30f7273a2 | Recursive public diagnostic validation covers free-form strings and keys. |
| MP-11 F28 | 8d3c2be3d | Exact native transcript IDs and bounded marker-only evidence; no whole-profile transcript export. |
| MP-11 F29 | 8d3c2be3d | Owned slice cleanup immediately after creation, on all subsequent setup failures. |
| MP-11 F30 | 8d3c2be3d | Exclusive private runtime roots; remove copied state even in evidence keep/debug modes. |
| MP-11 F31 | 30f7273a2 | Environment evidence is allowlisted presence/length/config shape; no raw native error tails. |
| MP-11 F32 | 7408beed7 | Stable sanitized provider errors and bounded credential HTTP response parsing. |
| MP-11 F33 | 7408beed7 | Revoke atomically invalidates pending authorization; complete only current pending state. |
| MP-11 F34 | 7408beed7 | No ordinary signed-owner adoption of legacy credentials; fresh authorization required. |
| MP-11 F35 | 7408beed7 | Trusted HTTPS/loopback endpoint validator; no credentials in redirect, userinfo or query. |
| MP-11 F36 | 7408beed7 | Frame-poll ingress byte cap, early declared-size reject, timeout and task budget. |
| MP-11 F37 | 7408beed7 d3442dd69 f43fb2bd4 3c1f5e1e5 | Owner/provider/generation-pinned reconcile and delivery fence; old selections cannot revive. |

MP-11 fixture correction `310eaee5a`: guard fast-exit stdin errors, provide the XFS capability fixture its required unprivileged context on root builders, and allow 45 seconds for the real owned-process inspection fixture. No product security check is weakened. `c43dbe5ef` also isolates publication fixture homes/current UID.

MP-11 original handoff inbox statement is superseded by the F7/#879 continuation below; allocated protocol 435 is now implemented. Non-Unix descriptor traversal fails closed (F16/F23); legacy ownership journals/helpers with no proof require operator recovery. These changes establish source/fixture seams, not full MP-11 ordinary/managed behavioral acceptance.
