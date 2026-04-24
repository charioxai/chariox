# M12 Controlled Exec And In-Turn Interaction Spike Plan

Status: validated on 2026-04-23.

This spike validates whether Arroba can own shell-style execution permissions
and mid-turn user interactions in a provider-independent way, before
implementing that control path in production kernel/runtime code.

## Goal

Prove these claims:

- Arroba-controlled command execution can feel close enough to provider-native
  bash for normal agent work.
- Arroba can enforce a simple, provider-independent permission model:
  `limited`, `yolo`, `yolo+rm`.
- `yolo` can block deletion of paths not created by the same agent while still
  allowing normal iterative work.
- A turn can pause for user input and continue in the same turn after a quick
  response.
- The same interaction model can serve both permission prompts and simple
  multiple-choice clarifications.

## Non-Goals

- No production replacement of provider-native bash in this spike.
- No full TUI popup implementation inside the main CLI.
- No remote execution semantics in this spike.
- No freeform user text responses in v1.
- No multi-select answers in v1.
- No workflow integration in this spike.
- No attempt to perfectly replicate every provider-native shell affordance.

## Decision Under Test

Arroba should own the execution gate for any operation that must obey a common
permission policy across providers.

That means testing an Arroba-supervised command tool instead of relying on
Codex/OpenCode native shell permissions.

If the spike fails, v1 should not promise uniform permissions across providers.
In that case, Arroba would fall back to provider-native execution behavior and
limit itself to coarse launch-time sandbox settings.

## Permission Model Under Test

The spike validates this user-facing model:

- `limited`
  - execution requires explicit approval
  - writes can require approval
  - deletes require approval
- `yolo`
  - execution auto-allowed
  - writes auto-allowed
  - delete allowed only for paths created by the same agent during the current
    session lifetime tracked by Arroba
- `yolo+rm`
  - execution and file deletion fully auto-allowed

The important rule is that these are Arroba permissions, not provider
permissions.

## Prototype Location

```text
experiments/controlled-exec-spike/
  README.md
  package.json
  src/
    controlled-exec-harness.mjs
    permission-ledger.mjs
    interaction-gateway.mjs
    fake-agent.mjs
    scenarios.mjs
  artifacts/
```

The prototype should be Node-based so it can reuse existing Arroba drill
conventions and local provider binaries quickly.

## Architecture Hypothesis

The spike tests this boundary:

```text
provider/model
  asks for command execution through an Arroba-like runtime tool

controlled exec gateway
  validates policy
  optionally pauses for user interaction
  executes the command in a supervised child process
  returns stdout/stderr/exit status without exposing forbidden operations

interaction gateway
  blocks the tool call pending a user response
  returns confirm/choice selections
  allows same-turn continuation

permission ledger
  tracks path creation provenance for yolo delete checks
```

## Test Questions

The spike must answer:

1. Can agents do normal command-driven work through Arroba-controlled exec
   without the experience degrading too much?
2. Is a simple permission gate sufficient for the common cases?
3. Can `yolo` path ownership checks be implemented with a lightweight ledger?
4. Can a command pause on approval and continue in the same turn?
5. Can the same pause/resume path handle non-permission multiple-choice
   interactions?
6. Which operations, besides shell execution, must move under Arroba control to
   keep the permission story coherent?

## Scenarios

### S1 Basic Command Execution

Fake agent asks the runtime to execute:

- `pwd`
- `ls`
- `git status`
- `echo ... > file`

Success:

- command output is returned cleanly
- exit code is preserved
- normal non-destructive work behaves predictably

### S2 Limited Approval Flow

In `limited`, fake agent requests:

- non-destructive command
- write command
- delete command

Harness pauses and simulates a user choice.

Success:

- denied commands do not execute
- approved commands execute once
- tool call result indicates approval outcome
- same-turn continuation remains understandable

### S3 Yolo Delete Provenance

Agent creates `tmp/owned.txt` through the controlled path, then tries to remove:

- `tmp/owned.txt`
- a pre-existing fixture file
- a directory containing mixed-owned content

Success:

- owned file delete is allowed
- pre-existing file delete is denied/escalated
- mixed ownership is denied/escalated

### S4 Yolo+Rm

Agent deletes a pre-existing disposable fixture tree.

Success:

- delete proceeds without approval
- command result is returned normally

### S5 Mid-Turn Choice Question

Fake agent requests a simple choice:

