# Cross-Repo Review Remediation — Progress Log

Companion to [`CROSS_REPO_REVIEW_REMEDIATION_PLAN.html`](CROSS_REPO_REVIEW_REMEDIATION_PLAN.html).
One line per landed task; keep this file short. Format: `date task — outcome (commit repo)`.

## Landed

- 2026-07-06 Plan + progress log committed (arroba).
- 2026-07-06 A3/A4/A5 relay auth hardening — constant-time shared-token compare, clock fail-closed, open-access opt-in (`f721ce1bd` arroba).
- 2026-07-06 A1/A2 cloud fail-closed startup — no silent dev secrets, prod guard on testAuth0IdentityHeader, `ARROBA_CLOUD_COOKIE_SECRET`/`ARROBA_CLOUD_CSRF_HMAC_KEY` env path (`118e6289` arroba-cloud).
- 2026-07-06 A3-TS timingSafeEqual in relay-tokens (`9751e8f3` arroba-cloud).
- 2026-07-06 A6 CI workflow for arroba-cloud (`834438b3` arroba-cloud). **Phase 0 complete.**
- 2026-07-06 B1 app-lock instrumentation — `runtime/app_lock.rs`, surfaced as daemon health `app_lock`; boundary test polices the helper (`9a554c1fc` arroba).
- 2026-07-06 B2 (dispatch) claude-headless injection retry moved off the app lock (`511297d07` arroba).
- 2026-07-06 B2 (Stop) claude-headless Stop/SessionEnd 300ms drain moved off the app lock via deferred-drain hint; B5 hex_bytes buffer cleanup (`d97def26a` arroba).
- 2026-07-06 B6 lock-poisoning recovery for `provider/run_actor/runtime_slots.rs` (`6efc4d201`) and `slice/store.rs` (`7991f41f0`) — poisoned guards recover via `PoisonError::into_inner` instead of cascading daemon-wide panics.
- 2026-07-06 B2 (inject) claude prompt Enter keystroke submitted off the app lock via timestamped `submit-wait` marker; removes the last 250ms under-lock sleeps in the injection path (`ce031a510` arroba). **B2 complete.**
- 2026-07-06 B4 (partial) home provider-auth scan cached behind a 5s TTL so `relay_registration` stops re-reading four files under the app lock per registration/peer request (`dc0fdece5` arroba, folded into a parallel commit).
- 2026-07-06 C1 client-supplied token TTLs capped server-side — shared `token-ttl-limits.ts`, per-type maxima, reject >10x (`6926ee6d` arroba-cloud).
- 2026-07-06 C4 report-only CSP on the cloud API with strict script-src; `ARROBA_CLOUD_CSP_MODE` selects report-only/enforce/off (`71cfe91e` arroba-cloud).
- 2026-07-06 C2 (enforcement) bounded revocation denylist in the relay scoped verifier — `RelayRevocationRegistry` rejects revoked `jti`/`account_id`, prunes on expiry (`d1aab6fb1` arroba).
- 2026-07-06 C3 (partial) relay rejects tokens issued implausibly far in the future (60s skew tolerance) (`24c373a19` arroba).
- 2026-07-06 C3 (drift) `session_id` added to Rust relay claims + cross-impl conformance test verifying a TS-issued JWT (`b463db17d` arroba, `02259302` arroba-cloud). Both repos verify one shared fixture; wire-shape drift fails a conformance test.
- 2026-07-06 F1 repo hygiene — untracked scratch strays, gitignored worktree/drill dirs (`ffd345c0b`, `98e85cdb9` arroba).
- 2026-07-06 F2 README slimmed; milestone prose moved to `docs/STATUS.md` (`98e85cdb9` arroba).
- 2026-07-06 D1 root Cargo workspace (kernel + relay + adapters, one lockfile); CI fmt/clippy/test now cover all crates (`f4509b45f` arroba). **Phase 3 complete.**
- 2026-07-06 E3 (substantially done) `apps/api/src` restructure — 14 prefix families / 91 files moved into subdirectories (billing, device-login, pairing, account, publication, managed-history, relay, runtime, cloud-api, browser, shared-session, machine-runtime, local-browser, hosted-relay); top-level `.ts` down from 90 to 28 (rest are singletons). `architecture-boundaries.test.ts` updated for the new layout. Commits `d54e7192`→`5b61b3be` arroba-cloud; all green.

