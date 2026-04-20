# Live Drill Gate

This is the post-M4.5 gate before starting new feature work. The M4.5 ownership points are closed; confidence now comes from live behavior across local providers, workflows, and relay-backed remote machines.

Script-level policy lives in `apps/cli/scripts/README.md`: live drills default to older Codex-capable models such as `gpt-5.2`, low effort, and OpenCode runs use OpenAI Codex-family models instead of OpenCode provider-default or `zen` models.

Codex model override rule: when a drill includes Codex, pass an explicit provider override such as `--provider-model codex=gpt-5.2` unless the drill is intentionally validating a different Codex model. Do not assume bare `--model gpt-5.2` is enough. Some drill helpers map bare `gpt-5.2`/`gpt-5.3` to `gpt-5.2-codex`/`gpt-5.3-codex`; ChatGPT-backed Codex accounts can reject those `*-codex` ids with HTTP 400, which presents as a stuck drill with prompt echo and no provider output.

Terminology:

- **Freeform multi-agent mode** means a normal multi-agent session with no workflow scheduler. Agents can be focused, prompted, and observed independently, but no workflow graph owns turn progression.
- **Workflow mode** means a daemon-owned workflow graph schedules node turns, validates outputs, and advances runs.
- **Remote mode** means one or more CLIs or agents connect through relay to a home daemon or to a remote machine accepting leases.

## Required Drill Matrix

1. Freeform local multi-agent:
   - Start one session.
   - Attach at least one local CLI.
   - Spawn at least one OpenCode-backed agent and one Codex-backed agent.
   - Submit targeted prompts to both agents.
   - Verify each agent completes without blocking the other, focus/state projections remain correct, and provider output is visible from the attached CLI.

2. Local workflow drills:
   - Run the existing workflow drill catalog:
     - `simple-chain`
     - `validated-increment-chain`
     - `console-increment-chain`
     - `final-run-output-chain`
     - `cyclic-final-run-output-chain`
     - `cyclic-budgeted-final-run-output-chain`
     - `cyclic-final-run-with-intermediate-output-chain`
     - `conditional-branch-subset`
     - `immediate-release-downstream`
   - Verify run status, node summaries, final output, intermediate output, console entries, provider cleanup, and session projection refresh.

3. Remote freeform multi-agent through relay:
   - Start relay, home daemon, and worker daemon.
   - Attach one local CLI directly and one remote CLI through relay.
   - Spawn local and remote agents through the home session.
   - Submit targeted prompts from both CLIs, including prompts to worker-machine agents.
   - Restart relay during the session and verify transport close/resume plus continued prompt completion.

4. Remote workflow through relay:
   - Start relay, home daemon, and worker daemon.
   - Run the workflow drill catalog with agents leased on the remote worker machine.
   - Verify workflow completion, runtime-tool handling, relay recovery, and final projection state from both local and relayed clients.

## Existing Automation

- Local freeform multi-agent: `node apps/cli/scripts/live-freeform-multi-agent-drill.mjs --providers opencode,codex`
- Local managed I/O: `node apps/cli/scripts/live-managed-io-drill.mjs --providers opencode,codex` or `pnpm --filter @arroba/cli run managed-io:drill`
- Local runtime MCP reattach: `node apps/cli/scripts/live-runtime-mcp-reattach-drill.mjs --providers opencode,codex` or `pnpm --filter @arroba/cli run runtime-mcp-reattach:drill`
- Local workflow: `node apps/cli/scripts/live-workflow-runtime-drill.mjs --spawn-daemon --scenario <scenario> --providers opencode,codex`
  - Add `--no-early-pass` when validating the full `immediate-release-downstream` completion path instead of only the immediate release point.