```json
{
  "kind": "choice",
  "title": "Pick target",
  "message": "Which file should I edit?",
  "choices": ["a", "b", "c"]
}
```

Harness answers one choice and the fake agent continues using that answer in
the same turn.

Success:

- tool call blocks until answered
- returned answer is structured
- the turn continues without manual restart

### S6 Provider-Backed Drill

Use one real Codex run and one real OpenCode run with a tightly constrained
prompt:

- instruct the agent to use only the Arroba-like controlled exec tool
- perform harmless shell work
- trigger one approval
- trigger one simple choice question

Success:

- both providers can operate through the controlled tool path
- same-turn continuation is reliable enough for v1

## Interaction Contract Under Test

The spike should validate a kernel-style interaction object:

```json
{
  "interaction_id": "uuid",
  "turn_id": "uuid",
  "agent_id": "agent-1",
  "kind": "permission|confirm|choice",
  "severity": "info|warning|critical",
  "title": "string",
  "message": "string",
  "choices": [
    { "id": "yes", "label": "Yes" },
    { "id": "no", "label": "No" }
  ],
  "default_choice": "yes"
}
```

Response shape:

```json
{
  "interaction_id": "uuid",
  "choice_id": "yes"
}
```

## Progress

- M12.1 spike plan and scaffold: complete.
- M12.2 controlled exec harness: complete.
- M12.3 permission ledger for owned-path deletion: complete.
- M12.4 interaction gateway with confirm/choice: complete.
- M12.5 fake-agent deterministic scenarios: complete.
- M12.6 provider-backed Codex/OpenCode drills: complete.
- M12.7 recommendation: complete.

Production integration can start. The spike passed the provider-backed gate.

## Success Criteria

The spike passes if:

- `limited`, `yolo`, and `yolo+rm` behave deterministically in local harness
  scenarios
- at least one real Codex run and one real OpenCode run can use the controlled
  exec path successfully
- approval and choice interactions resume the same turn without requiring a
  fresh user prompt
- normal shell work remains usable enough that replacing provider-native bash
  is credible for v1

Validated commands:

```bash
cd experiments/controlled-exec-spike
npm run check
npm test
npm run drill:fake
npm run drill:providers
```

Validated provider-backed result on 2026-04-23:

- Codex called the disposable `request_choice` and `controlled_exec` MCP tools
  and wrote `codex-choice.txt` with `codex:beta`.
- OpenCode called the same MCP tools and wrote `opencode-choice.txt` with
  `opencode:beta`.
- Both providers completed the same-turn flow and returned the required success
  markers.

Artifacts from the live provider drill are written under:

```text
experiments/controlled-exec-spike/artifacts/<timestamp>-provider-drill/
```

Validated popup-blocking result on 2026-04-24:

- The interaction gateway was extended with a blocking `request_popup` path,
  timeout handling, and default-on-timeout resolution.
- Fake drills proved that a popup can:
  - block until timeout and resume with the configured default reply
  - block until an external resolver answers later and then resume
- Codex and OpenCode were both re-drilled with a forced delayed popup response.
  Both providers waited on the popup tool call and still completed the same turn.

Latest popup-blocking live artifacts:

```text
experiments/controlled-exec-spike/artifacts/2026-04-24T11-19-09-661Z-provider-drill/
```

## Recommendation

Adopt this direction for production M12.

The spike shows that:

- Arroba can own the execution gate without breaking normal command-driven work
- the `limited` / `yolo` / `yolo+rm` model is implementable at the Arroba layer
- same-turn confirmation and single-choice interactions are viable through the
  runtime MCP boundary
- synchronous popup interactions are viable through the same blocking tool-call
  model, including timeout/default fallback

The remaining production question is not feasibility but scope: which
provider-native operations must be suppressed or replaced so the Arroba
permission model remains coherent for end users.

## Failure Criteria

The spike fails if:

- command behavior is too degraded compared to provider-native bash
- same-turn continuation after approval/choice is too unreliable
- path-ownership checks become too brittle or expensive
- too many additional operations must move under Arroba control just to make
  the permission model coherent

## Expected Follow-Up If The Spike Passes

Production M12 should then implement:

- Arroba-owned permission levels on agents
- controlled execution through the runtime MCP
- client-rendered interaction prompts in CLI and shell
- provider launch changes that discourage or disable direct provider-native
  shell when uniform permissions are required
