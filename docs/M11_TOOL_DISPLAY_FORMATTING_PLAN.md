# M11 Tool Display Formatting Plan

Status: in progress. M11.1-M11.8 are implemented.

## Goal

Make provider and runtime tool output readable across Arroba clients. Codex,
OpenCode, CLI, web, iOS, and Android should not each invent their own parser
for provider-specific JSON blobs.

The client-facing rendering rules now live in
`docs/CLI_AGENT_RESPONSE_RENDERING.md`.

## Design

Arroba normalizes raw provider tool updates into a shared `ToolDisplay` JSON
model. Clients render the shared model instead of interpreting provider raw
JSON directly.

Shared assets live in:

```text
packages/tool-display/
  schema/tool-display.schema.json
  fixtures/
  src/
```

The schema is language-neutral and can be consumed by terminal, web, and native
clients. TypeScript clients use `@arroba/tool-display`; other clients can use
the JSON schema and fixture corpus as their rendering contract.

## Display Model

Each normalized tool display contains:

- `version`: schema version.
- `tool`: canonical tool name.
- `status`: provider status when available.
- `title` and `summary`: expanded header text.
- `collapsed`: compact title/summary for collapsed transcript blobs.
- `blocks`: renderable blocks such as text, code, table, JSON, or patch.

Patch blocks expose `previewLines`, which show the new side of a change only.
Old-side removed lines are intentionally omitted in the default transcript
view because split diffs become noisy when several agents are active.

## Milestone Tasks

- M11.1 document shared tool display schema and client contract: complete.
- M11.2 add `@arroba/tool-display` package with legacy formatter parity:
  complete.
- M11.3 move CLI transcript tool parsing/formatting to the shared package:
  complete.
- M11.4 change `apply_patch` blobs to new-changes-only rendering: complete.
- M11.5 collect Codex/OpenCode fixture corpus across available models and
  tools: complete for the default authenticated local targets.
- M11.6 add golden tests for every supported tool family: complete for the
  current fixture corpus.
- M11.7 run live drills across expanded provider/model catalog and update
  fixtures when raw provider shapes differ: complete for OpenCode Zen model
  coverage with documented provider/model exceptions.
- M11.8 document web/iOS/Android consumption rules: complete.

## Initial Scope

The first implementation keeps the existing CLI markdown summaries stable, then
adds the shared `ToolDisplay` representation beside them. CLI continues to
render the same transcript entry text for compatibility, but patch expansion is
now rendered through `previewLines`.

Initial supported tool families:

- shell/bash commands
- file read
- grep/search
- todo write
- provider-native `apply_patch`
- Arroba managed-I/O patch/edit/write/move/delete outputs

## Validation

Validated commands:

```bash
pnpm --filter @arroba/tool-display test
pnpm --filter @arroba/cli test
node --check apps/cli/scripts/live-tool-display-fixture-drill.mjs
node apps/cli/scripts/live-tool-display-fixture-drill.mjs --providers codex,opencode --list-targets
node apps/cli/scripts/live-tool-display-fixture-drill.mjs --providers codex,opencode --timeout-ms 300000 --keep-artifacts-on-failure
pnpm --filter @arroba/tool-display test
pnpm --filter @arroba/cli test
node apps/cli/scripts/live-tool-display-fixture-drill.mjs --provider opencode --all-models --max-models-per-provider 80 --timeout-ms 180000 --poll-ms 1000 --continue-on-failure --keep-artifacts-on-failure
```

The fixture drill resolved the default local targets as:

- `codex gpt-5.4`
- `opencode openai/gpt-5.4`

Codex `gpt-5.4` produced raw provider tool events for Arroba runtime read and
patch tools and wrote them to `target/tool-display-fixtures/codex-gpt-5.4.jsonl`.

OpenCode `openai/gpt-5.4` initially narrated tool use without invoking tools.
The fixture drill now uses two stricter phases and requires read plus patch
events before passing. With that prompt shape, OpenCode produced runtime read,
grep, bash, and runtime patch events and wrote them to
`target/tool-display-fixtures/opencode-openai_gpt-5.4.jsonl`.

OpenCode also exposed an important raw-shape difference: Arroba runtime MCP
tools can arrive as underscore names such as `arroba_read_artifact` and
`arroba_apply_patch`, with snake_case inputs such as `patch_text`. The shared
formatter now normalizes these aliases so CLI/web/native clients do not render
those blobs as raw JSON.

The expanded `--all-models` drill now qualifies OpenCode catalog model ids as
`opencode/<model>` so the OpenCode submit path is forced to the intended Zen
model instead of falling back to the provider default. The drill also waits for
explicit assistant phase markers in streamed history so slower models cannot be
advanced to phase 2 before phase 1 has actually completed.

OpenCode Zen model coverage on 2026-04-23:

- 37 `opencode/*` model targets produced read plus patch tool fixtures.
- `opencode/minimax-m2.5` originally emitted XML-like pseudo-tool text instead
  of OpenCode tool events when prompted with dotted Arroba tool names. The drill
  now prompts OpenCode targets with underscore runtime tool names such as
  `arroba_read_artifact` and `arroba_apply_patch`; `opencode/minimax-m2.5`
  passes with that prompt shape.
- `opencode/claude-3-5-haiku` is skipped in expanded catalog drills. After
  `opencode models --refresh`, direct OpenCode invocation still fails with
  `model: claude-3-5-haiku-20241022`, before Arroba or tool display handling is
  involved.
- `opencode/gpt-5.4-pro` failed because the drill used low effort, while that
  model requires medium/high/xhigh. We are not expanding effort coverage in M11
  because effort variants should expose the same tool surface.
- `opencode/gpt-5.4-pro` is also skipped in expanded catalog drills with the
  default low-effort invocation.

## Client Consumption Rules

All clients should consume the shared display contract in this order:

1. Parse provider/runtime tool chunks as `ToolTranscriptUpdate`.
2. Call the shared formatter for a normalized `ToolDisplay`.
3. Render `ToolDisplay.blocks` as the canonical expanded representation.
4. Use `ToolDisplay.collapsed` for compact transcript rows.
5. Avoid provider-specific parsing in app code unless a new raw provider shape
   has first been added to `packages/tool-display/fixtures/` and covered by a
   golden test.

Patch blocks should render `previewLines` by default. Removed old-side lines
are intentionally excluded from the normal transcript expansion. A future
full-diff mode can still use the original `files[].diff` payload if a client
needs an explicit before/after review view.

Web and native clients that are not TypeScript consumers should validate
against `packages/tool-display/schema/tool-display.schema.json` and use the
fixture corpus as compatibility examples. TypeScript clients should depend on
`@arroba/tool-display` directly.

## Drills

For each available provider/model, run a disposable session that asks the model
to call each available tool once with harmless inputs. Store sanitized raw tool
updates under `packages/tool-display/fixtures/` and add a golden assertion for
the normalized `ToolDisplay`.

Live drill requirements:

- include both Codex and OpenCode where authenticated
- include provider-native tools and Arroba runtime MCP tools
- include `apply_patch` against disposable files only
- verify collapsed blob title/summary
- verify expanded block rendering does not expose raw JSON unless the tool is
  genuinely a JSON payload
- verify patch expansion does not show the old-code column