- Local shell/scriptability: `pnpm --filter @arroba/cli run shell:drill`. The drill launches an isolated local kernel with a temporary home/workspace/history/ports, runs `arroba-shell run <file>` scripts, covers shell-local state, session/agent/config/MCP/skill/workflow/provider-process/stop command families without launching real provider model turns, and verifies seeded variables, `source <file>` loading, and `--continue-on-error` line diagnostics.
- Local restart/recovery: `pnpm --filter @arroba/cli run local-restart:drill`. The drill rebuilds the local kernel, launches an isolated daemon, creates a session, spawned agent, MCP grant, skill grant, provider profile, completed transcript marker, and in-flight workflow run, restarts the kernel process, then verifies restored state, cleared stale runtime work, workflow restart interruption events, transcript paging, and history search.
- Local CLI restart/reconnect: `pnpm --filter @arroba/cli run cli-restart:drill`. The drill launches the real CLI under a PTY with automation enabled, verifies it renders a connected session, stops the kernel process, verifies the CLI shows the disconnected state, restarts the kernel, and verifies the CLI reattaches to the restored session and clears the disconnected state.
- Local Git observation: `pnpm --filter @arroba/cli run git-observation:drill`. The drill launches an isolated local kernel and dev-stub provider in a temporary Git repo, waits until the prompt has been dispatched, commits a changed file during the agent turn, completes the prompt, then verifies operational history contains a `git_commit_detected` event searchable by commit subject/path/provider/model/prompt attribution.
- Remote Git observation: `pnpm --filter @arroba/cli run remote-git-observation:drill`. The drill launches isolated relay/home/worker kernels, spawns a remote dev-stub agent in a worker Git repo, waits for the remote prompt to dispatch, commits a changed file on the worker worktree during the remote turn, completes through home, then verifies home operational history contains a `git_commit_detected` event with home agent/prompt ids and worker machine/repo/worktree metadata.
- Shell prompt submission: covered by shared executor tests for no-wait prompt id output, `context` busy-state refresh, and `--wait --show-summary` prompt-id blob rendering. Provider-backed prompt completion remains covered by freeform/provider live drills.
- Embedded workflow shell: `pnpm --filter @arroba/cli run embedded-shell:drill`. The drill launches the real CLI under a PTY, controls it through `--automation-socket`, runs `source <file>` in the workflow-pane shell, and asserts structured CLI snapshots for workflow screen activation, selected workflow, graph counts, and shell transcript updates.
- Local MCP/skill management: `node apps/cli/scripts/live-mcp-skill-drill.mjs --providers opencode,codex` or `pnpm --filter @arroba/cli run mcp-skill:drill`. The drill installs real MCPs, installs at least one public GitHub skill into `.arroba/skills` when the network is available, verifies per-agent grants, verifies same-turn skill request bodies, and cleans its isolated workspace/daemon/session artifacts on success. Pass `--live-mcp-use` to require actual provider-native Playwright tool calls for both user-triggered MCP grant activation and agent-triggered `request_capability` activation with automatic continuation.
- Remote freeform multi-agent: `node apps/cli/scripts/live-remote-multi-agent-relay-drill.mjs --providers opencode,codex`
- Remote managed I/O smoke: `node apps/cli/scripts/live-remote-managed-io-drill.mjs --providers opencode,codex`
- Remote managed I/O full: `node apps/cli/scripts/live-remote-managed-io-drill.mjs --providers opencode,codex --provider-model opencode=openai/gpt-5.3-codex --full` or `pnpm --filter @arroba/cli run managed-io:remote-drill`
- Remote skill management: `node apps/cli/scripts/live-remote-skill-drill.mjs --provider opencode --model openai/gpt-5.2 --effort low` and `node apps/cli/scripts/live-remote-skill-drill.mjs --provider codex --model gpt-5.2 --effort low`. The drill verifies local-only grants do not materialize on the worker, local-to-remote move synchronizes existing skill grants, remote-first grant-time materialization works, prompt-time synchronization repairs a removed worker copy, same-turn remote `request_capability` materializes the skill, worker-local `materialized_root` injection works, and providers use a synchronized skill asset through managed I/O.
- Remote MCP management: `node apps/cli/scripts/live-remote-mcp-drill.mjs --provider opencode --model openai/gpt-5.2 --effort low` and `node apps/cli/scripts/live-remote-mcp-drill.mjs --provider codex --model gpt-5.2 --effort low`. V1 behavior requires the worker to already have the matching MCP definition installed in its project/user Arroba registry, and missing/mismatched/missing-command/missing-env cases fail fast before remote provider launch/prompt. Pass `--live-mcp-use` to additionally require a provider-native remote Playwright/browser MCP tool call on the worker and a managed-I/O marker write.
- Remote restart/recovery: `pnpm --filter @arroba/cli run remote-restart:drill`. The drill launches isolated relay, home, and worker kernels, spawns a remote dev-stub agent, verifies baseline prompting, restarts home, restarts worker, restarts both, and requires the home kernel to restore the remote agent and refresh stale worker leases. Use `--keep-artifacts-on-failure` to preserve the isolated logs. After restarts, validate through the home kernel's remote-machine/kernel APIs rather than direct relay alias probes; the product path is what must recover.
- Remote workflow: `node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario <scenario> --providers opencode,codex`
- Lower-level relay runtime: `node apps/cli/scripts/live-relay-runtime-drill.mjs`
- Lower-level remote machine runtime: `node apps/cli/scripts/live-remote-machine-runtime-drill.mjs`