## In progress / notes for the next agent

- Phase 1 remaining (both need live end-to-end verification with the app running):
  - **B3 relay off-lock** — `app/relay_runtime.rs:56` `block_on_relay_future` + ~20 callers in `app/remote_agent_binding.rs`, `app/remote_workspace_live_sync_fanout.rs`. Full multi-round-trip remote-binding flow (with error-cleanup closures) under `&mut DaemonApp` while the app lock is held. Snapshot inputs under lock → spawn relay future off-lock → apply result via follow-up command. Coordinate with M4.5; verify remote-agent binding live.
  - **B4 write-path (transcript append)** — transcript *append* on the fanout/prompt-transcript path (`app/provider_output_fanout.rs:438`, `runtime/state/prompt_transcript_owned_state.rs`). Appends return `HistoryEvent` used by callers, so a naive `spawn_blocking` reorders; do it as a single-threaded writer actor to preserve ordering. Reads already use `spawn_blocking`.
- Phase 2 remaining:
  - **C2 sync feed** — enforcement primitive is in (`RelayRevocationRegistry`, wire via `ScopedTokenVerifier::with_revocations`). Remaining: cloud exposes revocations (`account-admin-revocations`) and the relay periodically pulls + calls `revoke_token_id`/`revoke_account`/`prune`. Needs a relay HTTP client + cloud endpoint; verify live.
  - **C3 format consolidation** — relay still accepts both `arroba-scoped-v1` and JWT with duplicated parsing; consolidate on JWT behind a deprecation window (keep accepting the old format one release with a warning metric, then delete `decode_scoped_token_parts`/`encode_scoped_hmac_token`). `session_id`, future-iat/skew, and the shared conformance fixture are done.
- Phase 4/5 remaining: **E3** only the 28 top-level singletons/2-file leftovers remain (they don't need directories); the multi-file family regroup is done. The move+rewrite recipe: git mv `prefix-*.ts`→`prefix/*.ts`, rewrite moved files' `./x`→`../x` keeping intra-family siblings colocated, rewrite importers' `./prefix-x.js`→`./prefix/x.js`, then `pnpm --filter @arroba-cloud/api build && test`. **E1** (`terminal-browser-app.ts`), **E2** (kernel `workflow_code.rs` — see `WORKFLOW_CODE_OVERARCHING_PLAN.html`), **E4** (styles→CSS) are in files a parallel agent edits; slice opportunistically. **F3** projection clone reduction — measure with B1 `app_lock` on a running daemon first.
- Already-done-in-tree: B5 blocking HTTP wrapped in `spawn_blocking` at `runtime/state/tool_dispatch/credential.rs:287`; history reads use `spawn_blocking`.
- Use daemon health `app_lock` (B1) to measure before/after for any B3/B4/F3 change.
- Known pre-existing failures on `main` (NOT caused by this work, verified via clean-tree stash): `lib_tests::provider_sessions::prompt_submission_queues_and_notifies_other_attachments` and `local::api::tests::workflow_definition_control::local_request_api_runs_workflow_code_with_generated_agent`. Worth a separate fix; they will block a fully-green `cargo test`.
- arroba-cloud `apps/web` is red on main: `ui/routes.test.js` asserts on terminal-app source that is mid-refactor by a parallel agent (uncommitted work in `apps/web/src/terminal`). Not touched on purpose; CI will go green when that lands. Everything else (api, worker, packages) is green — verified in a clean worktree.
- arroba-cloud has parallel-agent activity under `apps/web/src/terminal` — stage only your own files, never `git add -A` there.
- Both repos commit directly to `main`; rebase before push if the remote advanced (use `--autostash` in arroba-cloud).
