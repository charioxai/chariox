# M7 MCP Isolation Spike Plan

Updated: 2026-04-18

## Decision

The repo-local disposable spike validated the provider/runtime architecture. Production implementation can proceed by integrating the validated isolation/proxy design into Arroba source code.

The spike lives inside the repo so it can reuse scripts, local provider binaries, and drill conventions, but it is not production Arroba code. The expected location is:

```text
experiments/mcp-isolation-spike/
```

Current implementation status:

- fake MCP server and supervisor are implemented.
- per-agent Codex/OpenCode provider config rendering is implemented.
- OpenCode live status isolation is validated with two isolated `opencode serve` processes.
- Codex live thread-start isolation is validated with two isolated `codex app-server` processes.
- Codex app-server restart/resume with an expanded proxied MCP set is validated with thread-ID preservation, warm `fake-alpha` backing runtime, and a model-level `fake-beta` tool call after resume.
- OpenCode restart/resume with an expanded proxied MCP set is validated with session-ID preservation, warm `fake-alpha` backing runtime, and a model-level `fake-beta` tool call after resume.
- Explicit transcript text capture is validated for restart/resume.
- Agent-triggered grant continuation is validated for Codex and OpenCode through provider relaunch plus synthetic continuation.
- A small scale matrix is validated for Codex and OpenCode across 1-2 agents and 1-3 MCPs per agent.
- Overlapping-but-not-identical grants are validated for Codex and OpenCode with `agent-a: fake-alpha,fake-beta` and `agent-b: fake-beta,fake-gamma`.
- Production integration has started: Arroba now has provider-facing proxy config generation, unique agent-scoped runtime MCP tokens, an authenticated `/mcp/proxy/<name>` route, stdio backing lifecycle supervision keyed by MCP definition hash, provider launch wiring to render proxy configs by default when the runtime MCP binding is available, and streamable HTTP/HTTPS MCP backing relay tests, including chunked responses.
- Local production drills now pass for Codex and OpenCode using real Playwright MCP plus a deterministic local echo MCP. The drills cover pre-granted MCP activation, provider-native MCP tool calls, agent-triggered `request_extension`, provider conversation relaunch/resume, automatic continuation, and workspace live sync marker writes.
- Remaining production work is remote/workflow MCP drills after production proxy integration.

## Why This Exists

Per-agent MCP grants are a central Arroba product bet. They require stricter isolation than provider-global MCP installs:

- Agent A granted Playwright must be able to use Playwright.
- Agent B in the same Arroba session, without that grant, must not see or use Playwright.
- Adding an MCP to one agent must not expose it to other agents.
- Adding one MCP to an agent must not cold-restart every existing MCP backing server for that agent.

Provider-global MCP configuration is simple but does not provide this boundary. The spike validated one provider server/process per agent as the practical way to make provider-native MCP exposure match Arroba's grant boundary, especially for OpenCode. Production work should now preserve that invariant and add the Arroba supervisor/proxy layer for backing MCP lifecycle reuse.

## Architecture Hypothesis

The architecture under test is:

```text
Arroba home/kernel truth
  owns MCP registry and per-agent grants

Provider runtime per agent
  one isolated provider server/process per agent/provider runtime
  receives only that agent's granted MCPs in provider-native MCP config

MCP supervisor/proxy layer
  owns MCP backing server lifetimes
  can keep MCP servers warm across provider restarts
  exposes provider-native MCP endpoints/config entries for the granted set
```

The important separation is:

- provider runtime lifecycle controls what an agent can see
- MCP runtime lifecycle controls whether MCP servers are cold-started, reused, supervised, or stopped

A provider process may need to restart/resume when its visible MCP set changes. Existing MCP backing servers should not be cold-restarted just because the provider process is re-created with a larger MCP set.

## Non-Goals

The spike must not become a second Arroba implementation; it is now evidence for production integration.

Do not implement:

- production CLI UX
- workflow integration
- remote worker support
- permissions prompts
- persistent production registry migration
- polished user-facing errors
- non-interactive command surfaces

The spike answered whether the design works and scales; production code must now implement the equivalent behavior inside the kernel/provider runtime paths.

## Test Questions