## Exit Criteria

- Every required drill either passes or has a filed blocker with repro command, observed failure, and owner.
- `cargo check --manifest-path apps/kernel/Cargo.toml` remains clean.
- Daemon lib, runtime integration, websocket integration, relay-client, and bin tests remain green after any drill fixes.
- Docs reflect the final drill results before new tasks begin.

Workspace identity note: managed I/O captures repo/branch/head identity for each provider run. If a drill changes `HEAD`, branch, or repo identity while another managed-I/O run is active, the kernel may reject the next tool call with `workspace_identity_changed`. That is expected protection, not a merge conflict; rerun the drill from a stable workspace identity.

## Current Results

- Freeform local multi-agent: **pass** with `opencode,codex` after the router bootstrap lock fix.
- Local managed I/O: **pass** with `opencode,codex`; agents read `seed.txt`, create provider-specific output files through `arroba.write_artifact`, edit/apply-patch/move/delete through managed tools, exercise opaque write/read/move/delete through base64 whole-file operations, fail if direct/native write attempts create forbidden files, serialize same-area agent collisions to one winning write, rebase stale non-overlapping external changes, and reject stale overlapping external changes. The drill owns and tears down its daemon, session, isolated workspace, session history, and transient CLI module cache.
- Local runtime MCP reattach: **pass** with `opencode,codex`. On 2026-04-18, `node apps/cli/scripts/live-runtime-mcp-reattach-drill.mjs --providers opencode,codex --provider-model opencode=openai/gpt-5.2 --provider-model codex=gpt-5.2 --timeout-ms 360000 --poll-ms 1000` passed in ~36s. The drill warms provider catalog endpoints before managed-I/O launch, detaches and reattaches the CLI, and verifies each provider uses Arroba runtime MCP `list_capabilities` plus `read_artifact` both before and after reattach.
- Local shell/scriptability: **pass**. On 2026-04-20, `pnpm --filter @arroba/cli run shell:drill` passed against an isolated local kernel. The drill created a standalone shell-attached session, spawned dev-stub agents, exercised config/MCP/skill/workflow graph/workflow config/watchdog/queue/provider-process/stop command families, and verified script seed variables, nested `source <file>` loading, and `--continue-on-error` handling for TUI-only, no-active-prompt, and missing-relay failures. Shared executor tests also cover no-wait `prompt` id output, `context` busy-state refresh, and wait/show prompt-id blob rendering.
- Embedded workflow shell: **automation pass, runtime provider-stub-limited**. On 2026-04-20, `pnpm --filter @arroba/cli run embedded-shell:drill` passed against an isolated kernel. The drill launched the real CLI under a PTY, controlled it through `--automation-socket`, switched to the workflow screen, ran `source embedded-flow.arroba` in the embedded shell, and verified the selected workflow updated to `nodes=2`, `edges=1`, `endpoints=1` with shell transcript entries preserved. Running provider-backed workflow turns is still covered by the workflow runtime drill suite; the embedded shell drill intentionally validates CLI integration and graph/source state without model-dependent output.
- Local restart/recovery: **pass**. On 2026-04-20, `pnpm --filter @arroba/cli run local-restart:drill` passed against a freshly rebuilt isolated local kernel. The drill created durable session state, a spawned dev-stub agent, MCP and skill grants, a provider profile, a completed transcript marker, and an in-flight workflow turn; after process restart it verified the session and agent restored, grants and workflow definitions restored, stale active provider/prompt state was cleared, the workflow run was marked `Stopped` with the kernel-restart failure event, and both transcript paging and operational history search retained the prompt marker.
- Local CLI restart/reconnect: **pass**. On 2026-04-20, `pnpm --filter @arroba/cli run cli-restart:drill` passed against a real PTY-hosted CLI and isolated kernel. The drill verified the CLI shows `Lost connection to the Arroba kernel.` after the daemon stops, then after daemon restart it reattaches to the restored session, resubscribes to kernel events with a fresh attachment, clears the disconnected state, and shows the restored agent.
- Local Git observation: **pass**. On 2026-04-20, `pnpm --filter @arroba/cli run git-observation:drill` passed against an isolated kernel, dev-stub agent, and temporary Git repo. The drill committed `feature.txt` during a dispatched local agent turn, completed the prompt, and verified operational history recorded `git_commit_detected` with the commit SHA, changed path, agent id, prompt id, provider/model metadata, and searchable commit subject/path text.
- Remote Git observation: **pending current slice**. Required command: `pnpm --filter @arroba/cli run remote-git-observation:drill`.
- Remote restart/recovery: **pass**. On 2026-04-20, `pnpm --filter @arroba/cli run remote-restart:drill` passed against isolated relay/home/worker kernels. The drill verified a remote dev-stub agent can prompt before restart, after home restart with worker alive, after worker restart with home alive and stale lease refresh, and after both kernels are manually restarted. The observed worker restart and both-restart prompts used new leased agent ids, confirming home rebound the durable remote agent to fresh worker leases.
- Remote managed I/O: **full pass with `opencode,codex`** after leased runs require managed I/O. The full drill covers managed text read/write/edit/apply-patch/move/delete, opaque whole-file write/read/move/delete, direct-write blocking, same-area collision serialization, stale non-overlap external-change rebase, and stale overlap external-change rejection. Use OpenAI/Codex-family models for OpenCode, for example `--provider-model opencode=openai/gpt-5.3-codex`.
- Local workflow catalog: **pass** with spawned local daemon and `opencode,codex`.
  - `simple-chain`
  - `validated-increment-chain`
  - `console-increment-chain`
  - `final-run-output-chain`
  - `cyclic-final-run-output-chain`
  - `cyclic-budgeted-final-run-output-chain`
  - `cyclic-final-run-with-intermediate-output-chain`
  - `conditional-branch-subset`: router node emits structured `workflow_handoffs`, only the selected even-value branch runs, and the excluded branch is not invoked.
  - `immediate-release-downstream`: producer submits validated intermediate output, downstream starts before producer completion, consumes `{"value":1842}`, and submits final workflow output `{"value":1843}`. Full completion is verified with both provider orders using `--no-early-pass`.
