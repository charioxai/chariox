# MCP Isolation Spike

This is a disposable repo-local spike for M7. It validates whether Chariox should keep per-agent MCP grants as a production design.

The spike is intentionally outside production `apps/` and `packages/` code. It should produce evidence before production work continues on dynamic MCP grant activation.

## What This Tests

The target architecture is:

```text
one isolated provider runtime per agent
provider-native MCP config contains only that agent's granted MCPs
MCP backing runtimes are supervised separately from provider process lifetimes
```

The key scalability question is whether adding MCP B/C/D to an agent forces MCP A to cold-restart. The desired answer is no: provider runtimes may reconnect/re-handshake, but existing MCP backing runtimes should remain warm.

## Current Slices

Implemented now:

- `src/fake-mcp-server.mjs`: minimal stdio MCP server with `initialize`, `tools/list`, and `tools/call` support.
- `src/mcp-proxy-server.mjs`: provider-facing stdio proxy that restarts with providers while keeping backing MCP lifecycle counters warm.
- `src/mcp-supervisor.mjs`: starts and reuses fake MCP backing processes, recording lifecycle counters.
- `src/provider-launcher.mjs`: renders per-agent Codex/OpenCode provider-native MCP config plans.
- `src/codex-driver.mjs`: launches isolated Codex app-servers and drives app-server JSON-RPC initialization/thread start.
- `src/scenarios.mjs lifecycle`: proves supervised fake MCP backing reuse across incremental grant growth.
- `src/scenarios.mjs visibility-plan`: generates per-agent provider config plans and verifies fake MCP framing.
- `src/scenarios.mjs opencode-status`: launches two isolated OpenCode servers and verifies one server has `fake-alpha` connected while the other has no `fake-alpha` MCP entry.
- `src/scenarios.mjs codex-thread-start`: launches two isolated Codex app-servers and verifies only the granted agent starts/lists `fake-alpha`.
- `src/scenarios.mjs codex-restart-resume`: restarts Codex with an expanded proxied MCP set, verifies the same thread ID resumes, and verifies existing backing MCPs stay warm.
- `src/scenarios.mjs opencode-restart-resume`: restarts OpenCode with an expanded proxied MCP set, verifies the same session ID is reused, and verifies existing backing MCPs stay warm.
- `src/scenarios.mjs codex-agent-triggered-grant`: simulates a Codex agent-requested MCP grant, provider relaunch, and synthetic continuation.
- `src/scenarios.mjs opencode-agent-triggered-grant`: simulates an OpenCode agent-requested MCP grant, provider relaunch, and synthetic continuation.
- `src/scenarios.mjs scale-matrix`: runs a small Codex/OpenCode provider/proxy/backing lifecycle scale matrix.
- `src/scenarios.mjs overlap-isolation`: validates overlapping-but-not-identical grants across two agents per provider.

Not implemented yet:

- productionizing this architecture in Chariox kernel/provider runtime code
- larger scale runs with real MCPs and provider memory measurements

## Commands

From this directory:

```bash
node --check src/fake-mcp-server.mjs
node src/scenarios.mjs lifecycle
node src/scenarios.mjs visibility-plan
node src/scenarios.mjs opencode-status
node src/scenarios.mjs codex-thread-start
node src/scenarios.mjs codex-restart-resume
node src/scenarios.mjs opencode-restart-resume
node src/scenarios.mjs codex-agent-triggered-grant
node src/scenarios.mjs opencode-agent-triggered-grant
node src/scenarios.mjs scale-matrix
node src/scenarios.mjs overlap-isolation
```

Or with npm/pnpm script runners:

```bash
npm run check
npm run scenario:lifecycle
npm run scenario:visibility-plan
npm run scenario:opencode-status
npm run scenario:codex-thread-start
npm run scenario:codex-restart-resume
npm run scenario:opencode-restart-resume
npm run scenario:codex-agent-triggered-grant
npm run scenario:opencode-agent-triggered-grant
npm run scenario:scale-matrix
npm run scenario:overlap-isolation
```

## Artifacts

Runs write timestamped artifacts under:

```text
experiments/mcp-isolation-spike/artifacts/<timestamp>-<scenario>/
```

Key files:

- `results.json`: scenario result and assertions
- `mcp-state.json`: fake MCP lifecycle counters
- `provider-plans/*.json`: generated provider-native config plans for Codex/OpenCode

These artifacts are disposable and should not be committed.

## Next Implementation Steps

1. Convert the spike result into a production implementation plan for Chariox kernel/provider runtime.
2. Decide whether production backing runtimes should default to per-agent, shared, or capability-configurable.
3. Add a larger optional/nightly drill with real MCPs once production wiring exists.