The spike must produce evidence for these questions:

1. Can Codex and OpenCode run one isolated provider server/process per agent?
2. Can two agents in one session/provider family see different MCP sets?
3. Can a provider process be restarted/resumed with an expanded MCP set while preserving enough conversation context?
4. Can existing MCP backing servers stay warm while provider processes restart?
5. Can agent-triggered MCP requests be completed through restart/resume plus a synthetic continuation message?
6. Does the approach remain stable with multiple agents and multiple MCPs per agent?

## Minimal Components

The spike should contain only small harness pieces:

```text
experiments/mcp-isolation-spike/
  README.md
  package.json                 optional, if Node is used
  src/
    fake-mcp-server.*          lifecycle counters and simple tools
    mcp-supervisor.*           start/reuse/stop fake MCP backing runtimes
    provider-launcher.*        launch Codex/OpenCode provider servers with generated MCP config
    codex-driver.*             start/resume/thread prompt helpers
    opencode-driver.*          start/resume/session prompt helpers
    scenarios.*                scenario matrix runner
  artifacts/                   gitignored outputs from local runs
```

Implementation language is not prescribed. Use the fastest route that can reliably drive both providers locally. Node is acceptable because existing live drill scripts are Node-based.

## Scenario Matrix

### S1 Per-Agent Visibility

Launch two provider runtimes for the same provider:

```text
agent-a: MCP fake-alpha
agent-b: no fake-alpha
```

Prompt both agents to list/use available MCP tools.

Expected:

- agent-a can see and call `fake-alpha`
- agent-b cannot see or call `fake-alpha`
- no provider-global leakage occurs

Run for:

- Codex
- OpenCode

### S2 Incremental Grant Growth

Start one agent with one MCP:

```text
agent-a: fake-alpha
```

Then expand grants:

```text
agent-a: fake-alpha, fake-beta
agent-a: fake-alpha, fake-beta, fake-gamma
agent-a: fake-alpha, fake-beta, fake-gamma, fake-delta
```

Expected:

- provider runtime can restart/resume with the larger visible MCP set
- fake-alpha backing server is not cold-restarted when fake-beta/gamma/delta are added
- provider may reconnect/re-handshake with fake-alpha; that is acceptable
- elapsed activation time is recorded for each grant expansion

### S3 Conversation Resume

For each provider:

1. Start a conversation and ask the model to remember a unique token.
2. Restart/resume provider runtime with an expanded MCP set.
3. Ask the model to use prior context and call the new MCP.

Expected:

- the provider resumes the same conversation/thread/session or an equivalent saved context
- prior context survives sufficiently for normal continuation
- the new MCP is visible and usable after restart

Codex-specific check:

- use saved Codex thread ID if available, equivalent to `codex resume <thread-id>` semantics

OpenCode-specific check:

- use saved OpenCode session ID or provider-supported resume path if available

### S4 Agent-Triggered Grant

Simulate:

```text
agent prompt: "If browser/playwright is needed, request it."
agent calls Arroba-like request_extension(kind=mcp, name=fake-browser)
harness grants MCP
harness restarts/resumes provider runtime
harness sends: "MCP is now loaded. Continue."
agent calls fake-browser provider-native MCP tool
```

Expected:

- the model continues without manual user intervention
- provider-native MCP tool is used after restart
- transcript makes the reload understandable, not confusing

If same-turn continuation is unreliable, document that agent-triggered MCP grants are only effective next prompt/run.

### S5 Scale Envelope

Run a small matrix:

```text
agents: 1, 2, 4, 8
MCPs per agent: 1, 3, 5, 10
providers: codex, opencode
```

Record:

- provider process count
- MCP backing process count
- cold MCP starts
- MCP initialize/list calls
- provider startup time
- grant activation time
- memory usage where practical
- failure rate

This does not need exhaustive benchmarking, but it must reveal whether the design is obviously too expensive.

### S6 Shared vs Per-Agent MCP Backing Runtime

Use fake MCP counters to compare:

- provider-owned raw stdio MCP process
- Arroba-supervised shared backing process with per-agent proxy endpoint
- Arroba-supervised per-agent backing process

Expected output:

