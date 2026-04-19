# Live Drill Policy

These scripts exercise Arroba against real provider sessions. Keep them deterministic, cheap, and explicit so future agents can rerun them safely.

## Provider Models

- Default live-drill model: `gpt-5.2`.
- Use older Codex-capable models such as `gpt-5.2` or `gpt-5.3` for routine drills.
- Use reasoning effort `low` unless the drill is specifically validating reasoning-heavy behavior.
- For `opencode`, use OpenAI models rather than OpenCode's provider-default or `zen` model family.
- If an OpenCode drill receives an unqualified model such as `gpt-5.2`, map it to `openai/gpt-5.2`.
- For `codex`, do not rely on drill-specific bare-model fallback when the exact model matters. Always pass an explicit Codex override such as `--provider-model codex=gpt-5.2` or `--provider-model codex=gpt-5.3`. Several workflow drills map bare `gpt-5.2`/`gpt-5.3` to `gpt-5.2-codex`/`gpt-5.3-codex`; ChatGPT-backed Codex accounts can reject those `*-codex` model ids with HTTP 400, leaving the drill looking stuck with only prompt echo.
- Prefer an explicit override when debugging provider-specific behavior:

```bash
node apps/cli/scripts/live-managed-io-drill.mjs --provider opencode --provider-model opencode=openai/gpt-5.2
node apps/cli/scripts/live-mcp-skill-drill.mjs --providers opencode,codex --provider-model opencode=openai/gpt-5.2 --provider-model codex=gpt-5.2
node apps/cli/scripts/live-runtime-mcp-reattach-drill.mjs --providers opencode,codex --provider-model opencode=openai/gpt-5.2 --provider-model codex=gpt-5.2
node apps/cli/scripts/live-remote-managed-io-drill.mjs --providers opencode,codex --provider-model opencode=openai/gpt-5.3-codex --full
```

## Wrapper Scripts

Remote wrapper drills must forward `--provider-model PROVIDER=MODEL` to their child drill scripts. This keeps local and remote provider sessions on the same model policy.

## Cleanup

Live drills should own their daemon/session/port/artifact lifecycle and clean up generated files on success. If a drill supports `--keep-artifacts-on-failure`, only leave artifacts behind on failure for debugging.

## Shell Scriptability Drill

`live-shell-scriptability-drill.mjs` validates `arroba-shell` against an isolated local kernel with a temporary `HOME`, workspace, daemon socket, ports, and history directory. It does not launch real provider model turns. It creates sessions and dev-stub agents, mutates config, installs/grants/revokes/uninstalls a deterministic MCP and skill, exercises workflow graph/config/watchdog/queue commands, runs `stop`, and verifies `arroba-shell run` seed variables plus `--continue-on-error` line diagnostics.

```bash
pnpm --filter @arroba/cli run shell:drill
```

## Managed I/O Identity

Managed-I/O drills coordinate only while the provider run remains in the same repo/branch/head identity captured by the kernel. If a drill or concurrent developer action changes that identity mid-run, `workspace_identity_changed` is a valid failure mode. Restart the drill from a stable workspace identity rather than treating that rejection as a file-edit collision.

## MCP/Skill Drills

`live-mcp-skill-drill.mjs` is local-only for now. It installs a real Playwright MCP into an isolated Arroba registry, optionally installs GitHub MCP when `--include-github-mcp` is set and `GITHUB_PERSONAL_ACCESS_TOKEN` or `GITHUB_TOKEN` is present, installs a deterministic local drill skill, attempts to install a public web skill repo, and verifies per-agent grants plus same-turn skill requests.

Use `--require-web-skill` when the network/web-skill install itself is the thing being validated. Without it, public skill clone failures are reported but do not fail the drill, so local registry/runtime coverage can still run offline.

Use `--live-mcp-use` to also require provider-native Playwright tool calls. The drill covers both user-triggered MCP grants, where `/mcp grant` causes Arroba to relaunch the idle provider conversation, and agent-triggered `request_capability`, where Arroba reloads after the current turn and sends an automatic continuation prompt before requiring a Playwright/browser tool call and managed-I/O marker write.

`live-runtime-mcp-reattach-drill.mjs` is the local regression drill for stale provider servers and CLI rejoin. It warms provider catalog endpoints before launching managed-I/O agents, forcing Codex/OpenCode through the path where a provider server may already be alive without run-specific Arroba MCP config. It then detaches the CLI, reattaches to the same session, submits another prompt to the same agents, and fails unless each agent completes `list_capabilities` plus `read_artifact` runtime MCP calls and writes before/after marker files through Arroba managed I/O.

`live-remote-mcp-drill.mjs` is the remote MCP v1 drill. It launches isolated relay/home/worker daemons with different `HOME` roots so home and worker Arroba user-global MCP registries can diverge on one machine. It verifies worker-missing MCPs, worker global definition mismatches, project-local worker override, missing stdio commands, and missing worker env vars. V1 remote MCPs must already be installed on the worker; the drill does not remotely install MCPs. Pass `--live-mcp-use` to also require a provider-native remote Playwright/browser MCP tool call on the worker and a marker write through Arroba managed I/O. Because the drill isolates `HOME` for Arroba registries, it preserves provider auth/config/cache via `CODEX_HOME`, `OPENCODE_CONFIG_DIR`, and `XDG_*` provider environment variables.

`live-workflow-runtime-drill.mjs --scenario mcp-echo-workflow` validates workflow-node MCP grants. It installs a deterministic stdio MCP named `workflow_echo`, grants it to the workflow agent before provider launch, requires a provider-native MCP tool call, writes an exact marker through Arroba managed I/O, and submits final workflow output through `validate_and_submit_workflow_run_output`. The scenario is single-provider by design; run it separately for Codex and OpenCode. The remote wrapper installs the same deterministic MCP in the worker registry before launching the remote workflow drill, matching the v1 remote MCP rule that worker kernels must already have the required MCP definition. For Codex, pass `--provider-model codex=gpt-5.2`; the workflow drill's bare `--model gpt-5.2` fallback maps to `gpt-5.2-codex`, which can be rejected by ChatGPT-backed Codex accounts.

## Before Commit

Run syntax checks for edited scripts before committing:

```bash
node --check apps/cli/scripts/<script>.mjs
```

## Kernel Reconnect Drill

Use this after touching CLI/kernel transport recovery:

```bash
pnpm --filter @arroba/cli run build
node apps/cli/scripts/live-kernel-reconnect-drill.mjs
```

It kills the event subscription lane while a control request is pending and fails unless the control request completes and the event lane resubscribes from the last event id.
