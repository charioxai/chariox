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

## In progress / notes for the next agent

- Phase 1 remaining, in priority order:
  - **B2 (inject_prompt PTY sleeps)** — `app/provider_output_claude_native.rs:~1446,1453` still `std::thread::sleep(250ms)` under the app lock between writing prompt text and the `\r` submit. Fixing needs splitting inject into write-text / off-lock-wait / write-Enter; PTY-timing sensitive, verify end-to-end with the headless provider before trusting it.
  - **B3 relay off-lock** — `app/relay_runtime.rs:56` `block_on_relay_future` + ~20 callers in `app/remote_agent_binding.rs`, `app/remote_workspace_live_sync_fanout.rs`. Snapshot inputs under lock → spawn relay future off-lock → apply result via follow-up command. Coordinate with M4.5 ownership migration order.
  - **B4 write-path** — history *reads* already use `spawn_blocking` (`runtime/history_requests/*`); remaining is transcript *append* on the fanout/prompt-transcript path (`app/provider_output_fanout.rs:438`, `runtime/state/prompt_transcript_owned_state.rs`). Watch transcript ordering.
  - **B6 poisoning policy** — `provider/run_actor/runtime_slots.rs`, `slice/store.rs` std-Mutex `.expect("… poisoned")`; consider `parking_lot`.
- Already-done-in-tree (no action needed): B5 blocking HTTP `http_request_with_credential` is wrapped in `spawn_blocking` at `runtime/state/tool_dispatch/credential.rs:287`; history read requests use `spawn_blocking`.
- Use daemon health `app_lock` (B1) to measure before/after for any B2/B3/B4 change.
- arroba-cloud `apps/web` is red on main: `ui/routes.test.js` asserts on terminal-app source that is mid-refactor by a parallel agent (uncommitted work in `apps/web/src/terminal`). Not touched on purpose; CI will go green when that lands. Everything else (api, worker, packages) is green — verified in a clean worktree.
- arroba-cloud has parallel-agent activity under `apps/web/src/terminal` — stage only your own files, never `git add -A` there.
- Both repos commit directly to `main`; rebase before push if the remote advanced (use `--autostash` in arroba-cloud).
