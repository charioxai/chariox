import test from "node:test"
import assert from "node:assert/strict"

import {
  formatToolTranscriptUpdate,
  guessPathFenceLanguage,
  mergeToolTranscriptUpdate,
  normalizeMarkdownFenceInfoStrings,
  parseToolTranscriptUpdate,
  readApplyPatchFiles,
  shouldRenderProviderStatus,
  shouldRenderTranscriptAsMarkdown,
  splitInlineCodeSpans,
} from "./transcript.js"

test("parseToolTranscriptUpdate reads structured tool payloads", () => {
  const parsed = parseToolTranscriptUpdate('{"id":"tool-1","tool":"bash","status":"running"}')
  assert.deepEqual(parsed, {
    id: "tool-1",
    tool: "bash",
    status: "running",
  })
})

test("formatToolTranscriptUpdate renders bash command inline with output", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-1",
      tool: "bash",
      status: "completed",
      input: { command: "git status" },
      output: "On branch main",
      description: "Shows working tree status",
    }),
    [
      "**bash** · completed",
      "Shows working tree status",
      "**Command**\n```bash\n$ git status\n```",
      "**Output**\n```text\nOn branch main\n```",
    ].join("\n\n"),
  )
})

test("formatToolTranscriptUpdate falls back to rendered input and errors", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-2",
      tool: "read",
      status: "error",
      input: { filePath: "/tmp/demo.txt" },
      error: "file not found",
    }),
    [
      "**read** · error",
      '**Input**\n```json\n{\n  "filePath": "/tmp/demo.txt"\n}\n```',
      "**Error**\n```text\nfile not found\n```",
    ].join("\n\n"),
  )
})

