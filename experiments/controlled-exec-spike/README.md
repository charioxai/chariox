# Controlled Exec Spike

Disposable experiment for M12.

Purpose:

- validate Arroba-controlled command execution
- validate provider-independent permission gating
- validate same-turn approval/choice interactions

This is not production Arroba code.

## Layout

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

## Intended Commands

```bash
cd experiments/controlled-exec-spike
npm install
npm test
npm run drill:fake
npm run drill:providers
```

Validated on 2026-04-23.

What is implemented:

- controlled exec harness with `limited`, `yolo`, and `yolo+rm`
- owned-path ledger for `yolo` delete checks
- interaction gateway for confirm/choice responses
- deterministic fake-agent scenarios
- provider-backed Codex/OpenCode drill through a disposable MCP server that
  wraps the same harness

Artifacts from live provider drills are written under:

```text
experiments/controlled-exec-spike/artifacts/
```
