# Row 432: a lost watched process says nothing useful

Base: union L14 head `0a04a47f5d1` (frozen, L14 not landed). No protocol bump.

## Where a watched process lives

- The **wake ledger** (`durable_state/agent_lifecycle`, `AgentWake`, kind `process`) is the durable
  record: `created_at_ms`, `verified_at_ms` (set once by `ProcessStarted`), `pid`, `label`, `command`,
  `state` (`starting|running|cancelling|exited|lost`). The supervised child itself (pid, birth,
  pipes, output tail) lives only in memory (`WatchedProcesses.live`).
- **Hot reload (#1499, `agent_process_watch/handover.rs`)**: the old program `hand_over()`s each live
  process (pid, birth, the read ends of its output pipes, cgroup, output snapshot); the new program's
  `adopt_watched_processes` supervises it again under the same wake id. Gone/unadoptable ->
  `ProcessExited{None, "watch_lost_on_reload: process N gone, exit unknown"}`.
  So a hot reload now KEEPS the watch and the owner is woken with the real exit
  (`hot_reload.rs::a_watched_process_still_wakes_its_agent_after_a_hot_reload`). The 11-23 widening
  is fixed by #1499; cell 1 needs no code. (Verified live: see report.)
- **Cold restart**: nothing is handed over. The scheduler's first tick runs
  `recover_lost_agent_processes` (`agent_wake_scheduler.rs`): every `process` wake still
  `starting|running|cancelling` that is older than this kernel's `started_at_ms` is marked `lost`
  (`ProcessExited{exit_code: None}`), orphans carrying its wake marker are stopped
  (`reap_orphans`), the owner gets an inbox event and a room notice.

## What the report says today

- Answer (`output_tail`): `process_lost: the kernel restarted; it is not supervised and is not relaunched`
  (+ orphan suffix).
- Room notice: `Wake alert: watched process '<label>' was lost when the kernel restarted; it is not relaunched`.

Missing: when it was last seen, whether the process ended while the kernel was down or was stopped
by recovery, that the job behind it is gone, and what to do. The ledger already holds the data
(`verified_at_ms`/`created_at_ms`, the `reap_orphans` count per wake); nothing new has to be stored.

## What it should say (one short sentence, plain words, no new tool)

- Process found running at restart and stopped:
  `'<label>' was last seen running at <UTC time> (3 h ago); the kernel restarted and stopped it, so it and the job behind it are gone and its exit is unknown: start it again with chariox.events.process and watch the new one.`
- No process found (it ended while the kernel was down):
  `'<label>' was last seen running at <UTC time> (3 h ago); it ended while the kernel was down, so it and the job behind it are gone and its exit is unknown: start it again with chariox.events.process and watch the new one.`

The same sentence goes in `output_tail` (prefixed `process_lost: ` so existing readers keep working)
and in the room notice. The orphan caveats (unconfirmed settlement, cleared environment) stay appended.
"Last seen" is the last time the kernel confirmed the process alive: `verified_at_ms`
(`ProcessStarted`), falling back to `created_at_ms`. There is no periodic proof of life for a process,
so for a long build it is the start time; the sentence says "last seen running", not "ran until".
Exit code: unknown on a cold restart (the kernel was not there to wait on it), so it is never
guessed; hot reload keeps the real exit.

## Smallest fix

In `recover_lost_agent_processes` only (about 25 lines): a `lost_watch_sentence(label, last_seen_ms,
now, stopped: bool) -> String` helper (age as `N h`/`N min`/`N s`, UTC `HH:MM:SS`), used for both the
`tail` and the `wake_notice`. No ledger, protocol, or tool schema change; `process_lost` stays in the
tail so `process_lost: true` in the answer is unchanged. The tool description for
`chariox.events.process` (`agent_events.rs`) already says a restart reports `process_lost`; leave it.

## Tests

`provider_output_runtime_tests/agent_wake_sources/lost_process.rs` (lib tests, real runtime state,
real ledger):
- cell 2: a process wake started 3 h before the restart -> answer and notice say "last seen", "3 h",
  "gone", "start it again".
- cell 3: no process of the watch left to stop -> answer and notice say "ended while the kernel was down"
  and "exit is unknown".
- cell 1: existing `hot_reload.rs::a_watched_process_still_wakes_its_agent_after_a_hot_reload` (must pass on the union head).