test("formatToolTranscriptUpdate renders todos as a checklist", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-3",
      tool: "todowrite",
      status: "completed",
      input: {
        todos: [
          {
            content: "Remove temporary idle-status debug logs from CLI and daemon",
            priority: "high",
            status: "completed",
          },
          {
            content: "Run CLI and daemon tests after log cleanup",
            priority: "medium",
            status: "pending",
          },
        ],
      },
    }),
    [
      "**Todo list** · completed",
      "Remaining: 1 todo",
      "- [x] Remove temporary idle-status debug logs from CLI and daemon",
      "- [ ] Run CLI and daemon tests after log cleanup",
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate renders read output with a compact header", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-4",
      tool: "read",
      status: "completed",
      input: {
        filePath: "apps/daemon/src/provider/service.rs",
        offset: 480,
        limit: 220,
      },
      output: [
        "<path>/Users/miguel/arroba/apps/daemon/src/provider/service.rs</path>",
        "<type>file</type>",
        "<content>1: first",
        "2: second",
        "</content>",
      ].join("\n"),
    }),
    [
      "**read** · completed",
      "`apps/daemon/src/provider/service.rs [offset=480, limit=220]`",
      "",
      "```rust",
      "1: first",
      "2: second",
      "```",
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate collapses long read output in the middle", () => {
  const content = Array.from({ length: 24 }, (_, index) => `${index + 1}: line ${index + 1}`).join("\n")

  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-5",
      tool: "read",
      status: "completed",
      input: {
        filePath: "apps/cli/src/runtime.ts",
      },
      output: `<path>/Users/miguel/arroba/apps/cli/src/runtime.ts</path>\n<type>file</type>\n<content>${content}\n</content>`,
    }),
    [
      "**read** · completed",
      "`apps/cli/src/runtime.ts`",
      "",
      "```typescript",
      ...Array.from({ length: 10 }, (_, index) => `${index + 1}: line ${index + 1}`),
      "...",
      ...Array.from({ length: 10 }, (_, index) => `${index + 15}: line ${index + 15}`),
      "```",
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate infers read syntax from common file extensions", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-8",
      tool: "read",
      status: "completed",
      input: {
        filePath: "apps/cli/src/index.tsx",
      },
      output: "<path>/Users/miguel/arroba/apps/cli/src/index.tsx</path>\n<type>file</type>\n<content>1: const value = 1\n</content>",
    }),
    [
      "**read** · completed",
      "`apps/cli/src/index.tsx`",
      "",
      "```typescriptreact",
      "1: const value = 1",
      "```",
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate renders grep output with a compact header", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-6",
      tool: "grep",
      status: "completed",
      input: {
        pattern: "status_updates.push|provider_idle = true|OpenCode is idle|thinking|idle",
        path: "/Users/miguel/arroba",
      },
      output: [
        "Found 13 matches",
        "/Users/miguel/arroba/apps/daemon/src/provider/service.rs:",
        "  Line 416:             status_updates.push(delta)",
        "  Line 418:             provider_idle = true;",
      ].join("\n"),
    }),
    [
      "**grep** · completed",
      "Pattern: `status_updates.push|provider_idle = true|OpenCode is idle|thinking|idle` in apps/daemon/src/provider/service.rs (13 matches)",
      "`apps/daemon/src/provider/service.rs`",
      "```rust",
      "Line 416:             status_updates.push(delta)",
      "Line 418:             provider_idle = true;",
      "```",
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate renders grep no-files-found compactly", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-7",
      tool: "grep",
      status: "completed",
      input: {
        pattern: "handleSessionCommand|submitPrompt\\(|transitionToNoSession|buildNoSessionRenderable",
        path: "/Users/miguel/arroba/apps/cli/src",
        include: "*.test.ts",
      },
      output: "No files found",
    }),
    [
      "**grep** · completed",
      "Pattern: `handleSessionCommand|submitPrompt\\(|transitionToNoSession|buildNoSessionRenderable` in /Users/miguel/arroba/apps/cli/src [*.test.ts] (0 matches)",
      "```text",
      "No files found",
      "```",
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate renders multi-file grep output with per-file syntax", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-9",
      tool: "grep",
      status: "completed",
      input: {
        pattern: "highlight|syntax",
        path: "/Users/miguel/arroba/opencode-dev",
      },
      output: [
        "Found 3 matches",
        "/Users/miguel/arroba/opencode-dev/package.json:",
        '  Line 55:       "marked-shiki": "1.2.1",',
        "/Users/miguel/arroba/opencode-dev/script/publish.ts:",
        "  Line 7: const highlightsTemplate = `",
      ].join("\n"),
    }),
    [
      "**grep** · completed",
      "Pattern: `highlight|syntax` (3 matches in 2 files)",
      "`package.json`",
      "```json",
      'Line 55:       "marked-shiki": "1.2.1",',
      "```",
      "`script/publish.ts`",
      "```typescript",
      "Line 7: const highlightsTemplate = `",
      "```",
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate truncates multi-file grep collections as one blob", () => {
  const output = [
    "Found 24 matches",
    "/Users/miguel/arroba/apps/cli/src/index.tsx:",
    ...Array.from({ length: 12 }, (_, index) => `  Line ${index + 1}: index line ${index + 1}`),
    "/Users/miguel/arroba/apps/cli/src/transcript.ts:",
    ...Array.from({ length: 12 }, (_, index) => `  Line ${index + 20}: transcript line ${index + 20}`),
  ].join("\n")

  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-9b",
      tool: "grep",
      status: "completed",
      input: {
        pattern: "line",
        path: "/Users/miguel/arroba/apps/cli/src",
      },
      output,
    }),
    [
      "**grep** · completed",
      "Pattern: `line` (24 matches in 2 files)",
      [
        "`index.tsx`",
        "```typescriptreact",
        ...Array.from({ length: 7 }, (_, index) => `Line ${index + 1}: index line ${index + 1}`),
        "```",
      ].join("\n"),
      "...",
      [
        "`transcript.ts`",
        "```typescript",
        ...Array.from({ length: 7 }, (_, index) => `Line ${index + 25}: transcript line ${index + 25}`),
        "```",
      ].join("\n"),
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate uses webfetch format for syntax highlighting", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-10",
      tool: "webfetch",
      status: "completed",
      input: {
        url: "https://example.com",
        format: "markdown",
      },
      output: "# Example\n\n```ts\nconst value = 1\n```",
    }),
    [
      "**webfetch** · completed",
      '**Input**\n```json\n{\n  "url": "https://example.com",\n  "format": "markdown"\n}\n```',
      "**Output**\n````markdown\n# Example\n\n```ts\nconst value = 1\n```\n````",
    ].join("\n\n"),
  )
})

test("formatToolTranscriptUpdate infers file rendering generically from tool inputs", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-10b",
      tool: "read_file_like",
      status: "completed",
      input: {
        filePath: "apps/cli/src/index.tsx",
      },
      output: "1: export function App() {\n2:   return null\n3: }",
    }),
    [
      "**read_file_like** · completed",
      '**Input**\n```json\n{\n  "filePath": "apps/cli/src/index.tsx"\n}\n```',
      [
        "**Output**",
        "`apps/cli/src/index.tsx`",
        "```typescriptreact",
        "1: export function App() {",
        "2:   return null",
        "3: }",
        "```",
      ].join("\n"),
    ].join("\n\n"),
  )
})

