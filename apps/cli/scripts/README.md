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
node apps/cli/scripts/live-workspace-live-sync-drill.mjs --provider codex --provider-model codex=gpt-5.2
node apps/cli/scripts/live-mcp-skill-drill.mjs --providers opencode,codex --provider-model opencode=openai/gpt-5.2 --provider-model codex=gpt-5.2
node apps/cli/scripts/live-runtime-mcp-reattach-drill.mjs --providers opencode,codex --provider-model opencode=openai/gpt-5.2 --provider-model codex=gpt-5.2
node apps/cli/scripts/live-remote-workspace-live-sync-drill.mjs --provider codex --provider-model codex=gpt-5.2 --full
```

Add OpenCode to the workspace live sync drills only when OpenCode auth is healthy; current failures with `Token refresh failed: 401` happen before workspace live sync behavior is exercised.

Workspace Live Sync validation aliases:

```bash
pnpm --filter @arroba/cli run workspace-live-sync:managed-drill
pnpm --filter @arroba/cli run workspace-live-sync:tracked-drill
pnpm --filter @arroba/cli run workspace-live-sync:permission-drill
pnpm --filter @arroba/cli run workspace-live-sync:remote-managed-drill
pnpm --filter @arroba/cli run workspace-live-sync:remote-tracked-drill
pnpm --filter @arroba/cli run workspace-live-sync:remote-permission-drill
```

## Wrapper Scripts

Remote wrapper drills must forward `--provider-model PROVIDER=MODEL` to their child drill scripts. This keeps local and remote provider sessions on the same model policy.

## Cleanup

Live drills should own their daemon/session/port/artifact lifecycle and clean up generated files on success. If a drill supports `--keep-artifacts-on-failure`, only leave artifacts behind on failure for debugging.

## Shell Scriptability Drill

`live-shell-scriptability-drill.mjs` validates `arroba-shell` against an isolated local kernel with a temporary `HOME`, workspace, daemon socket, ports, and history directory. It does not launch real provider model turns. It creates sessions and dev-stub agents, mutates config, installs/grants/revokes/uninstalls a deterministic MCP and skill, exercises workflow graph/config/watchdog/queue commands, runs `stop`, and verifies `arroba-shell run` seed variables, `source <file>` loading, and `--continue-on-error` line diagnostics.

The shell `prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]` command is covered by shared executor tests. Live provider response quality and completion timing remain covered by the freeform/provider drills; shell scriptability drills should not depend on model-specific wording unless the drill is explicitly provider-backed.

The embedded workflow-pane shell uses the same script runner. Manual TUI drills should launch the default CLI agent with a non-emitting dev-stub model, then spawn any workflow agents explicitly from the shell script. Do not use a workflow-outputting dev-stub model as the initial CLI model unless the drill is intentionally testing startup output noise.

`live-embedded-shell-automation-drill.mjs` launches the real CLI under a PTY but drives it through `--automation-socket` instead of raw keystrokes. The automation API returns structured snapshots for the current screen, selected workflow, workflow graph counts, workflow runs, shell context, and shell transcript, so embedded-shell drills can assert CLI state without parsing ANSI terminal output. Keep the automation socket path short; Unix socket path limits are strict on macOS.

```bash
pnpm --filter @arroba/cli run shell:drill
pnpm --filter @arroba/cli run embedded-shell:drill
```

## Workspace live sync Identity

Workspace live sync drills coordinate only while the provider run remains in the same repo/branch/head identity captured by the kernel. If a drill or concurrent developer action changes that identity mid-run, `workspace_identity_changed` is a valid failure mode. Restart the drill from a stable workspace identity rather than treating that rejection as a file-edit collision.

## Multi-User Workflow Drill

Use this after touching session membership, relay caller identity, per-user projection/redaction, or workflow graph authorization:

```bash
pnpm --filter @arroba/cli run multi-user-workflow:drill
```

It launches a scoped-token relay plus a local kernel, connects three relay clients with different `user_id`s, joins them into one session through an invite, and verifies the live transport path for per-user agent visibility, workflow node ownership, cross-owner edge creation, unrelated edge-removal denial, stale workflow revision rejection, endpoint-owner invocation denial, incident-edge removal by node owner, and private node-instruction redaction. It uses `dev-stub` agents only, so it does not spend provider turns.

## Hosted Cloud Relay Drill

Use this after touching Arroba Cloud device login, cloud relay pairing, hosted relay token issuance, or CLI/kernel relay setup:

```bash
node apps/cli/scripts/live-hosted-cloud-relay-drill.mjs
```

By default the drill targets `https://arroba-cloud-staging.osc-fr1.scalingo.io`, whose hosted relay URL is the Caddy-fronted `wss://195.201.123.115.sslip.io` endpoint. Set `ARROBA_CLOUD_HOSTED_API_URL` to use another cloud API. The single-user path validates local CLI to local kernel login, cloud client pairing, machine pairing, machine relay connect, client relay-token issuance, and remote client session create/list through the hosted relay. Local/self-hosted relay drills intentionally keep using `ws://127.0.0.1:<port>`.

