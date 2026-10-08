# Managed login copies: cold Claude dispatch and lost acknowledgements

2026-10-08. Branch `cred/managed-login-copies`; reviewed PR #913 head `65a4c8c35`. New commits are local only; no push or GitHub CI.

| Review finding | Fix commit | Fail-first commit | Result |
| --- | --- | --- | --- |
| P2 home-side cold Claude launch | `5eb73ab98` | `07b0be92d` | Ordinary/workflow home dispatch uses the confirmed receiving official-login profile before requiring a Vault setup token. Typed worker credential requests retain the setup-token fallback, including cold native TUI launches. |
| P2 lost first copy acknowledgement | `1b5fc6f31` | `b20e15a41` | A private credential-free journal records issued generations before transfer, bound to source account and receiving machine/kernel. A preserved G1 receipt can reconcile a G2 retry; confirmed receipts retire pending attempts while retaining concurrent requests. Removed copies cannot satisfy installation reuse. |

Both regressions failed at the reviewed behavior before their fixes: the exact home Vault-token error, and the exact home receipt-generation rejection after the production receiver installed G1 and its response was dropped. The final lost-ack regression uses production receiving installation, preserved-login reuse and leased profile update. The home Claude regression exercises ordinary/workflow dispatch through the transport boundary with a synthetic official login and no setup-token registry entry; it does not certify a live Claude provider run.

## Validation for this round

Four explicit new regressions and 20 focused suites pass: 438 passing test executions across overlapping filters, plus one real-login safe drill. Coverage includes the previous security and remap/cache suites, production lost-ack retry, generation scope/restart/concurrency, cold Claude home dispatch, native setup-token fallback, launch/Vault state, protocol snapshots and synthetic queued-work recovery.

Kernel test build, kernel clippy `--all-targets`, workspace fmt and diff checks pass. Clippy reports existing warnings (445 lib-test warnings, including 324 duplicates).

**Real linked Codex safe cells: PASS.** The rebuilt kernel test executable drove production account services and encrypted peer dispatch through a real websocket relay, using an existing official linked Codex login. Home deliberately left G1's successful receipt unconfirmed, issued G2, then reconciled the production receiver's preserved G1 receipt. The official receiving CLI confirmed login before kernel-managed local profile deletion. Home observed the removal and stopped treating the copy as installed. Exact initial import replay and preserved-login import replay after receiver restart were rejected without restoring credentials. This live cell leaves the first receipt unconfirmed; the regression separately exercises a timed-out acknowledgement. The receiver is a pinned home-managed kernel on this host, not Docker.

No official logout, revocation or re-login ran. Read-only `claude auth status` reported no logged-in Claude account. Live Claude cold-launch acceptance remains **BLOCKED(owner)**. Official logout/new-login and subsequent real queued-work resumption remain **BLOCKED(Miguel: disposable login)**; shared logins and all their receiving copies are excluded from those steps. Synthetic queued-work recovery tests do not fulfill that live acceptance.

Credential-free evidence is outside Git at `/Users/miguel/.codex/evidence/credcopies-review-ack/`: valid fail-first logs, green regressions, focused suite/build/clippy/fmt logs, validation manifest, read-only Claude availability and `live-codex-local-removal-replay.json`.

Local protocol stays 455 and relay peer 74. No serialized CLI/app/relay contract changed; the pending-generation journal is private local metadata. Cloud G9 copy inventory, warning and receiving-machine login action remain the owner follow-up.

Cleanup completed: the receiving copy was removed locally through the kernel, owned test/relay processes ended, and this round's dedicated compiler output was removed after a protected-path inventory. Runtime identities, source linked profiles, generation/replay journals and shared reviewer infrastructure remain protected.

The previous rounds remain recorded below for continuity.

# Previous P2 round (reviewed d05118c56)

2026-10-08. Branch `cred/managed-login-copies`; reviewed published head PR #913 `d05118c56`. New commits are local only; no push or GitHub CI.

