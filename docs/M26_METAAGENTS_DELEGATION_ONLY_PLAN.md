# M26 Metaagents Delegation-Only Plan

## Objective

Make metaagents delegation-only for implementation work while preserving their
authority to provision capabilities for owned workers.

Metaagents should spend their budget planning, delegating, supervising, and
equipping regular agents. They must not directly implement by editing workspace
files, running shell/script/connector tools, using user MCP tools themselves, or
receiving raw credential payloads. Reading workspace context, recall, artifacts,
and slice/browser state is planning work and remains allowed when the
corresponding environment is available.

They may still:

- Install/register MCPs for the session.
- Grant and revoke MCPs/skills for owned regular agents.
- Create/update/remove vault credential handles and manage vault unlock/status
  through kernel-owned flows.
- Confirm or deny capability/credential use for owned regular agents.
- Spawn, prompt, and supervise owned regular agents.
- Create, wire, run, cancel, resume, and inspect workflows.
- Inspect session overview, events, owned-agent turns, and turn blobs.

This should be enforced by kernel policy, not only by provider prompt text.

## Current Assessment

The repo already has most primitives needed:

- `AgentRole::Meta` and `AgentInstance::is_metaagent()`.
- First-class provider execution mode through `AgentExecutionMode::Plan`.
- Runtime MCP auth tokens tied to provider runs.
- Runtime MCP tool spec generation and dispatch by auth token.
- Meta-only runtime tools under `arroba.meta.*`.
- `arroba.meta.run_command` and a meta command registry.
- Existing workflow, prompt, event, turn inspection, and interaction surfaces.
- Prompt assembly backed by Markdown template materialization.

Important current gaps:

- Metaagent provider runs can still inherit build execution settings.
- Metaagent runtime MCP specs currently include normal runtime tools before
  adding meta tools.
- Dispatch does not deny all guessed non-meta calls from a metaagent token.
- Metaagent system behavior is not loaded from a dedicated Markdown prompt
  template.
- Capability and vault provisioning need explicit meta-owned surfaces so
  metaagents can equip workers without receiving raw execution/secret tools.
- Existing drills validate broad metaagent behavior but not isolated
  delegation-only capability boundaries.

## Implementation Changes

### Launch Policy

Add a shared helper in the provider launch policy layer, for example
`apply_metaagent_launch_policy(request, agent)`, and call it from every launch
preparation path that has the target agent:

- `DaemonApp::prepare_app_provider_launch_request`
- `KernelRuntimeOwnedState::prepare_provider_launch_request`
- remote leased-agent spawn/config propagation
- remote leased native-provider launch
- prompt auto-launch paths that construct a `LaunchProviderRequest`

For metaagents, the helper must:

- Force `execution_mode = AgentExecutionMode::Plan`.
- Preserve normal permission-level selection from user/session/agent config.
- Clear user-granted MCP servers from the metaagent provider run.
- Clear worker/home remote extension manifests from the metaagent provider run.
- Preserve only the Arroba runtime MCP binding needed for `arroba.meta.*`.

This override wins over session defaults, agent overrides, native TUI flags, and
remote leased-agent inherited config.

### Prompt Assembly

Add a bundled prompt template:

```text
apps/kernel/src/provider/metaagent_delegation_instructions.md
```

Register it as:

```text
runtime/metaagent-delegation
```

Add a prompt assembly mode such as `PromptAssemblyMode::MetaagentProviderTurn`.
Prompt dispatch should select it whenever the target provider run belongs to a
metaagent.

The template must say, in concrete terms:

- You are delegation-only.
- Read workspace context, recall, artifacts, and available slice/browser state
  when useful for planning.
- Do not edit workspace files directly.
- Do not run shell commands, scripts, connectors, user MCPs, or external tools
  yourself.
- Use `arroba.meta.*` to inspect the session, create workflows, spawn/prompt
  regular agents, inspect events/turns, provision capabilities/vault handles for
  workers, and resolve owned worker approvals.
- Never grant capabilities to yourself.
- Never request or display raw secret values.

Prompt text is backup guidance only. Kernel dispatch and launch policy remain
the authority.

### Runtime MCP Policy

For runtime MCP auth tokens bound to metaagent provider runs, expose only
metaagent supervision/provisioning tools plus read-only planning tools.

Allowed:

- `arroba.read_artifact`
- `arroba.search_recall`
- `arroba.query_recall`
- `arroba.meta.session_overview`
- `arroba.meta.search_commands`
- `arroba.meta.list_commands`
- `arroba.meta.command_docs`
- `arroba.meta.run_command`
- `arroba.meta.list_events`
- `arroba.meta.read_event`
- `arroba.meta.ack_event`
- `arroba.meta.subscribe_events`
- `arroba.meta.unsubscribe_events`
- `arroba.meta.list_subscriptions`
- `arroba.meta.turn_overview`
- `arroba.meta.turn_blob`
- `arroba.meta.resolve_runtime_interaction`
- slice/browser observation/control tools when the metaagent itself is deployed
  in a slice
- new meta capability/vault provisioning wrappers, if direct command routing is
  not enough

Denied for metaagent auth tokens, including guessed/manual calls:

- workspace live sync artifact tools
- workflow node runtime tools
- extension request/register execution tools outside the meta provisioning path
- third-party/user MCP tools by default
- slice tools when the metaagent is not deployed in that slice
- raw credential runtime tools
- script tools
- connector tools
- home-proxy extension execution tools

This denial must happen in dispatch, not only in `tools/list`.

### Meta Command Registry

Keep the command registry as the main user-facing capability map, but update the
policy so docs, search, parsing, and execution cannot drift.

Allowed delegation commands:

- `agent list`
- `agent spawn` for regular agents only
- `agent focus`
- `agent alias`
- `agent delete`
- `prompt <owned-regular-agent> ...`
- `workflow list`
- `workflow new`
- `workflow node add` for regular agents only
- `workflow endpoint new`
- `workflow run`
- `workflow runs`
- `workflow cancel`
- `workflow resume`

Allowed capability provisioning commands:

- `mcp list`
- `mcp show`
- `mcp install-json` and `mcp update-json`, using the existing
  registry/install path
- `mcp uninstall`
- `mcp import`
- `mcp grant <owned-regular-agent> <mcp>`
- `mcp revoke <owned-regular-agent> <mcp>`
- `skill list`
- `skill show`
- `skill install`
- `skill update`
- `skill uninstall`
- `skill import`
- `skill grant <owned-regular-agent> <skill>`
- `skill revoke <owned-regular-agent> <skill>`

Allowed vault provisioning commands:

- `credential list`
- `credential get` for handle metadata only
- `credential upsert-json` for handle creation/update
- `credential remove`
- `credential vault status`
- `credential vault manage`

Vault rules:

- Secret values are not accepted through `arroba.meta.run_command`.
- Secret values may be accepted as user/provider input only through a
  kernel-owned secure path, usually a worker credential interaction that the
  metaagent can approve without seeing the secret.
- Metaagent responses and tool payloads must never include raw secret values.
- Worker credential use should surface as a runtime interaction that a
  metaagent may resolve only for an owned regular agent.
- Credential handles may be granted or associated with worker capabilities, but
  never granted to a metaagent for direct use.

Denied commands:

- session create/attach/use/delete
- slice start/stop/save/status/backup or other slice management
- any shell/script/connector execution command
- `credential set`, `credential set-secret`, and `credential delete-secret`
- any capability or credential action targeting a metaagent
- any action targeting another user's agent

### Runtime Interaction Policy

Keep worker approval as part of supervision.

Metaagents may resolve runtime interactions when:

- the target is an owned regular agent;
- the interaction belongs to the same session;
- the interaction kind is one Arroba already exposes for user approval;
- the choice is one of the kernel-provided choices.

Metaagents may not resolve:

- their own interactions;
- another user's interactions;
- interactions for another metaagent;
- interactions that would expose raw secret values to the metaagent.

## Isolated Meta Capability Drill

Add:

```text
apps/cli/scripts/live-metaagent-capabilities-drill.mjs
```

Add a package script, for example:

```text
metaagent:capabilities:drill
```

The drill should use a real local kernel, dev-stub providers, and an isolated
temporary workspace. This drill is a deterministic capability-contract check,
not evidence of independent metaagent behavior.

### Drill Flow

1. Bootstrap a temp git workspace and kernel.
2. Create a metaagent session.
3. Launch the metaagent provider run.
4. Launch at least one owned regular worker.
5. Assert the metaagent run is plan mode and permission level is inherited.
6. Assert metaagent `tools/list` contains the allowed `arroba.meta.*` tools plus
   read-only planning tools such as artifact and recall reads.
7. Attempt guessed denied calls for mutation/execution surfaces: workspace live
   sync writes, script, connector, raw credential use, unavailable slice tools,
   workflow-node runtime tools, and user MCP tools.