test("formatToolTranscriptUpdate infers embedded file payloads generically", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-10c",
      tool: "custom",
      status: "completed",
      output: "<path>/Users/miguel/arroba/apps/cli/src/transcript.ts</path>\n<type>file</type>\n<content>1: export const value = 1\n</content>",
    }),
    [
      "**custom** · completed",
      [
        "**Output**",
        "`/Users/miguel/arroba/apps/cli/src/transcript.ts`",
        "```typescript",
        "1: export const value = 1",
        "```",
      ].join("\n"),
    ].join("\n\n"),
  )
})

test("formatToolTranscriptUpdate truncates large generic blobs in the middle", () => {
  const output = Array.from({ length: 24 }, (_, index) => `line ${index + 1}`).join("\n")

  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-10d",
      tool: "bash",
      status: "completed",
      description: "Shows long output",
      output,
    }),
    [
      "**bash** · completed",
      "Shows long output",
      [
        "**Output**",
        "```text",
        ...Array.from({ length: 10 }, (_, index) => `line ${index + 1}`),
        "...",
        ...Array.from({ length: 10 }, (_, index) => `line ${index + 15}`),
        "```",
      ].join("\n"),
    ].join("\n\n"),
  )
})

test("readApplyPatchFiles parses update and add operations into diffs", () => {
  const files = readApplyPatchFiles({
    id: "tool-11",
    tool: "apply_patch",
    input: {
      patchText: [
        "*** Begin Patch",
        "*** Update File: src/app.ts",
        "@@",
        "-const oldValue = 1",
        "+const newValue = 2",
        "*** Add File: src/new.ts",
        "+export const value = 1",
        "*** End Patch",
      ].join("\n"),
    },
  })

  assert.deepEqual(files, [
    {
      kind: "update",
      filePath: "src/app.ts",
      title: "Patched src/app.ts",
      diff: [
        "diff --git a/src/app.ts b/src/app.ts",
        "--- a/src/app.ts",
        "+++ b/src/app.ts",
        "@@ -1,1 +1,1 @@",
        "-const oldValue = 1",
        "+const newValue = 2",
      ].join("\n"),
    },
    {
      kind: "add",
      filePath: "src/new.ts",
      title: "Created src/new.ts",
      diff: [
        "diff --git a/src/new.ts b/src/new.ts",
        "new file mode 100644",
        "--- /dev/null",
        "+++ b/src/new.ts",
        "@@ -0,0 +1,1 @@",
        "+export const value = 1",
      ].join("\n"),
    },
  ])
})

test("readApplyPatchFiles prefixes apply_patch context lines for diff rendering", () => {
  const [file] = readApplyPatchFiles({
    id: "tool-13",
    tool: "apply_patch",
    input: {
      patchText: [
        "*** Begin Patch",
        "*** Update File: src/index.tsx",
        "@@",
        " function greet() {",
        "-  return 'old'",
        "+  return 'new'",
        " }",
        "*** End Patch",
      ].join("\n"),
    },
  })

  assert.equal(
    file?.diff,
    [
      "diff --git a/src/index.tsx b/src/index.tsx",
      "--- a/src/index.tsx",
      "+++ b/src/index.tsx",
      "@@ -1,3 +1,3 @@",
      " function greet() {",
      "-  return 'old'",
      "+  return 'new'",
      " }",
    ].join("\n"),
  )
})

test("readApplyPatchFiles normalizes absolute paths and bare hunk markers", () => {
  const [file] = readApplyPatchFiles({
    id: "tool-14",
    tool: "apply_patch",
    input: {
      patchText: [
        "*** Begin Patch",
        "*** Update File: /Users/miguel/arroba/apps/cli/src/transcript.test.ts",
        "@@",
        " test(\"a\", () => {",
        "-  old()",
        "+  next()",
        " })",
        "*** End Patch",
      ].join("\n"),
    },
  })

  assert.equal(
    file?.diff,
    [
      "diff --git a/Users/miguel/arroba/apps/cli/src/transcript.test.ts b/Users/miguel/arroba/apps/cli/src/transcript.test.ts",
      "--- a/Users/miguel/arroba/apps/cli/src/transcript.test.ts",
      "+++ b/Users/miguel/arroba/apps/cli/src/transcript.test.ts",
      "@@ -1,3 +1,3 @@",
      ' test("a", () => {',
      "-  old()",
      "+  next()",
      " })",
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate summarizes apply_patch changes", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-12",
      tool: "apply_patch",
      status: "completed",
      input: {
        patchText: [
          "*** Begin Patch",
          "*** Update File: src/app.ts",
          "@@",
          "-const oldValue = 1",
          "+const newValue = 2",
          "*** Delete File: src/old.ts",
          "*** End Patch",
        ].join("\n"),
      },
    }),
    [
      "**patch** · completed",
      "2 files · 1 updated, 1 deleted",
      "- Patched src/app.ts",
      "- Deleted src/old.ts",
    ].join("\n"),
  )
})