- whether existing MCP backing servers restart during provider reload
- whether tool calls can be attributed to the correct agent
- whether shared backing is safe enough for stateless MCPs
- whether stateful MCPs should default to per-agent backing runtimes

## Production Lessons From Integration

The production integration kept the spike's provider-process-per-agent boundary, but changed backing exposure from direct raw MCP configs to provider-facing proxy MCP configs.

Validated details:

- Codex and OpenCode can use normal provider-native MCP tools through Arroba proxy endpoints.
- Stdio MCP compatibility required newline-delimited JSON writes to the backing process; responses accept newline-delimited JSON or `Content-Length`.
- The proxy must not wait for JSON-RPC notification responses. Notifications through `/mcp/proxy/<name>` return `202 Accepted`.
- Codex launch and thread-start config both need the same proxy MCP config. Otherwise Codex can merge stale stdio definitions with proxy URL definitions and fail startup.
- Codex should receive MCP bearer tokens through `bearer_token_env_var`, not inline HTTP headers in launch arguments.
- Provider-native approval systems can reject newly exposed third-party MCP tools after Arroba has already granted them. V1 treats the Arroba grant as the approval boundary and marks granted proxy tools as non-destructive/non-open-world/read-only in `tools/list`.
- Agent-triggered relaunch must preserve the previous run's workspace live sync mode. A combined drill caught an OpenCode replacement run launching unmanaged; this is now fixed.

## Measurements

Every scenario should write a machine-readable result file, for example:

```text
experiments/mcp-isolation-spike/artifacts/<timestamp>/results.json
```

Suggested fields:

```json
{
  "provider": "opencode",
  "scenario": "incremental_grant_growth",
  "passed": true,
  "agents": 2,
  "mcps_per_agent": 4,
  "provider_processes_started": 2,
  "mcp_backing_processes_started": 4,
  "mcp_cold_starts": { "fake-alpha": 1 },
  "mcp_initialize_calls": { "fake-alpha": 3 },
  "provider_restart_ms": [1200, 1320],
  "grant_activation_ms": [1450, 1510],
  "notes": []
}
```

## Success Criteria

The grant-based MCP architecture is viable only if all of these are true:

- Codex isolates MCP visibility per agent.
- OpenCode isolates MCP visibility per agent.
- Provider conversation resume works after provider restart for both providers.
- Adding MCP B does not cold-restart MCP A's backing server.
- Agent-triggered grant continuation works or has an acceptable next-turn fallback.
- Four agents with multiple MCPs each are stable enough for interactive use.
- The resulting design can be explained as provider-runtime isolation plus MCP lifecycle supervision.

## Failure Criteria

If any of these happen, production M7 should simplify:

- OpenCode cannot reliably preserve conversation/session across isolated server restart.
- Codex cannot reliably preserve conversation/thread across isolated app-server restart with changed MCP config.
- Provider-native MCP visibility leaks across agents.
- Provider restart/reconnect latency becomes unacceptable as MCP count grows.
- MCP process supervision/proxying requires too much harness-specific emulation.
- The same-turn illusion creates confusing or brittle transcripts.

## Fallback Design If Spike Fails

If the spike fails, v1 should drop per-agent MCP grants and follow harness-style MCP exposure:

- MCPs are provider-global, project-global, or remote-machine-global according to provider behavior.
- Arroba still manages MCP install/import/list/sync UX.
- Arroba remote support checks that worker machines have equivalent provider/global MCP setup.
- Skills may remain Arroba-managed and per-agent if prompt injection/materialization remains reliable.
- Agent-triggered MCP request/reload is removed from v1.

This fallback keeps useful MCP management while avoiding a brittle isolation system.

## Production Work Paused Pending Spike

Do not continue production work on these areas until the spike reports results:

- provider restart/reload as MCP grant activation
- OpenCode dynamic MCP grant activation
- Codex MCP reload/resume drills
- per-agent MCP grant strictness beyond already-landed storage/rendering
- MCP supervisor/proxy production implementation

Allowed while spike is pending:

- documentation cleanup
- non-MCP skill work that does not depend on provider-runtime isolation
- unrelated bug fixes
- experiments under `experiments/mcp-isolation-spike/`
