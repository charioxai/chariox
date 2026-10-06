# MP-08 / MP-10 / MP-11 — kaext

## 2026-10-06 20:30 UTC — MP-08 / MP-10 / MP-11 handoff: BLOCKED on real acceptance
- Implementation committed locally: `5c05f8f0f95c192c9a2bc971d4834654fe1cc537`, `0fc289663bf008fe81e4edbd19f36285b42e9dca`; smoke/docs follow-up `dcb35fd5337ee406abfa3520e77e8e75b7a41f2f` (runtime binaries unchanged). No push, PR, deployment or GitHub CI.
- Base `e325afa580d81954e2c179757fc53fa02ed2a2b3`; branch `ka/external-agent-kernel-scope`; allocated local451, peer73 unchanged.
- Final supplementary checks: access 78 pass / 4 ignored; sudo 33 pass / 1 ignored; protocol snapshots 182 pass. Filters overlap. Each exits0. CLI type checking/build and compiled TUI/kernel/launcher pass; Node 25 pass, popup 8 pass. These do not close real MP-10 acceptance or replace MP-11 semantic review.
- Real compiled TUI/CLI + real built kernel, external Python holder, no sessions: RED on base435 (`missing field session_id`); candidate451 shows whole-local-kernel popup and returns the user's refusal. Refreshed after final TUI change. Screenshots/ANSI/logs retained. No official provider or typed-passkey acceptance claimed.
- First intermediate state suite (43 pass) and failed initial cargo-check diagnostics are explicitly historical, not final-source results. Base compiled entry also rejects `access` at startup; fixed both launchers.
- Supplementary smoke helper exits0: global grant popup approval, cross-session access, sibling/descendant/critical boundaries, both sudo refusals, revoke, allowlisted receipt projection and graceful cleanup. Fixture-backed; not official-provider/user-driven acceptance. Node syntax check exits0.
- REVIEW_INBOX absent after all three source milestones; mapping in PUSH_READY.md.
- Evidence: `/root/.codex/evidence/browser-resume-20260930/kaext/`; command/exit logs and source-bound binary hashes included.

## MP-08 / MP-10 / MP-11 — implementation
- Deleted access request/grant session fields, CLI `--session`, session filtering/pinning and grant revocation on session closure. Owner identity derives from the local kernel's configured owner.
- Access/extension use the shared pending interaction board, verifier, pump and expiry with `kernel-access` routing; no session is fabricated. Protocol451 adds `KernelAccessDecisionResponded`; CLI gates only this new access behavior at451. Relay shape unchanged.
- Whole-local-kernel ordinary authority includes all sessions, workflows, Apps and kernel-owned credential flows. No access levels. Remote-kernel admission/attachment, critical/passkey decisions, secret disclosure/export and grant mint/extension remain human-only. Ordinary revocation/listing stays available.
- Every external `/sudo` request uses the existing fresh-passkey terminal popup and kernel-managed target run; no remembered approval and no holder elevation. Defaults480/max1440 minutes, extend notice5; process/expiry/restart/explicit/rotation revocation retained.
- Updated kernel access plan D6/D9, sections5–6/threat model, protocol and sudo docs; kit/receipt projections no longer contain grant session fields.

## MP-08 / MP-10 / MP-11 — Cloud follow-up for coordinator
- Read-only Cloud audit: `/root/work/cloud` HEAD `d1be0aa1a5328c646003771761f30cdcd1e8e01f` has no `access_grant`, `grant.session_id` or `passkey_prompts_changed` match under `apps/web/src`. No external-grant session text found at that identity. Cloud was not changed; audit actual publishing head if different.
- Access/extension popups should say `Local kernel`; remove session selection and grant-session labels/types. Route replies on `kernel-access`, handle `KernelAccessDecisionResponded { interaction_id }` without SessionState, and gate this behavior at local451. Sudo/critical target-session attribution remains.

## Owner questions — MP-08 / MP-10 / MP-11
- BLOCKED: supply an approved Chariox-materialized CODEX_HOME path or documented materialization command for the standalone official Codex drill. Safe public status reports authenticated profiles but deliberately hides paths. Asked while continuing independent work; no answer yet. No provider-account files were inspected/copied; no root/.codex-agents profile was used for drills.
- BLOCKED: supply an approved real App package/service for install/bind acceptance. The shared Apps machine is explicitly off limits.
- BLOCKED: supply a permitted hosted-relay endpoint and approved drill identity/network path. The reserved relay machine is explicitly off limits. No hosted, public-site/DPR/timing/soak result claimed.
- Required official-provider/user-typed-passkey cross-session/spawn/App/refusal/sudo/lifecycle live matrix has NOT passed. Fixtures and popup refusal remain supplementary; nothing is accepted or ready for staging.

## Coordinator asks — MP-08 / MP-10 / MP-11
- Unblock the three resources above and arrange the full live matrix before publishing/staging. Cloud follow-up above is required when adopting the new wire behavior. No new protocol allocation requested.
- MP-11 narrowed scope applies to KA/passkey, Vault, relay admission and signal boundaries; unrelated source does not create an open exact-blob review requirement.

## MP-08 / MP-10 / MP-11 — resource ownership and cleanup
- Shared compile lock used for every cargo/test invocation; jobs4. Resource samples retained; no shared process/cache/container/reviewer or protected credential/key path changed.
- Popup harness retries fixed terminal capture compatibility and a shared default-MCP-port collision by assigning dynamic lane ports and explicit socket. These were setup failures, not accepted product results.
- All lane popup processes and disposable product identity/Vault state were removed after exact-child positive-PID checks. Final cleanup inventory is recorded in evidence.