- Remote freeform relay: **pass** with `opencode,codex`, direct local client plus relayed client, local sidecar agents plus worker-machine leased agents, relay restart, transport close/resume, and post-reconnect prompt completion.
- Remote workflow relay: **pass** with `opencode,codex`, relay, home daemon, worker daemon, remote worker-machine leases, forwarded workflow runtime tools, cyclic workflow progression, intermediate output, final workflow output, and clean final projections.
- Workflow prompt-injection invariant: **pass**. Workflow prompt construction is scheduler-owned and shared by local dispatch, remote dispatch, retry/replay paths, and tests; the renderer injects turn index, last-turn guidance only on the last allowed node turn, final-output tool guidance, runtime-tool instructions, handoff payloads, edge contracts, and control mailbox content.
- Local MCP/skill management: **strict local pass**. On 2026-04-18, `node apps/cli/scripts/live-mcp-skill-drill.mjs --provider codex --require-web-skill --live-mcp-use --timeout-ms 300000 --poll-ms 1000` passed in ~40s, and `node apps/cli/scripts/live-mcp-skill-drill.mjs --provider opencode --provider-model opencode=openai/gpt-5.2 --require-web-skill --live-mcp-use --timeout-ms 300000 --poll-ms 1000` passed in ~35s. Both installed real Playwright MCP, installed public `vercel-labs/agent-skills` skill `deploy-to-vercel`, installed the deterministic drill skill, verified pre-granted skill use, verified same-turn `request_capability` returns a skill body followed immediately, restarted the idle provider process for next-launch MCP rendering, verified a provider-native Playwright/browser tool call, and wrote the Playwright marker through Arroba managed I/O. GitHub MCP remains optional and is skipped unless `GITHUB_PERSONAL_ACCESS_TOKEN` or `GITHUB_TOKEN` is present with `--include-github-mcp`.
- Remote skill management: **pass**. On 2026-04-18, `node apps/cli/scripts/live-remote-skill-drill.mjs --provider opencode --model openai/gpt-5.2 --effort low --timeout-ms 300000 --poll-ms 1000` and `node apps/cli/scripts/live-remote-skill-drill.mjs --provider codex --model gpt-5.2 --effort low --timeout-ms 300000 --poll-ms 1000` passed. Both created isolated relay/home/worker daemons, installed an Arroba-owned skill with an asset, verified local-only grants do not materialize on the worker, moved a pre-granted local agent to remote and verified grant synchronization, verified remote-first grant-time materialization under `.arroba/remote/skills/...`, deleted the worker materialization and verified prompt-time repair, verified same-turn remote `request_capability` materialization, submitted live remote prompts, and verified provider output file `outputs/remote-skill-provider.txt` contained the synchronized asset token plus `REMOTE_SKILL_DRILL_OK`.
- Remote MCP management: **strict pass**. On 2026-04-18, after production HTTPS/chunked MCP proxy support landed, `node apps/cli/scripts/live-remote-mcp-drill.mjs --provider opencode --model openai/gpt-5.2 --effort low --timeout-ms 300000 --live-mcp-use` passed in ~31s and `node apps/cli/scripts/live-remote-mcp-drill.mjs --provider codex --model gpt-5.2 --effort low --timeout-ms 300000 --live-mcp-use` passed in ~79s. Both created isolated relay/home/worker daemons with separate `HOME` roots, verified worker-missing MCP fails fast, verified worker global definition mismatch fails fast, verified worker project-local matching MCP overrides a mismatched global MCP, verified missing worker stdio command fails fast, verified missing worker env var fails fast, verified a provider-native remote Playwright/browser MCP tool call on the worker, and wrote `outputs/remote-playwright-mcp.txt` with `M7_REMOTE_PLAYWRIGHT_MCP_OK` through Arroba managed I/O. OpenCode used `playwright_browser_snapshot`; Codex used `browser_tabs`.
- Workflow MCP grants: **pass**. On 2026-04-19, `node apps/cli/scripts/live-workflow-runtime-drill.mjs --spawn-daemon --scenario mcp-echo-workflow --providers opencode --provider-model opencode=openai/gpt-5.2 --poll-limit 180 --poll-interval-ms 1000` passed in ~14s, and `node apps/cli/scripts/live-workflow-runtime-drill.mjs --spawn-daemon --scenario mcp-echo-workflow --providers codex --provider-model codex=gpt-5.2 --poll-limit 180 --poll-interval-ms 1000` passed in ~14s. Both installed deterministic MCP `workflow_echo`, granted it to the workflow agent, completed a provider-native echo MCP call, wrote an exact managed-I/O marker, and submitted final workflow output `{"echo":"ECHO:M7_WORKFLOW_ECHO_OK"}`. Remote OpenCode passed with `node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario mcp-echo-workflow --provider opencode --model gpt-5.2 --provider-model opencode=openai/gpt-5.2 --poll-limit 240 --poll-interval-ms 1000` in ~72s. Remote Codex passed with `node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario mcp-echo-workflow --provider codex --model gpt-5.2 --provider-model codex=gpt-5.2 --poll-limit 240 --poll-interval-ms 1000` in ~116s. The earlier remote Codex failure was a drill invocation error: without the explicit provider override, the workflow drill mapped bare `gpt-5.2` to `gpt-5.2-codex`, and Codex logs showed that model is rejected by ChatGPT-backed Codex accounts.