For non-interactive staging drills, set `ARROBA_CLOUD_DEV_AUTH_SECRET` to the matching staging secret. The cloud API must have its guarded dev device approval endpoint enabled. This still starts device login and polls through the kernel; only the browser/Auth0 approval step is replaced by a synthetic verified user.

Set `ARROBA_CLOUD_HOSTED_MULTI_USER=1` to add the hosted multi-user path. With the dev secret, the drill creates distinct synthetic owner, peer, and third users, accepts a cloud session invite, joins the kernel session through the hosted relay, and verifies multi-user workflow ownership and redaction over the remote relay transport.

Set `ARROBA_CLOUD_HOSTED_REMOTE_CLI=1` to launch a real CLI over SSH on the configured remote host and verify it creates a session through the hosted relay. The default remote target is `root@195.201.123.115` with key `~/.ssh/arroba_hetzner_staging` and repo `/opt/arroba-cli-drill`; override them with `ARROBA_CLOUD_HOSTED_REMOTE_CLI_HOST`, `ARROBA_CLOUD_HOSTED_REMOTE_CLI_KEY`, and `ARROBA_CLOUD_HOSTED_REMOTE_CLI_REPO`. The remote host must have Node, Bun, and a built Arroba CLI.

For the terminal-pairing QR/link path, run:

```bash
pnpm --filter @arroba/cli run hosted-terminal-pairing-tui:drill
```

This drill targets the same staging cloud API and Hetzner host, but it launches the remote CLI with `--terminal-pairing-link` generated by the local kernel instead of passing `--relay-url` and `--relay-token` explicitly. It starts a local TUI and an orphan remote TUI, creates one session from each CLI, submits one prompt from each CLI through a real provider, and verifies both completed prompt markers through both the local kernel path and the hosted relay path. Set `ARROBA_CLOUD_DEV_AUTH_SECRET` for non-interactive staging validation. The default provider is `codex` with `gpt-5.2-codex`; override with `ARROBA_CLOUD_HOSTED_REMOTE_CLI_PROVIDER`, `ARROBA_CLOUD_HOSTED_REMOTE_CLI_MODEL`, and `ARROBA_CLOUD_HOSTED_REMOTE_CLI_EFFORT`.

Set `ARROBA_CLOUD_HOSTED_SECOND_KERNEL=1` to launch a second local kernel as a cloud-paired remote machine through the hosted relay. That path uses a `dev-stub` remote agent so it validates remote-machine leasing, prompt dispatch, and completion without requiring provider-account login on the worker machine.

Set `ARROBA_CLOUD_HOSTED_TOKEN_ROTATION=1` to force a hosted machine relay-token refresh while a relay client continuously probes the kernel. This catches token-refresh presence gaps on the WSS hosted path.

## MCP/Skill Drills

`live-mcp-skill-drill.mjs` is local-only for now. It installs a real Playwright MCP into an isolated Arroba registry, optionally installs GitHub MCP when `--include-github-mcp` is set and `GITHUB_PERSONAL_ACCESS_TOKEN` or `GITHUB_TOKEN` is present, installs a deterministic local drill skill, attempts to install a public web skill repo, and verifies per-agent grants plus same-turn skill requests.

Use `--require-web-skill` when the network/web-skill install itself is the thing being validated. Without it, public skill clone failures are reported but do not fail the drill, so local registry/runtime coverage can still run offline.

Use `--live-mcp-use` to also require provider-native Playwright tool calls. The drill covers both user-triggered MCP grants, where `/mcp grant` causes Arroba to relaunch the idle provider conversation, and agent-triggered `request_extension`, where Arroba reloads after the current turn and sends an automatic continuation prompt before requiring a Playwright/browser tool call and workspace live sync marker write.