| Review finding | Fix commit | Regression and resulting behavior |
| --- | --- | --- |
| P2 receiving account ID when reusing a copy | `ae3c2cb3c` | A managed-context default remap launches a leased agent with its registered receiving ID. Existing-copy profile updates send that ID and validate the worker acknowledgement against it; the home agent retains its selected source ID. |
| P2 validate receipt before caching installation | `4d75b0e3b` | Placement or generation rejection leaves Error/uninstalled status. A retry must obtain and validate another receipt before updating the worker profile; a valid retry succeeds. |

`c9074f8bc` adds four fail-first regressions. All four failed against `d05118c56`: an unregistered source ID at leased-agent creation, the source ID sent during a profile update, and installation-cache bypasses after placement and generation rejection. All four pass with these fixes. Tests exercise production receiving-registry import, lease creation, profile environment resolution and public profile-update requests/acknowledgements. They use synthetic credentials, with no provider logout or login enrollment.

## Validation for this round

The four explicit regressions and all 15 focused suites pass: 393 passing test executions across overlapping filters, plus the real-login drill. Coverage includes remote binding (15), profile/configuration state (57), and the previous security round's 13 suites (317): copy notices, managed-context imports, Claude portability and transcript guards, lease accounts, package transfer, publication, peer transport, protocol hashes and synthetic queued-work recovery.

Kernel test build passes. Kernel clippy --all-targets passes with existing warnings. Workspace fmt and diff checks pass. Local protocol remains 455 and relay peer 74: no serialized shape changed.

**Real linked Codex import/local-removal/replay drill: PASS.** The rebuilt kernel test executable exercised the production account services and encrypted peer dispatcher through a real websocket relay. The receiving official CLI confirmed the copied login. Kernel-managed local receiving-profile deletion removed the copy, and both initial-frame replay and preserved-login-frame replay after receiving-kernel restart were rejected without restoring credentials. The receiving environment was a pinned, home-managed kernel on this host, not a Docker container. No official logout, revocation or re-login ran.

Credential-free evidence is outside Git at `/Users/miguel/.codex/evidence/credcopies-review-p2/`: fail-first and green regression logs, focused suite logs, build/clippy/fmt logs, `validation-manifest.json`, and `live-codex-local-removal-replay.json`.

Cleanup is completed before handoff: receiving copy removed through the kernel, owned test/relay processes ended, and this round's compiler output removed. Retained runtime identities, source linked profiles, replay journals, key stores and shared reviewer state are protected.

## Remaining acceptance

`BLOCKED(Miguel: disposable login)`: official logout/revocation, new official login and subsequent queued-work resumption acceptance. Never run those steps on the shared host/worker/builder logins or their receiving copies. Claude live acceptance remains `BLOCKED(owner)` without a linked account. Cloud G9 Provider Accounts copy inventory, warning and receiving-machine official-login action remain a follow-up.

The earlier MP-11 findings and evidence below remain recorded for continuity.

## Earlier MP-11 security round

Branch: `cred/managed-login-copies`. Published baseline: PR #913 `791c01ebde`. Local-only commits; no push or GitHub CI.

| Finding | Fix commit | Behavior |
| --- | --- | --- |
| F2 Medium | `c55029c44` | Persist private, credential-free receiving import generations before installation, including preserved-login imports; reject replay after removal/restart. |
| F3 Low | `df67d9126` | Bind copy receipts and observations to the home-issued account/generation and authenticated receiving placement; observations only update auth state. |
| F4 Low | `c4c5a5ee5` | Bind copy provenance to authenticated lease/package source kernel before installation. |
| F5 Low | `f4d5af849` | Remove a committed credential package before receipt bookkeeping, including bookkeeping failure. |
| F6 Low | `155c3663f`, `240c78da7` | Validate canonical Claude aliases at the shared installer and before existing-profile lease returns; retain the empty-artifact Vault path. |
| F7 Low | `e3ee62dc6` | Model/tool transcript text cannot supply authentication authority; official structured API-error evidence remains supported. |

`6447e7e32` records seven fail-first regressions covering F2–F7. `240c78da7` closes the F6 existing-profile return. The final validation commit adds the safe real-path drill, formats touched Rust files and records this handoff.

