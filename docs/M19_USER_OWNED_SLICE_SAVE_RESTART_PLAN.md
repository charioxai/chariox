# M19 User-Owned Slice Save And Restart Plan

## Objective

Make reusable slice state a user-owned lifecycle operation. Agents should keep using slices and runtime MCP tools, but agents should not be able to save or checkpoint the slice image themselves in this version.

The user-facing save action must support office-work slices where an agent has logged in to services, installed programs, or changed local application state. The primary flow is:

1. User opens a slice-backed terminal view.
2. Agent works in the slice.
3. User clicks `Save and restart agents`.
4. Arroba stops the slice, saves its state, starts it again from that state, and relaunches the attached idle agents so they continue in the saved slice.

## Scope

### In Scope

- Remove agent-accessible runtime MCP slice save and backup tools.
- Keep user/kernel-owned slice state APIs.
- Add explicit save modes for attached-agent slices:
  - `save_restart_agents`
  - `save_shutdown`
- Reject save if any attached managed agent is currently running a prompt/provider turn.
- Reconcile stale slice agent attachments before validation.
- Restart idle attached agents after a user save/restart.
- Add controls in the web terminal view page that contains the slice screen, prompt area, and agent pane.
- Validate through live web-terminal drills with screenshots.

### Out Of Scope

- Agent-triggered image save/checkpoint.
- Arbitrary process supervision inside the container.
- Multi-kernel ownership of a single running slice container.
- A dedicated slice settings area.
- Guarantees that external services, especially Google, will preserve authenticated sessions after a cloned/restored browser profile.

## Source Of Truth

The home kernel remains the authority. Active slice agents are determined from Arroba state, not from Docker process inspection.

Agent categories:

- Managed active agent: an Arroba agent attached to `slice.agent_ids`, with a live session and a remote execution binding matching the slice worker.
- Busy active agent: managed active agent with an in-flight prompt/provider run. Save is rejected.
- Idle active agent: managed active agent attached to the slice with no running turn. Save/restart may relaunch it.
- Stale attachment: `slice.agent_ids` entry whose session/agent no longer exists or whose remote binding no longer matches the slice. Reconcile and detach before save validation.
- Foreign process: any process not represented by Arroba home-kernel state. Preserve filesystem state, but do not manage or relaunch it.

## Kernel Changes

1. Extend `SliceStateSaveRequest` with an explicit save mode:
   - `restart_agents`
   - `shutdown`

2. Bump the local daemon protocol version and update protocol shape/hash tests.

3. Add a slice save coordinator below clients:
   - resolve slice;
   - acquire the slice operation lock;
   - reconcile stale attached agents;
   - inspect attached managed agents;
   - reject if any managed agent is busy;
   - capture relaunch manifests for idle attached agents;
   - execute the requested save mode.

4. Relaunch manifest fields:
   - home session id;
   - home agent id;
   - provider adapter/provider/model/account/variant;
   - provider session id when known;
   - current remote execution binding;
   - effective permissions and execution mode.

5. `restart_agents` flow:
   - park/end idle provider runs for attached agents;
   - stop the slice container;
   - save stopped Docker image and home archive;
   - persist the new slice state record;
   - start the slice from the saved state;
   - discover the new worker kernel;
   - recreate or repair leased-agent bindings;
   - relaunch provider runs using the captured provider session ids where available;
   - emit audit/progress events.

6. `shutdown` flow:
   - park/end idle provider runs for attached agents;
   - stop the slice container;
   - save stopped Docker image and home archive;
   - persist the new slice state record;
   - leave the slice stopped and agents remote-unreachable until the user starts it again.

7. Keep `/slice start <slice-ref>` as the way to start a stopped existing slice without creating a new session or agent. Ensure attached agents can repair/rebind when the slice becomes reachable again.

## Runtime MCP Cleanup

Remove agent-facing slice state tools:

- `arroba.save_slice_state`
- `arroba.create_slice_backup`

Also remove dispatch/proxy code and relay peer request paths that only exist to let a runtime MCP call save or backup slice state.

Keep user-facing commands and APIs for:

- save state;
- backup;
- reset state;
- start;
- stop.

## TUI UX

Update slash commands:

- `/slice save-state <slice-ref> --restart-agents`
- `/slice save-state <slice-ref> --shutdown`
- `/slice start <slice-ref>`

If attached agents exist and no mode is provided, show usage instead of silently choosing behavior.

Error copy:

- Busy agents: `cannot save slice while agents are running; wait for them to finish or stop them: <ids>`.
- Relaunch failures: list affected agent ids and the next repair action.

## Web View UX

Add slice controls to the existing web terminal view page that contains:

- slice screen;
- prompt area;
- agent pane.

Controls:

- `Save and restart agents`
- `Save and shut down slice`
- `Start slice` when stopped

Behavior:

- show a confirmation before restart/shutdown;
- show progress states for stopping, saving, starting, relaunching, and completion;
- surface busy-agent rejection clearly;
- keep controls scoped to the current slice-backed session/agent view.

## Validation

### Automated Tests

Kernel:

- save rejects a busy attached agent;
- save restart succeeds for idle attached agents;
- save shutdown leaves the slice stopped;
- stale `agent_ids` are reconciled;
- relaunch manifests preserve provider session id when available;
- protocol shape/version tests are updated.

CLI:

- new flags parse correctly;
- attached agents require explicit mode;
- `/slice start <slice-ref>` works for existing stopped slices.

Web:

- controls render in the terminal view page;
- buttons send the correct save mode;
- loading, confirmation, success, and error states render correctly.

### Live Drill

Run the final drill from the web terminal view page, not by manually driving Docker or the browser outside Arroba.

1. Open a headed slice-backed web terminal session.
2. Ask the agent to install a simple program inside the slice, for example `jq` or `htop`, and verify it runs.
3. Ask the agent to log into Gmail using vault-backed password insertion and runtime MCP slice/browser tools.
4. Ask the agent to send a first email to confirm it can drive Gmail.
5. As the user, click `Save and restart agents` in the web terminal view.
6. Verify the UI shows save/restart progress and the same agent becomes reachable again.
7. Ask the relaunched agent to verify the installed program still exists and runs.
8. Ask the relaunched agent to open Gmail and send a second email.
9. If Gmail requires re-login, record that as external service session invalidation while still validating saved slice state through installed program persistence and preserved browser/account state.

### Artifacts

Save screenshots under `./.artifacts`:

- web view before save;
- save confirmation/progress;
- restarted slice view;
- agent proof that the installed program survived restart;
- Gmail sent or Gmail re-login/block proof.

Report exact filenames and display the screenshots in Codex at the end of validation.