test("mergeToolTranscriptUpdate keeps prior tool details across partial updates", () => {
  const merged = mergeToolTranscriptUpdate(
    {
      id: "tool-1",
      tool: "bash",
      status: "running",
      input: { command: "git status" },
      description: "Shows working tree status",
    },
    {
      id: "tool-1",
      status: "completed",
      output: "On branch main",
    },
  )

  assert.deepEqual(merged, {
    id: "tool-1",
    tool: "bash",
    status: "completed",
    input: { command: "git status" },
    description: "Shows working tree status",
    output: "On branch main",
  })
  assert.equal(
    formatToolTranscriptUpdate(merged),
    [
      "**bash** · completed",
      "Shows working tree status",
      "**Command**\n```bash\n$ git status\n```",
      "**Output**\n```text\nOn branch main\n```",
    ].join("\n\n"),
  )
})

test("shouldRenderProviderStatus suppresses idle notices only", () => {
  assert.equal(shouldRenderProviderStatus("OpenCode is idle."), false)
  assert.equal(shouldRenderProviderStatus("OpenCode is thinking..."), true)
})

test("splitInlineCodeSpans marks inline code runs", () => {
  assert.deepEqual(splitInlineCodeSpans("Run `git status` and `git diff`."), [
    { text: "Run ", code: false },
    { text: "git status", code: true },
    { text: " and ", code: false },
    { text: "git diff", code: true },
    { text: ".", code: false },
  ])
})

test("splitInlineCodeSpans leaves unmatched backticks as plain text", () => {
  assert.deepEqual(splitInlineCodeSpans("Use `unfinished inline code"), [
    { text: "Use `unfinished inline code", code: false },
  ])
})

test("normalizeMarkdownFenceInfoStrings expands common code fence aliases", () => {
  assert.equal(
    normalizeMarkdownFenceInfoStrings("```ts\nconst value = 1\n```\n```sh\necho hi\n```"),
    "```typescript\nconst value = 1\n```\n```shellscript\necho hi\n```",
  )
})

test("normalizeMarkdownFenceInfoStrings expands react and infra aliases", () => {
  assert.equal(
    normalizeMarkdownFenceInfoStrings("```tsx\nexport const App = () => null\n```\n```tf\nresource \"x\" \"y\" {}\n```"),
    "```typescriptreact\nexport const App = () => null\n```\n```terraform\nresource \"x\" \"y\" {}\n```",
  )
})

test("guessPathFenceLanguage covers major file types", () => {
  assert.equal(guessPathFenceLanguage("src/app.tsx"), "typescriptreact")
  assert.equal(guessPathFenceLanguage("src/app.jsx"), "javascriptreact")
  assert.equal(guessPathFenceLanguage("src/main.rs"), "rust")
  assert.equal(guessPathFenceLanguage("src/server.go"), "go")
  assert.equal(guessPathFenceLanguage("src/index.py"), "python")
  assert.equal(guessPathFenceLanguage("src/template.html.erb"), "erb")
  assert.equal(guessPathFenceLanguage("infra/main.tf"), "terraform")
  assert.equal(guessPathFenceLanguage("Dockerfile"), "dockerfile")
  assert.equal(guessPathFenceLanguage("Makefile"), "makefile")
  assert.equal(guessPathFenceLanguage(".env"), "dotenv")
  assert.equal(guessPathFenceLanguage("notes/unknown.custom"), "text")
})

test("shouldRenderTranscriptAsMarkdown enables markdown rendering for assistant code blocks", () => {
  assert.equal(shouldRenderTranscriptAsMarkdown("assistant", "Here is code:\n```ts\nconst value = 1\n```"), true)
  assert.equal(shouldRenderTranscriptAsMarkdown("reasoning", "## Plan\n- step one"), true)
  assert.equal(shouldRenderTranscriptAsMarkdown("assistant", "A plain paragraph with **bold** text"), true)
  assert.equal(shouldRenderTranscriptAsMarkdown("tool", "```ts\nconst value = 1\n```"), true)
  assert.equal(shouldRenderTranscriptAsMarkdown("error", "**OpenCode error**\n\nnetwork timeout"), true)
  assert.equal(shouldRenderTranscriptAsMarkdown("assistant", "plain text only"), true)
})