### F1 evidence-backed disposition

The read-only `SECREV_E_913.md` explicitly withdraws prior F1 as a security finding in “MP-11 owner documentation question — prior F1 withdrawn as a security finding.” Its frozen owner-policy and AGENTS.md analysis authorizes managed Claude official-login copies. Reinstating the old blanket refusal would contradict that policy. `docs/ARCHITECTURE.md` Docker-lab wording remains an owner documentation follow-up; the separate Vault setup-token path remains supported.

### Earlier validation

All 13 focused suites pass (317 passing test executions across overlapping filters). Coverage: F2–F7 security regressions (8), copy notices (8), managed-context installer (7), two Claude portability checks (1 each), lease accounts (9), native Claude transcript (41), package receiver (16), outbound context service (21), protocol shapes/hashes (184), publication accounts (7), peer transport (10), and synthetic OAuth/queued-work recovery (4). The ignored real-login test is run explicitly below.

`cargo clippy -p chariox-kernel --all-targets -j2` passes with warnings; `cargo fmt --all -- --check` and `git diff --check` pass. Production kernel and final test binary build successfully. The prior build also reproduced the F6 existing-profile return failure before the fix.

**Real Codex Medium drill: PASS.** The existing official linked login was exported/imported only by production Chariox services into a receiving kernel with real persisted machine/kernel identities and pinned managed-slice bootstrap. A real websocket relay forwarded encrypted peer requests to the production kernel dispatcher. The official receiving Codex CLI reported a login. Kernel-managed local profile deletion removed the receiving copy; replay of the exact initial frame was rejected without restoring credentials. A captured later import that had preserved the receiving login was also rejected after restarting the receiving kernel with its retained identity/registry. The receiving profile and credential remain absent. No official logout, revocation or re-login was invoked.

The production built kernel was additionally exercised over its real local websocket API to remove an earlier incomplete drill copy before the fresh transfer. The receiving runtime was a home-managed kernel on this host with pinned slice bootstrap, **not a Docker container**; the import/removal/replay proof does not certify container isolation or TUI logout/new-login acceptance.

Credential-free evidence is retained outside Git under `/Users/miguel/.codex/evidence/credcopies-security/`: `red-tests.log`, `f6-existing-profile-red.log`, `validation-manifest.json`, `live-kernel-preclean.json`, `live-codex-local-removal-replay.json`, and final suite/build/check logs. The opt-in test is `transport::relay_client::tests::managed_copy_security::secrev_f2_live_linked_codex_replay_after_local_removal`; run it alone with its exact name, `--exact --ignored --test-threads=1`, and explicit `CREDCOPIES_LIVE_REPLAY_ROOT` / `CREDCOPIES_LIVE_REPLAY_EVIDENCE` paths. It never invokes provider logout or login enrollment.

Cleanup: the receiving managed credential was removed through the kernel; owned test/kernel/relay processes ended. Stable runtime identities, source linked profiles and replay journals remain protected. The dedicated compiler tree was removed after retaining the validated kernel for the disposable-login follow-up.

F4 validates the authenticated source kernel and requires a nonempty source machine ID. Existing lease/package binding contracts do not supply an independent authoritative source machine ID; the direct slice path also checks the pinned owner-machine bootstrap. No claim of independent machine attestation is made.

Local protocol remains 455; relay peer remains 74. These changes introduce no serialized CLI/app/relay shapes. The generation journal is private local scalar metadata.

### Remaining blocked acceptance

`BLOCKED(Miguel: disposable login)`: official logout/revocation, new official login and subsequent queued-work resumption acceptance. Never perform these on the shared host/worker/builder logins or any receiving copy of those logins. The security Medium drill uses only import, kernel-managed local receiving-profile deletion and replay. Claude live acceptance remains `BLOCKED(owner)` without a linked Claude account.

Cloud Provider Accounts copy inventory, warning and receiving-machine official-login action remain the original G9 follow-up; shared kernel contracts already supply the metadata.
