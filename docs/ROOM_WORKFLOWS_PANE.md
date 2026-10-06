# Room workflows pane

Local daemon protocol 439 adds a kernel-owned room workflow inventory. Relay peer
protocol is unchanged: this uses the existing home-session attachment stream.

`GetSessionState` returns `SessionState.room_workflows`; `session_snapshot` carries
the same `room_workflows` field. `room_workflows_changed` carries a full inventory
replacement as `inventory`. Initial attach and replay-gap snapshots include an
explicit empty inventory when the room has no definitions. Definition changes
continue to reach existing canvas consumers through session snapshots.

The inventory identifies `session_id`, `home_kernel_id`, a semantic content
`revision`, and `workflow_count` (including blank definitions). `workflows` hold
public labels, definition revision, endpoints, current run summaries and queued
counts. One manual row corresponds to each endpoint, regardless of schedules,
HTTP publications or App events. There are no trigger badges or deployed-copy
run counts. Creation order/id and endpoint definition order provide stable rows.

A workflow is running when any room-local run is Created, Running, Waiting or
Completing. Paused-only workflows are paused; running plus paused is mixed;
queued-only remains idle with a queued count. Terminal archived runs do not keep
the pane running. Run summaries exclude prompts, provider traces and agent
bindings. Start capabilities preserve endpoint ownership; room membership and
existing metaagent scope govern run controls.

`ControlRoomWorkflowRuns` takes `session_id`, `workflow_id`, `action`
(`pause`, `resume`, `stop`) and explicit captured `run_ids`. It validates exact
room/workflow run identity, calls existing durable per-run controls and returns
`RoomWorkflowRunsControlled { results, inventory }`. Results identify each run
and outcome (`applied`, `unchanged`, `failed`) with an optional error. A run that
finished since capture is unchanged. Runs admitted later are never added to the
selection. Schedules, notifications, HTTP and queued admission remain active;
these controls do not suspend the workflow or promise atomic rollback.

The pane feature minimum is 439. Older kernels retain existing terminal
functionality. Client dismissal, selection and drafts are presentation state,
keyed by client plus home kernel and room. A nonempty authoritative inventory
opens the pane unless dismissed; authoritative empty clears dismissal and hides
it. Offline/unknown retains stale rows and disables mutations until resynced.

Web rows select the independent bottom composer; Enter starts a nonblank prompt
and Shift+Enter inserts a newline. Manage opens the existing workflow canvas.
The room header reopens a dismissed pane. On narrow screens, Agents returns to
agent panes without deleting workflow selection or drafts.

In the TUI, Ctrl+W opens/focuses the pane, Ctrl+Up/Down selects endpoints,
Ctrl+P/R/S pauses/resumes/stops the selected workflow's captured current runs,
and Ctrl+G opens Manage. Enter starts from the workflow composer. Tab returns
to agents; Escape dismisses the pane. The pane occupies a separate side column
on wide terminals and a room screen on narrow terminals.