`live-runtime-mcp-reattach-drill.mjs` is the local regression drill for stale provider servers and CLI rejoin. It warms provider catalog endpoints before launching workspace live sync agents, forcing Codex/OpenCode through the path where a provider server may already be alive without run-specific Arroba MCP config. It then detaches the CLI, reattaches to the same session, submits another prompt to the same agents, and fails unless each agent completes `list_extensions` plus `read_artifact` runtime MCP calls and writes before/after marker files through Arroba workspace live sync.

`live-script-extension-drill.mjs` validates the v1 script extension control plane. It runs an isolated daemon, registers an external Python environment, validates and registers a realistic vector-lookup script with `run`/`test_run`, lists environments/scripts, creates an agent, grants the script extension with its environment, and verifies the durable `extension_grants` shape. Run it with `pnpm --filter @arroba/cli run script-extension:drill`.

`live-script-extension-agent-drill.mjs` validates script extensions through real provider agents. It registers one Python script and one TypeScript script, grants both to each requested agent, requires the provider to call both tools with fixed inputs, verifies per-run hidden tokens returned by plain `run` return values, and verifies the agent writes those observed values through workspace live sync. Run all local providers with `pnpm --filter @arroba/cli run script-extension-agent:drill -- --providers codex,opencode,claude --provider-model codex=gpt-5.2 --provider-model opencode=opencode/gpt-5.2 --provider-model claude=sonnet`.

`live-remote-mcp-drill.mjs` is the remote MCP v1 drill. It launches isolated relay/home/worker daemons with different `HOME` roots so home and worker Arroba user-global MCP registries can diverge on one machine. It verifies worker-missing MCPs, worker global definition mismatches, project-local worker override, missing stdio commands, and missing worker env vars. V1 remote MCPs must already be installed on the worker; the drill does not remotely install MCPs. Pass `--live-mcp-use` to also require a provider-native remote Playwright/browser MCP tool call on the worker and a marker write through Arroba workspace live sync. Because the drill isolates `HOME` for Arroba registries, it preserves provider auth/config/cache via `CODEX_HOME`, `OPENCODE_CONFIG_DIR`, and `XDG_*` provider environment variables.

`live-workflow-runtime-drill.mjs --scenario mcp-echo-workflow` validates workflow-node MCP grants. It installs a deterministic stdio MCP named `workflow_echo`, grants it to the workflow agent before provider launch, requires a provider-native MCP tool call, writes an exact marker through Arroba workspace live sync, and submits final workflow output through `validate_and_submit_workflow_run_output`. The scenario is single-provider by design; run it separately for Codex and OpenCode. The remote wrapper installs the same deterministic MCP in the worker registry before launching the remote workflow drill, matching the v1 remote MCP rule that worker kernels must already have the required MCP definition. For Codex, pass `--provider-model codex=gpt-5.2`; the workflow drill's bare `--model gpt-5.2` fallback maps to `gpt-5.2-codex`, which can be rejected by ChatGPT-backed Codex accounts.

## Before Commit

Run syntax checks for edited scripts before committing:

```bash
node --check apps/cli/scripts/<script>.mjs
```

## Claude Provider Drill

`live-claude-provider-drill.mjs` is the M13.1 live smoke test for the local
Claude Code provider. It launches a kernel, creates a session, launches
`provider=claude`, submits a deterministic prompt, pumps terminal output, and
fails unless the marker reaches history and the prompt settles.

```bash
pnpm --filter @arroba/cli run claude-provider:drill
node apps/cli/scripts/live-claude-provider-drill.mjs --scenario attachment
```

The drill uses the locally installed/logged-in `claude` CLI. It does not set
Anthropic SDK/API-key environment variables.

## Workflow Publication Drill

Use this after touching the workflow gateway, publication auth, publication
export packaging, or HTTP invocation path:

```bash
pnpm --filter @arroba/cli run publication:drill
```

It launches an isolated kernel and gateway, creates a kernel-owned HTTP
publication, invokes it directly, verifies parser failures return HTTP 400,
invokes the same publication over WebSocket, restarts the gateway with
self-signed HTTPS/TLS and invokes it again over HTTPS and WSS, exports it with
`workflow publication export`, starts the gateway from the exported
`publication.config.json`, invokes the exported package through
`arroba-workflow-call`, validates signed Slack URL verification and signed
Slack slash-command invocation, validates Telegram webhook-secret rejection and
accepted invocation, validates Discord Ed25519 ping, signature rejection, and
accepted invocation, validates WhatsApp webhook challenge/HMAC invocation,
validates Signal bridge-secret invocation, then validates paired sender
reject/redeem/invoke/revoke/reject behavior.

