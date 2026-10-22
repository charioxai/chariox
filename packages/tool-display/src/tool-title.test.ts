import assert from "node:assert/strict"
import test from "node:test"

import { TOOL_DISPLAY_ICON_GLYPHS, formatToolDisplay, toolDisplayIcon, toolDisplayTitle } from "./index.js"

// MP-08 / MP-10 (panes R2, PANE-02): titles are verb + object, never a raw
// underscore function name, and identical for every client.
test("tool titles use a verb and an object for each provider's tool shapes", () => {
  const cases: Array<[Parameters<typeof toolDisplayTitle>[0], string]> = [
    // Codex app-server
    [{ id: "1", tool: "bash", status: "completed", input: { command: "/bin/bash -lc 'git status --short'" } }, "Ran git status --short"],
    [{ id: "2", tool: "bash", status: "running", input: { command: "/bin/bash -lc 'cargo test -p kernel'" } }, "Running cargo test -p kernel"],
    [{ id: "3", tool: "bash", status: "completed", input: { command: "/bin/bash -lc 'cat README.md'" } }, "Read README.md"],
    [{ id: "4", tool: "bash", status: "completed", input: { command: "/bin/bash -lc 'ls -la'" } }, "Listed files"],
    [{ id: "5", tool: "bash", status: "completed", input: { command: "/bin/bash -lc 'rg -n toolDisplay packages'" } }, "Searched for toolDisplay in packages"],
    [{ id: "6", tool: "bash", status: "completed", input: { command: "/bin/bash -lc 'echo hi > /tmp/a.txt && cat /tmp/a.txt'" } }, "Ran echo hi > /tmp/a.txt && cat /tmp/a.txt"],
    // MP-08 / MP-10 (panes R2 follow-up): option values are not the pattern; double-quoted scripts unescape.
    [{ id: "6b", tool: "bash", status: "completed", input: { command: "/bin/bash -lc \"rg --hidden -n -w --max-count 3 --glob '!.git/**' 'pane' .\"" } }, "Searched for pane in ."],
    [{ id: "6c", tool: "bash", status: "completed", input: { command: "/bin/bash -lc \"printf 'hello\\\\n' > /tmp/x\"" } }, "Ran printf 'hello\\n' > /tmp/x"],
    [{ id: "7", tool: "search_docs", title: "context7", status: "completed", input: { q: "x" } }, "Called context7.search docs"],
    // Claude Code
    [{ id: "c1", tool: "Bash", status: "running", input: { command: "npm test", description: "Run tests" } }, "Running npm test"],
    [{ id: "c2", tool: "Read", status: "completed", input: { file_path: "/repo/src/app.ts" } }, "Read /repo/src/app.ts"],
    [{ id: "c3", tool: "Edit", status: "completed", input: { file_path: "src/app.ts", old_string: "a", new_string: "b" } }, "Edited src/app.ts"],
    [{ id: "c4", tool: "Write", status: "completed", input: { file_path: "notes.md", content: "x" } }, "Wrote notes.md"],
    [{ id: "c5", tool: "Grep", status: "completed", input: { pattern: "TODO", path: "src" } }, "Searched for TODO in src"],
    [{ id: "c6", tool: "Glob", status: "completed", input: { pattern: "**/*.ts" } }, "Searched files matching **/*.ts"],
    [{ id: "c7", tool: "WebFetch", status: "completed", input: { url: "https://example.com", prompt: "x" } }, "Fetched https://example.com"],
    [{ id: "c8", tool: "WebSearch", status: "completed", input: { query: "chariox" } }, "Searched the web for chariox"],
    [{ id: "c9", tool: "TodoWrite", status: "completed", input: { todos: [] } }, "Updated plan"],
    [{ id: "c10", tool: "mcp__github__create_pull_request", status: "completed", input: {} }, "Called github.create pull request"],
    [{ id: "c11", tool: "Task", status: "running", input: { description: "Audit the pane", prompt: "..." } }, "Delegating Audit the pane"],
    // OpenCode
    [{ id: "o1", tool: "read", status: "completed", input: { filePath: "/w/README.md" } }, "Read /w/README.md"],
    [{ id: "o2", tool: "edit", status: "running", input: { filePath: "a.ts", oldString: "x", newString: "y" } }, "Editing a.ts"],
    [{ id: "o3", tool: "list", status: "completed", input: { path: "/w" } }, "Listed /w"],
    [{ id: "o4", tool: "webfetch", status: "completed", input: { url: "https://developer.mozilla.org", format: "markdown" } }, "Fetched https://developer.mozilla.org"],
    [{ id: "o5", tool: "todowrite", status: "completed", input: { todos: [] } }, "Updated plan"],
    [{ id: "o6", tool: "some_custom_tool", status: "completed" }, "Called some custom tool"],
  ]
  const formattedTitles: Record<string, string> = {
    c3: "Edited src/app.ts (+1 −1)", c4: "Edited notes.md (+1 −0)",
  }
  for (const [update, expected] of cases) {
    const title = toolDisplayTitle(update)
    assert.equal(title, expected, JSON.stringify(update))
    assert.equal(formatToolDisplay(update).title, formattedTitles[update.id] ?? expected)
    assert.doesNotMatch(title, /_/)
  }
})