Point-2 fixes proven by the local workflow catalog:

- Workflow endpoint invocation now enqueues the entry dispatches returned by owned invoke admission instead of dropping them.
- Owned workflow prompt rendering now includes workflow prompt, node instructions, outgoing edge contracts, ack/validation/tool instructions, and the final fenced JSON contract for entry and downstream turns.
- Terminal pumping drains all starting/running provider runs in the session, not only the focused active provider.
- Structured provider lifecycle handling ignores stale completion/idle events and requires a submitted prompt before treating idle as completion.
- Workflow console writes no longer deadlock by taking a session-store read while constructing arguments for a write-locked mutation.
- Resolved missing-output/output-validation failures are cleared when a later successful turn for the same workflow node produces output or submits final output.
- Node max-turn budget accounting counts completed turns with completion payloads, so recovered missing-output attempts do not consume the production success budget.
- Intermediate-output runtime tools now enqueue the downstream prompt dispatches they prepare; the previous path created downstream node runs but dropped the async provider submission work.
- Final workflow-output submissions no longer generate downstream handoff validation failures while completion is still committing pending turn outputs.
- The budgeted cyclic final-output scenario now checks the exact final output payload, so a stale pre-ack provider output cannot accidentally satisfy a later final workflow turn.
- Conditional branching drill: **pass**. A router node with two outgoing edges emitted a structured `workflow_handoffs` payload selecting only the even branch; the selected branch completed with `{"bucket":"even","values":[2]}` and the excluded branch was not invoked.
- Immediate-release drill: **pass for release semantics**. A valid intermediate output `{"value":1842}` created and prompted the downstream node while the producer node was still running, proving asynchronous handoff release before producer turn completion. Provider settlement after the intermediate tool remains a separate follow-up hardening item.

Point-3 evidence:

```
node apps/cli/scripts/live-remote-multi-agent-relay-drill.mjs --providers opencode,codex --model gpt-5.2 --timeout-ms 240000 --poll-ms 1000
```

Observed result: both providers were visible on the worker machine, both providers had local sidecar agents and remote leased worker agents, both local and relayed clients observed four assistant completions, the relayed client observed `transport_closed` and `transport_resumed`, and both remote worker agents completed prompts after relay restart.

Point-4 evidence:

```
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario validated-increment-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario simple-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario console-increment-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario final-run-output-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario cyclic-final-run-output-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario cyclic-budgeted-final-run-output-chain --providers opencode,codex --model gpt-5.2 --poll-limit 600 --poll-interval-ms 250
node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs --scenario cyclic-final-run-with-intermediate-output-chain --providers opencode,codex --model gpt-5.2 --poll-limit 700 --poll-interval-ms 250
```

Observed result: the remote workflow catalog completed through the relay with worker-machine leased agents. The closing cyclic intermediate-output run completed with validated intermediate output and final workflow output from `workflow-node-run-2`, final output `{"value":1843}`, no failure events, and status `Completed`.
