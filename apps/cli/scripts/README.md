# Live Drill Policy

These scripts exercise Arroba against real provider sessions. Keep them deterministic, cheap, and explicit so future agents can rerun them safely.

## Provider Models

- Default live-drill model: `gpt-5.2`.
- Use older Codex-capable models such as `gpt-5.2` or `gpt-5.3` for routine drills.
- Use reasoning effort `low` unless the drill is specifically validating reasoning-heavy behavior.
- For `opencode`, use OpenAI Codex-family models rather than OpenCode's provider-default or `zen` model family.
- If an OpenCode drill receives an unqualified model such as `gpt-5.2`, map it to `openai/gpt-5.2-codex`.
- Prefer an explicit override when debugging provider-specific behavior:

```bash
node apps/cli/scripts/live-managed-io-drill.mjs --provider opencode --provider-model opencode=openai/gpt-5.2-codex
```

## Wrapper Scripts

Remote wrapper drills must forward `--provider-model PROVIDER=MODEL` to their child drill scripts. This keeps local and remote provider sessions on the same model policy.

## Cleanup

Live drills should own their daemon/session/port/artifact lifecycle and clean up generated files on success. If a drill supports `--keep-artifacts-on-failure`, only leave artifacts behind on failure for debugging.

## Managed I/O Identity

Managed-I/O drills coordinate only while the provider run remains in the same repo/branch/head identity captured by the kernel. If a drill or concurrent developer action changes that identity mid-run, `workspace_identity_changed` is a valid failure mode. Restart the drill from a stable workspace identity rather than treating that rejection as a file-edit collision.

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
