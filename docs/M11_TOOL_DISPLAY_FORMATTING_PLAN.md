# M11 Tool Display Formatting Plan

Status: in progress. M11.1-M11.6 are implemented.

## Goal

Make provider and runtime tool output readable across Arroba clients. Codex,
OpenCode, CLI, web, iOS, and Android should not each invent their own parser
for provider-specific JSON blobs.

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
  fixtures when raw provider shapes differ.
- M11.8 document web/iOS/Android consumption rules.

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
