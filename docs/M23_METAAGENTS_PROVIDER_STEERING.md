# M23 Metaagents Provider Steering Characterization

Date: 2026-06-11

## Drill

Command:

```sh
node apps/cli/scripts/live-provider-context-injection-drill.mjs \
  --providers codex,opencode \
  --provider-model codex=gpt-5.4-mini \
  --provider-model opencode=opencode/gpt-5.2 \
  --timeout-ms 180000 \
  --include-midturn-steering
```

Local provider binaries:

- Codex: `codex-cli 0.135.0`
- OpenCode: `1.4.1`
- Claude: not present on `PATH` in this environment, so Claude steering still needs a machine with Claude Code installed.

The same run also revalidated per-turn hidden context injection for Codex and OpenCode.

Native TUI prompt-injection validation was also rerun after repairing the drill
for the current history-outline API and provider-native hidden-context fields:

```sh
pnpm --filter @arroba/cli run native-tui:prompt-injection-drill -- --providers codex,opencode
```

Result: passed for Codex and OpenCode. The drill verifies native-origin prompts
enter Arroba history/output, provider-hidden context is forwarded using the
current provider-native fields, and the native TUI screen does not show Arroba
hidden instruction markers.

## Arroba Kernel Behavior

Ordinary `SubmitPrompt` does not reach provider steering while an agent has an active prompt. The prompt owner admits the second prompt as queued. Metaagent event prompts and `arroba.meta.run_command` prompt commands intentionally bypass that queue only when the target is a local running provider run with an active prompt; that path records Arroba prompt history and dispatches directly to the adapter.

If the target is remote, idle, missing a running provider run, or otherwise not steerable, the metaagent prompt falls back to normal prompt submission.

## Provider-Native Observations

| Provider | Midturn submit API | Observed result | Transcript shape |
| --- | --- | --- | --- |
| Codex | second `turn/start` while prior turn active | accepted | second marker appeared in the same completed turn; one `turn/completed` was observed for the probe window |
| OpenCode | second `POST /session/{id}/prompt_async` while session busy | accepted | second prompt text appeared under the first assistant child; no separate second assistant child was observed |
| Claude | stream-json stdin second user message while active | not run | blocked because `claude` binary is unavailable locally |

## Metaagent Adaptation

- Codex and OpenCode can receive metaagent event/command prompts midturn through the current adapter calls.
- The metaagent prompt text must include inline event content when small, because the provider may fold the prompt into the active turn rather than produce a separate turn boundary.
- The inbox remains the replay/detail source for large events.
- Claude must stay marked uncharacterized until the same drill is run with Claude Code installed. The implementation still writes the prompt through the existing Claude stdin adapter, but validation is pending.

## Follow-Up Drills

- Run the same steering drill with Claude Code installed.
- Add a steering-specific native prompt submission once the native prompt
  injection drill accepts `--include-midturn-steering`.
- Repeat while the active turn is blocked on a permission/runtime interaction.