test("patch tool titles name the file and the line counts", () => {
  const display = formatToolDisplay({
    id: "p1",
    tool: "apply_patch",
    status: "completed",
    raw: JSON.stringify([{ path: "src/app.ts", kind: "update", diff: "@@ -1,2 +1,2 @@\n-old()\n+next()\n+more()" }]),
  })
  assert.equal(display.title, "Edited src/app.ts (+2 −1)")
})

// MP-08 / MP-10 (panes, Miguel 2026-10-12 addition A): every tool row has an
// icon left of its title, one per tool kind, shared by web and TUI.
test("tool icons follow the tool kind for each provider's tool shapes", () => {
  const cases: Array<[Parameters<typeof toolDisplayIcon>[0], ReturnType<typeof toolDisplayIcon>]> = [
    [{ id: "1", tool: "exec_command", input: { cmd: "/bin/bash -lc 'pnpm test'" } }, "terminal"],
    [{ id: "2", tool: "bash", input: { command: "/bin/bash -lc 'cat README.md'" } }, "read"],
    [{ id: "3", tool: "bash", input: { command: "rg -n pane src" } }, "search"],
    [{ id: "4", tool: "Read", input: { file_path: "/w/a.ts" } }, "read"],
    [{ id: "5", tool: "read", title: "screenshot.png", input: { filePath: "/w/screenshot.png" } }, "read"],
    [{ id: "6", tool: "edit", input: { filePath: "a.ts", oldString: "x", newString: "y" } }, "edit"],
    [{ id: "7", tool: "Write", input: { file_path: "notes.md", content: "x" } }, "edit"],
    [{ id: "8", tool: "Grep", input: { pattern: "TODO" } }, "search"],
    [{ id: "9", tool: "webfetch", input: { url: "https://example.com" } }, "web"],
    [{ id: "10", tool: "mcp__playwright__browser_navigate", input: { url: "https://example.com" } }, "browser"],
    [{ id: "11", tool: "mcp__context7__search_docs", input: { q: "x" } }, "mcp"],
    [{ id: "12", tool: "search_docs", title: "context7", input: { q: "x" } }, "mcp"],
    [{ id: "13", tool: "Task", input: { description: "Audit" } }, "agent"],
    [{ id: "14", tool: "todowrite", input: { todos: [] } }, "plan"],
    [{ id: "15", tool: "mystery" }, "tool"],
    [{ id: "16", tool: "chariox_chariox_list_extensions", input: {} }, "mcp"],
    [{ id: "17", tool: "chariox.chariox.list_extensions", input: {} }, "mcp"],
    [{ id: "18", tool: "charioxify" }, "tool"],
  ]
  for (const [update, icon] of cases) {
    assert.equal(toolDisplayIcon(update), icon, `${update.tool}`)
    assert.equal(formatToolDisplay(update).icon, icon)
  }
  for (const glyph of Object.values(TOOL_DISPLAY_ICON_GLYPHS)) {
    assert.equal([...glyph].length, 1)
  }
})

// MP-08 / MP-10 (panes Phase-1 drill): OpenCode's workspace-relative title
// ("../../tmp/a.txt") does not repeat the file the row title already names.
test("file tool summaries skip the provider's relative copy of the path", () => {
  const display = formatToolDisplay({
    id: "w1",
    tool: "write",
    status: "completed",
    title: "../../../../tmp/a.txt",
    input: { filePath: "/tmp/a.txt", content: "x" },
    output: "Wrote file successfully.",
  })
  assert.equal(display.title, "Edited /tmp/a.txt (+1 −0)")
  assert.doesNotMatch(display.summary, /\.\.\//)
  assert.equal(formatToolDisplay({ id: "m1", tool: "search_docs", title: "context7", input: { q: "x" } }).summary, "context7")
})

test("MP-08 / MP-10 shell titles keep explicit search patterns and ordinary listing flags", () => {
  for (const [command, expected] of [
    ["rg -e TODO src", "Searched for TODO in src"],
    ["grep --regexp TODO src", "Searched for TODO in src"],
    ["rg --regexp=TODO src", "Searched for TODO in src"],
    ["ls -A src", "Listed src"],
    ["ls -t src", "Listed src"],
    ["cat -A README.md", "Read README.md"],
  ]) assert.equal(toolDisplayTitle({id:"review-search",tool:"bash",status:"completed",input:{command:command!}}), expected)
})

// MP-08 / MP-10: -f supplies a pattern file; following operands remain paths.
test("shell search titles preserve pattern-file operands", () => {
  for (const command of ["rg -f patterns.txt src", "grep --file patterns.txt src", "rg --file=patterns.txt src"]) {
    assert.equal(toolDisplayTitle({ id: "patterns", tool: "bash", status: "completed", input: { command } }), "Searched using patterns from patterns.txt in src")
  }
})
