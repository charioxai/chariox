# Managed login copies: MP-11 security follow-up

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

## F1 evidence-backed disposition

The read-only `SECREV_E_913.md` explicitly withdraws prior F1 as a security finding in “MP-11 owner documentation question — prior F1 withdrawn as a security finding.” Its frozen owner-policy and AGENTS.md analysis authorizes managed Claude official-login copies. Reinstating the old blanket refusal would contradict that policy. `docs/ARCHITECTURE.md` Docker-lab wording remains an owner documentation follow-up; the separate Vault setup-token path remains supported.

## Validation

All 13 focused suites pass (317 passing test executions across overlapping filters). Coverage: F2–F7 security regressions (8), copy notices (8), managed-context installer (7), two Claude portability checks (1 each), lease accounts (9), native Claude transcript (41), package receiver (16), outbound context service (21), protocol shapes/hashes (184), publication accounts (7), peer transport (10), and synthetic OAuth/queued-work recovery (4). The ignored real-login test is run explicitly below.

`cargo clippy -p chariox-kernel --all-targets -j2` passes with warnings; `cargo fmt --all -- --check` and `git diff --check` pass. Production kernel and final test binary build successfully. The prior build also reproduced the F6 existing-profile return failure before the fix.

**Real Codex Medium drill: PASS.** The existing official linked login was exported/imported only by production Chariox services into a receiving kernel with real persisted machine/kernel identities and pinned managed-slice bootstrap. A real websocket relay forwarded encrypted peer requests to the production kernel dispatcher. The official receiving Codex CLI reported a login. Kernel-managed local profile deletion removed the receiving copy; replay of the exact initial frame was rejected without restoring credentials. A captured later import that had preserved the receiving login was also rejected after restarting the receiving kernel with its retained identity/registry. The receiving profile and credential remain absent. No official logout, revocation or re-login was invoked.

The production built kernel was additionally exercised over its real local websocket API to remove an earlier incomplete drill copy before the fresh transfer. The receiving runtime was a home-managed kernel on this host with pinned slice bootstrap, **not a Docker container**; the import/removal/replay proof does not certify container isolation or TUI logout/new-login acceptance.

Credential-free evidence is retained outside Git under `/Users/miguel/.codex/evidence/credcopies-security/`: `red-tests.log`, `f6-existing-profile-red.log`, `validation-manifest.json`, `live-kernel-preclean.json`, `live-codex-local-removal-replay.json`, and final suite/build/check logs. The opt-in test is `transport::relay_client::tests::managed_copy_security::secrev_f2_live_linked_codex_replay_after_local_removal`; run it alone with its exact name, `--exact --ignored --test-threads=1`, and explicit `CREDCOPIES_LIVE_REPLAY_ROOT` / `CREDCOPIES_LIVE_REPLAY_EVIDENCE` paths. It never invokes provider logout or login enrollment.

Cleanup: the receiving managed credential was removed through the kernel; owned test/kernel/relay processes ended. Stable runtime identities, source linked profiles and replay journals remain protected. The dedicated compiler tree was removed after retaining the validated kernel for the disposable-login follow-up.

F4 validates the authenticated source kernel and requires a nonempty source machine ID. Existing lease/package binding contracts do not supply an independent authoritative source machine ID; the direct slice path also checks the pinned owner-machine bootstrap. No claim of independent machine attestation is made.

Local protocol remains 455; relay peer remains 74. These changes introduce no serialized CLI/app/relay shapes. The generation journal is private local scalar metadata.

## Remaining blocked acceptance

`BLOCKED(Miguel: disposable login)`: official logout/revocation, new official login and subsequent queued-work resumption acceptance. Never perform these on the shared host/worker/builder logins or any receiving copy of those logins. The security Medium drill uses only import, kernel-managed local receiving-profile deletion and replay. Claude live acceptance remains `BLOCKED(owner)` without a linked Claude account.

Cloud Provider Accounts copy inventory, warning and receiving-machine official-login action remain the original G9 follow-up; shared kernel contracts already supply the metadata.