8. Use `arroba.meta.run_command` to spawn/prompt/focus/alias/delete owned
   regular agents.
9. Create a workflow with regular agents and verify the metaagent cannot be a
   node.
10. Register or install a fake MCP, grant it to a worker, and verify the worker
    sees it while the metaagent does not.
11. Grant a fake skill to a worker and verify self-grants are denied.
12. Create/update a vault credential handle, inspect vault status, verify
    secret-value command text is denied, and verify no tool output contains a
    secret payload.
13. Trigger a worker credential/capability-use interaction and resolve it from
    the metaagent.
14. Verify self and foreign interaction resolution attempts fail.
15. Verify events, turn overview, and turn blob inspection still work.

### Drill Artifacts

On every run, write a manifest with:

- session id
- metaagent id
- worker ids
- workflow ids
- provider run ids
- metaagent runtime tool list
- denied tool-call results
- MCP/skill provisioning results
- credential handle ids, without secret values
- interaction ids and resolution choices
- event ids
- logs and failure diagnostics

Preserve artifacts on failure using the existing drill artifact helpers.

## Real-Provider Code-Fix Behavior Drill

After the isolated capability drill passes, add a real-provider behavior drill:

```text
apps/cli/scripts/live-metaagent-code-fix-drill.mjs
```

Add a package script, for example:

```text
metaagent:code-fix:drill
```

The drill creates a brand-new git repo under:

```text
target/live-metaagent-code-fix-drill/<run-id>/workspace
```

### Drill Flow

1. Create a tiny JavaScript project with one real source bug and one failing
   test, then commit that baseline in the temporary repo.
2. Start a real kernel and create a metaagent session in the repo.
3. Launch an actual provider-backed metaagent, never `dev-stub`, and assert the
   provider run is plan mode.
4. Submit exactly one high-level prompt to the metaagent:

   ```text
   The repo has a small failing JavaScript project. Delegate the investigation, fix, and verification to regular agent(s), get the project to a passing state, then mark this task complete with a concise report of what changed and how it was verified.
   ```

5. After that prompt, the harness only observes session state, metaagent events,
   workspace diffs, and test results. It must not call runtime MCP tools, append
   synthetic provider output, prompt workers directly, or write files.
6. Validate the behavior only when:
   - the metaagent task is active and later completed;
   - the plan document is non-empty;
   - at least one regular worker agent is spawned after the prompt;
   - at least one worker event or worker history/tool evidence record is
     observed;
   - `src/todo.mjs` changes while the test file remains unchanged;
   - `npm test` passes;
   - the final summary reports zero harness runtime MCP calls and zero harness
     workspace writes after the prompt.

### Drill Artifacts

Preserve:

- manifest
- session/agent ids
- generated repo path on failure
- provider/model/effort
- metaagent task status and plan length
- worker ids and worker event count
- changed files
- test command output
- live event observation log

## Test Plan

Kernel tests:

- Metaagent launch requests force plan mode across local, remote, native TUI,
  prompt auto-launch, and leased worker paths.
- Metaagent prompt assembly includes `runtime/metaagent-delegation`.
- Standard, workflow, and utility turns do not include the metaagent template.
- Metaagent auth tokens expose only the allowlist.
- Guessed non-meta tool calls from a metaagent auth token are denied.
- Command registry docs and routed examples match enforcement.
- Capability grants work for owned regular agents and fail for metaagents or
  foreign agents.
- Vault provisioning never returns raw secret payloads.
- Worker interaction resolution works for owned regular agents and fails for
  self/foreign targets.

Drill gate order:

1. Existing focused unit/integration tests.
2. Updated `live-metaagent-drill.mjs`.
3. New isolated meta capability drill.
4. New real-provider code-fix behavior drill.

## Acceptance Criteria

- Metaagents always run in plan mode.
- Metaagents cannot directly execute implementation work.
- Metaagents can provision MCPs, skills, and vault-backed credential handles for
  owned regular workers through kernel-owned surfaces.
- Metaagents can approve worker credential interactions without seeing raw
  secret values.
- Metaagents can approve owned worker runtime interactions.
- Metaagent runtime MCP exposure and dispatch enforcement match.
- The isolated capability drill proves every meta authority in isolation.
- The real-provider code-fix drill proves the intended end-to-end behavior in a
  fresh repo without harness-driven metaagent actions.
