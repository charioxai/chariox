# CLI Agent Response Rendering

Status: active client contract for terminal implementations.

## Goal

Any Chariox CLI should render agent responses consistently without embedding
provider-specific parsing rules in the client.

This document defines:

- what a CLI should consume from Chariox
- how normal assistant output should be rendered
- how tool output should be rendered
- which parsing logic belongs in shared libraries rather than in each client

## Scope

This document covers response rendering for:

- assistant text replies
- provider reasoning/streaming text as exposed through Chariox transcript events
- tool calls emitted by Codex, OpenCode, and Chariox runtime tools

It does not define:

- screen layout or pane structure
- colors/themes
- keyboard shortcuts
- workflow graph rendering

## Rendering Layers

A Chariox CLI should treat agent response rendering as three layers:

1. `Transcript entry selection`
   Choose which transcript/session events belong in the visible response stream.
2. `Message rendering`
   Render normal assistant/user/system text through the shared transcript
   markdown path.
3. `Tool rendering`
   Render tool output through the shared `ToolDisplay` contract.

The important rule is that only layer 3 is provider-shape-sensitive, and that
sensitivity must live in shared Chariox code, not in each CLI.

## Canonical Inputs

### Normal assistant responses

Normal agent replies should be rendered from Chariox transcript/message records,
not from provider raw PTY output.

Clients should prefer:

- finalized assistant transcript entries
- streamed assistant deltas that Chariox has already classified as assistant
  text

Clients should not scrape:

- raw provider terminal text
- provider JSON events directly
- tool blobs embedded in provider stderr/stdout

### Tool responses

Tool responses should be rendered from normalized `ToolTranscriptUpdate`
records and formatted through the shared tool display layer.

Current shared assets:

```text
packages/tool-display/src/index.ts
packages/tool-display/schema/tool-display.schema.json
packages/tool-display/fixtures/
```

TypeScript clients should depend on `@chariox/tool-display`.

Non-TypeScript clients should consume:

- `packages/tool-display/schema/tool-display.schema.json`
- `packages/tool-display/fixtures/`

## Normal Assistant Message Rendering

For non-tool agent output, the CLI should:

1. render markdown/plain text through the shared transcript markdown renderer
2. preserve paragraph breaks and code fences
3. keep inline code, fenced code, and lists readable in narrow terminals
4. avoid showing provider-native wrappers or JSON unless the assistant
   intentionally returned JSON as content

Recommended behavior:

- stream text progressively while the assistant is speaking
- collapse provider bookkeeping
- keep the final assistant reply identical to the saved transcript entry

## Tool Rendering Contract

The canonical expanded tool representation is `ToolDisplay`.

Schema:

- `version`
- `tool`
- `status`
- `title`
- `summary`
- `collapsed`
- `blocks`

Expanded blocks can currently be:

- `text`
- `code`
- `json`
- `table`
- `patch`

Clients should follow this order:

1. receive a normalized `ToolTranscriptUpdate`
2. call the shared formatter
3. render `ToolDisplay.blocks` for the expanded view
4. render `ToolDisplay.collapsed` for compact transcript rows

Clients should not implement provider-specific shape handling locally unless a
new raw provider shape has first been added to the shared fixture corpus and
covered by shared tests.

## Patch Rendering

Patch rendering has a specific rule:

- default transcript expansion shows the new side only

That means:

- no side-by-side diff
- no old-code left column
- no duplicated removed text in the normal transcript view

The patch block exposes `previewLines`, which are the default rendering source
for CLIs.

`files[].diff` may still exist for future full-diff/review views, but that is
not the default transcript rendering contract.

## Collapsed vs Expanded Output

Each tool blob should support two render states.

### Collapsed

Use:

- `ToolDisplay.collapsed.title`
- `ToolDisplay.collapsed.summary`

This is the compact row shown in transcript mode when the blob is not expanded.

### Expanded

Use:

- `ToolDisplay.title`
- `ToolDisplay.summary`
- `ToolDisplay.blocks`

Expanded rendering should be deterministic and should not require the client to
inspect provider raw JSON.

## Provider Independence Rule

Codex and OpenCode emit different raw tool shapes. Chariox normalizes them
before the client should care.

Examples already handled in the shared formatter:

- Codex runtime patch payloads using `patch_text`
- OpenCode Chariox runtime tool aliases such as:
  - `chariox_read_artifact`
  - `chariox_apply_patch`
- OpenCode snake_case inputs

If another provider/model emits a new raw shape, fix it once in the shared tool
display package, add fixtures, and keep client renderers unchanged.

## Error Rendering

A CLI should distinguish:

- assistant/tool content
- terminal/provider failures
- kernel/runtime notices

Tool failure rendering should use the normalized tool/error status where
available. Provider crash text or transport failures should not be rendered as
if they were successful tool output.

Recommended behavior:

- show tool `status` when present
- surface provider failure text in an error-styled transcript blob
- never silently drop provider/kernel failure events that terminate a turn

## Cross-Client Reuse

To keep CLI, web, iOS, and Android aligned:

- normalization lives in shared Chariox code
- fixtures define raw-provider compatibility examples
- schema defines the language-neutral output contract

This is the intended reuse model:

1. provider adapters produce transcript/tool updates
2. shared formatter converts tool updates into `ToolDisplay`
3. each client renders the same normalized structure in its own UI idiom

Clients should differ in presentation, not in parsing semantics.

## Minimal Implementation Checklist

Any new CLI implementation should:

- consume normal transcript entries for assistant text
- use the shared transcript markdown path for message bodies
- format tool updates through `@chariox/tool-display` or the schema-equivalent
  implementation
- render patch preview from `previewLines`
- use collapsed tool title/summary for compact rows
- avoid direct Codex/OpenCode raw JSON parsing in UI code

## Validation

When changing rendering behavior, validate against:

- shared unit tests in `packages/tool-display`
- fixture corpus in `packages/tool-display/fixtures/`
- live drill script:

```bash
node apps/cli/scripts/live-tool-display-fixture-drill.mjs --providers codex,opencode --timeout-ms 300000 --keep-artifacts-on-failure
```

If a client needs a new rendering behavior, update the shared contract first,
then the client.