Use this after touching cross-kernel publication calls or the custom parser
path:

```bash
pnpm --filter @arroba/cli run workflow-to-workflow-publication:drill
```

It launches two isolated kernels and gateways. Workflow A's published gateway
uses a custom parser that calls workflow B's published HTTP endpoint, then
passes B's accepted run id into workflow A's normalized input.

Use this after touching connector ingress, gateway bind behavior, Docker/local
network assumptions, or provider-shaped webhook verification:

```bash
pnpm --filter @arroba/cli run publication:docker-connectors-drill
```

It builds a Docker client image, launches an isolated kernel and gateway, then
invokes workflow publications from inside the container over HTTP, HTTPS,
WebSocket, WSS, Slack-shaped signed slash commands, Telegram webhook-secret
requests, Discord Ed25519 interactions, WhatsApp HMAC webhooks, and Signal
bridge-secret webhooks. IPC is intentionally excluded because it is a local
process connector rather than a network ingress connector.

Use this to validate the semantic URL renderer application shape on top of
workflow publication:

```bash
pnpm --filter @arroba/cli run semantic-url-renderer:drill
```

It creates a CLI/shell-driven session with one Codex `gpt-5.4` agent, builds a
small static website, creates and publishes a one-node workflow with a custom
parser, then serves URLs like `/about/<prompt>` through an async wrapper. The
first response must be a loading page; later polling must return the workflow
rendered HTML page with prompt-driven styling.

## Kernel Reconnect Drill

Use this after touching CLI/kernel transport recovery:

```bash
pnpm --filter @arroba/cli run build
node apps/cli/scripts/live-kernel-reconnect-drill.mjs
```

It kills the event subscription lane while a control request is pending and fails unless the control request completes and the event lane resubscribes from the last event id.

## Git Observation Drill

Use this after touching local prompt dispatch, provider working directories, operational recall search, or Git observation:

```bash
pnpm --filter @arroba/cli run git-observation:drill
pnpm --filter @arroba/cli run remote-git-observation:drill
```

It launches an isolated local kernel and dev-stub provider inside a temporary Git repo, waits for a prompt to be dispatched, commits a file during the turn, completes the prompt, and fails unless operational history contains a searchable `git_commit_detected` event with the expected commit, path, provider/model, agent, and prompt attribution.

The remote variant launches isolated relay/home/worker kernels, spawns a remote dev-stub agent in a worker Git repo, commits a file on the worker worktree during the remote turn, completes the prompt through home, and fails unless home operational history contains the commit with home agent/prompt ids plus worker machine/repo/worktree metadata.

## Postgres Archive Adapter Drill

Use this after touching operational history archive export, outbox checkpointing, archive adapter auth, or external archive protocol handling:

```bash
pnpm --filter @arroba/cli run postgres-archive:drill
```

It launches an isolated kernel plus real `postgres:16-alpine`, `minio/minio`, and `minio/mc` containers, runs a small HTTP archive adapter in front of Postgres and MinIO, creates transcript events and a transferred artifact through a dev-stub provider/session, and flushes Arroba's durable archive outboxes through `arroba-history-archive-flush`. The drill validates bearer-token auth, `GET /arroba/history/capabilities`, `POST /arroba/history/events`, `POST /arroba/history/search`, `POST /arroba/history/semantic-search`, `PUT /arroba/artifacts/blobs/:artifact_id`, `POST /arroba/artifacts/manifest`, adapter idempotency by `event_id`, HTTP failure retry, durable partial-rejection safety, non-durable rejected-event checkpointing, retry to acceptance, operational-only recall search when external archive search is disabled, Postgres-backed archive search after deleting the matching operational row, semantic archive-search protocol handling, artifact blob storage in MinIO, and artifact manifest storage in Postgres.

## Remote Restart Drill

Use this after touching durable remote agents, relay registration, leased prompt dispatch, or remote restart recovery:

```bash
pnpm --filter @arroba/cli run remote-restart:drill
```

It launches isolated relay, home, and worker kernels, spawns a remote dev-stub agent, prompts it, restarts home, restarts worker, restarts both, and fails unless the home kernel restores the durable remote agent and refreshes stale worker leases. Pass `--keep-artifacts-on-failure` to preserve the isolated logs.
