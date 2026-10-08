# MP-08 / MP-09 / MP-10 / MP-11: durable agent wake drill

Run this drill on the exact source being reviewed, using its built TUI, kernel
and an authenticated official provider. Repeat the scheduling request on the
base to establish a failing comparison. Component tests supplement these runs.
Keep state and evidence outside the checkout; use a product-linked provider
profile without reading or copying its credentials.

## MP-08 / MP-09: missed wakes and delivery receipts

1. Through the TUI, ask the agent to schedule a one-minute check-in and a
   thirty-minute timer using `chariox.events.timer`. Ask it to list them with
   `chariox.events.wakes`, then yield on both registrations with a finite
   deadline. Record the first expected fire and scheduler confirmation.
2. Restart only the drill's kernel across the check-in's due time. Reconnect
   the TUI. The overdue occurrence must fire once and the real agent must
   handle it, acknowledge it, and yield on the remaining timer.
3. Suspend and resume only the drill's kernel across the thirty-minute due
   time to simulate host sleep. Verify the overdue fire, lateness notice,
   delivery and acknowledgement, and the agent's final disposition.
4. Repeat with a one-minute recurring check-in, missing several intervals
   across restart. Verify one coalesced fire, its missed count, the next due
   time, and independent receipts for each fire. Let three handled
   check-ins pass without other progress. The third must block the task with
   one owner interaction; the fourth must not start a provider turn. Verify
   the visible owner gate, explicitly Resume, and confirm scheduling resumes
   on the original task. In a separate pass choose Cancel and confirm wakes
   retire without recording cancellation as a fire.
5. Disconnect the TUI during a wait and reconnect. `/agent wakes` must show
   the same registrations, next fire, last fire and last receipt. A future
   timer alone must not count as active managed work.

## MP-09: stalled scheduler and delivery supervision

For an isolated fault run, set `CHARIOX_WAKE_SCHEDULER_STALL_FILE` to an
absolute lane-owned path outside the repository before starting the kernel.

1. Create that file, then ask the real agent to schedule a near-term timer.
   Creation must report absent scheduler confirmation and show a notice.
2. Leave it present across the due time. The thirty-second lifecycle sweep
   must report the stalled scheduler and fire the overdue timer within the
   fifteen-second fire tolerance plus one sweep. The agent handles the wake.
3. Remove the file and verify recovery without a second logical fire.
4. Let a real agent remain busy across a timer's due time and delivery
   tolerance. Verify the visible delivery alert, then ordinary delivery and
   handling when the provider becomes available. No duplicate turn is allowed
   when provider acceptance is uncertain.

## MP-08 / MP-09 / MP-11: owned processes

The current process tool requires a local unrestricted agent on Linux. It
fails closed on other hosts until native process ownership can be verified.
Timer scheduling does not have this platform restriction. On a managed kernel
the command runs inside the same managed provider isolation as every other
agent-launched command, or is refused when that isolation is unavailable.

1. Ask the agent to run its actual repository test/build command through
   `chariox.events.process`, watch meaningful output and exit, schedule a
   check-in, and yield. Verify the actual test/build result and each receipt.
2. Repeat with `gh pr checks --watch` on an approved real public PR using an
   approved GitHub CLI account. Missing authentication is a blocker, not a
   passing check.
3. Cancel an owned running command through the provider/user flow. Its
   obligation must remain unresolved until physical exit is confirmed.
   Verify that every owned descendant settles and no unrelated process changes.
4. Lose a watched process, then separately restart the kernel while watching.
   Both must produce an explicit failure wake without automatically relaunching
   the command. The agent must re-evaluate or become visibly blocked. Include a
   descendant that leaves the process session: a graceful kernel stop settles
   it before exit; after a hard kernel kill the restarted kernel stops it and
   reports the count in the lost answer. On managed kernels the Bubblewrap PID
   namespace settles every descendant. Unmanaged kernels find a descendant that
   left the session only by its inherited `CHARIOX_AGENT_WAKE_ID`; one that also
   cleared its environment (`env -i`, `sudo`, `systemd-run`, daemonizers) is not
   attributed at exit, stop or restart, and the lost answer says so.
5. End the Room or delete the agent while wakes are armed and an unowned
   (delivery-blocked) task exists. Teardown must complete and retire the wakes.
6. Supplement the real command runs with tests of protected output, unknown
   protection state, bounded drains, stale process ownership, reserved signal
   targets, and resisting descendants. Report these as security regressions.

## MP-10 / MP-11: evidence and placement

Record exact source and binary identities, commands, exit codes, resource
samples, TUI captures, fire/delivery/acknowledgement times, and owned cleanup.
Keep results attributed to the source that ran them. Repeat applicable checks
through the authorized hosted relay and normal managed STOP/start path; an
ordinary restart does not establish that placement gate. Missing accounts,
relay access or a managed machine must identify the exact owner action needed.
Do not claim an MP item closed from these focused runs alone.

## MP-08 / MP-09 / MP-10 / MP-11: cancellation and confirmation races

1. Ask a real provider to launch a child workflow that runs the repository's
   actual tests, then waits on a recurring timer. Have the creating task request
   an owner decision with `chariox.events.blocked`. Once the child is Waiting,
   choose **Cancel** in the TUI's owner interaction. Verify the returned run is
   Stopped, the child's owned resources settle, the parent's `workflow_run`
   obligation closes, and its completion source is released. A cancelled task
   with an open `cancel_requested` workflow obligation is a failure.
2. In the isolated scheduler-fault setup above, request one one-shot timer with
   `delay_ms=1000`. Remove the stall file after that provider-created timer is
   due, before its five-second confirmation wait expires. The scheduler verifies
   and fires it in the same tick. Verify the actual timer reply still reports
   `scheduler_confirmed: true` with the fired wake's exact `verified_at_ms`, then
   acknowledge its one event. Repeat on the base and retain the false-confirmation
   reply as RED evidence; firing alone does not prove the caller saw confirmation.
