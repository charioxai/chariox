import assert from "node:assert/strict"
import { readdirSync, readFileSync } from "node:fs"
import test from "node:test"

import { formatToolDisplay, type ToolDisplayBlock, type ToolTranscriptUpdate } from "./index.js"

// Owner, Landing 8: a tool row that has a chevron must never open to nothing, whatever the provider and the kind of call.
// Every fixture in fixtures/records is a tool record the kernel produced in a live turn of the real provider (named in
// its `source`); the test opens each one. A row with no content has no chevron, and the fixtures that expect none say so.
type Fixture = { provider: string; kind: string; source: string; record: ToolTranscriptUpdate; shows?: string; empty?: true }

const directory = new URL("../fixtures/records/", import.meta.url)
const fixtures = readdirSync(directory).filter((name) => name.endsWith(".json")).sort().map((name) => ({ name, ...JSON.parse(readFileSync(new URL(name, directory), "utf8")) as Fixture }))

export const textOf = (blocks: readonly ToolDisplayBlock[]) => blocks.map((block) => {
  if (block.kind === "text" || block.kind === "code") return block.text
  if (block.kind === "table") return [block.columns.join(" "), ...block.rows.map((row: string[]) => row.join(" "))].join("\n")
  if (block.kind === "json") return JSON.stringify(block.value)
  if (block.kind === "patch") return block.files.map((file: { filePath: string; diff: string | null; note?: string }) => `${file.filePath}\n${file.diff ?? file.note ?? ""}`).join("\n")
  return ""
}).join("\n")

test("there is a live record for every provider", () => {
  for (const provider of ["codex", "claude", "opencode"]) {
    assert.ok(fixtures.some((fixture) => fixture.provider === provider), `a ${provider} fixture`)
  }
})

for (const fixture of fixtures) {
  test(`an opened ${fixture.provider} ${fixture.kind} row is never empty: ${fixture.name}`, () => {
    const display = formatToolDisplay(fixture.record)
    const shown = textOf(display.blocks)
    if (fixture.empty) {
      assert.equal(display.blocks.length, 0, "a call with nothing to show has no content, so no chevron")
      return
    }
    assert.ok(display.blocks.length > 0 && shown.trim().length > 0, `"${display.title}" opens to something`)
    if (fixture.shows) assert.match(shown, new RegExp(fixture.shows), "shows the call's own content")
  })
}

// Kinds no live turn of this round produced, in the shapes the providers document. Not live records.
const completed = (tool: string, input: unknown, extra: Partial<ToolTranscriptUpdate> = {}): ToolTranscriptUpdate => ({ id: "call-1", tool, status: "completed", input, ...extra })
const shapes: Record<string, { update: ToolTranscriptUpdate; shows: RegExp }> = {
  "Codex web search that opened a page": { update: completed("web_search", { url: "https://example.com/tickets" }), shows: /example\.com\/tickets/ },
  "Codex web search that looked inside a page": { update: completed("web_search", { url: "https://example.com/tickets", pattern: "prices" }), shows: /Find: prices/ },
  "Codex web search with several queries": { update: completed("web_search", { query: "alpha", queries: ["alpha", "beta"] }), shows: /beta/ },
  "Claude WebSearch, result list": { update: completed("WebSearch", { query: "alhambra" }, { output: 'Web search results for query: "alhambra"\n\nLinks: [{"title":"Alhambra","url":"https://example.com/a"}]\n\nThe Alhambra is a palace.' }), shows: /Alhambra\s+https:\/\/example\.com\/a[\s\S]*palace/ },
  "OpenCode websearch, result list": { update: completed("websearch", { query: "alhambra" }, { output: JSON.stringify({ search_id: "s", results: [{ url: "https://example.com/a", title: "Alhambra", excerpts: ["a palace"] }, { url: "https://example.com/g", title: "Granada", excerpts: ["a city"] }] }, null, 2) }), shows: /Granada\s+https:\/\/example\.com\/g/ },
  "OpenCode websearch whose result list the kernel cut short": { update: completed("websearch", { query: "alhambra" }, { output: '{\n  "search_id": "s",\n  "results": [\n    {\n      "url": "https://example.com/a",\n      "title": "Alhambra",\n      "excerpts": ["a pal\n\n[chariox: output truncated, omitted 900 bytes]' }), shows: /Alhambra\s+https:\/\/example\.com\/a/ },
  "web fetch": { update: completed("WebFetch", { url: "https://example.com/a", prompt: "summarise" }, { output: "A page" }), shows: /example\.com\/a[\s\S]*A page/ },
  "MCP call with arguments and a result": { update: completed("mcp__docs__lookup", { topic: "relay" }, { output: '{"hits":2}' }), shows: /Input[\s\S]*relay[\s\S]*Output[\s\S]*hits/ },
  "MCP call with arguments and no result yet": { update: { id: "c", tool: "mcp__docs__lookup", status: "running", input: { topic: "relay" } }, shows: /relay/ },
  "Chariox tool with no arguments and a result": { update: completed("chariox.list_session_agents", {}, { output: '[{"alias":"a"}]' }), shows: /alias/ },
  "a tool no one wrote a view for": { update: completed("future_kind_9", { a: 1 }, { output: "result text" }), shows: /"a": 1[\s\S]*result text/ },
  "a tool that only has raw details": { update: completed("future_kind_9", undefined, { raw: '{"detail":true}' }), shows: /detail/ },
  "a failed call shows its error": { update: { id: "c", tool: "future_kind_9", status: "error", error: "it broke" }, shows: /it broke/ },
  "Claude TodoWrite": { update: completed("TodoWrite", { todos: [{ content: "write tests", status: "completed" }, { content: "ship", status: "pending" }] }, { output: "Todos have been modified" }), shows: /\[x\] write tests[\s\S]*\[ \] ship/ },
  "Claude Task": { update: completed("Task", { description: "Explore", prompt: "Look at the relay code", subagent_type: "Explore" }, { output: "Found it" }), shows: /Look at the relay code[\s\S]*Found it/ },
  "OpenCode todowrite": { update: completed("todowrite", { todos: [{ content: "one", status: "pending" }] }), shows: /one/ },
  "Claude Grep with its result": { update: completed("Grep", { pattern: "foo" }, { output: "a.ts:1:foo" }), shows: /a\.ts/ },
  "Claude Glob with no result": { update: completed("Glob", { pattern: "**/*.ts" }), shows: /\*\*\/\*\.ts/ },
  "Claude LS": { update: completed("LS", { path: "/w" }, { output: "- /w/a.ts" }), shows: /a\.ts/ },
  "image view": { update: completed("view_image", { path: "/w/chart.png" }), shows: /chart\.png/ },
}
for (const [name, { update, shows }] of Object.entries(shapes)) {
  test(`an opened row is never empty: ${name}`, () => {
    const shown = textOf(formatToolDisplay(update).blocks)
    assert.ok(shown.trim().length > 0, "opens to something")
    assert.match(shown, shows)
  })
}

test("a call with nothing to show has no content and so no chevron", () => {
  for (const update of [completed("future_kind_9", undefined), completed("web_search", undefined, { status: "running" }), completed("Bash", {}), { id: "c", tool: "mcp__x__y", status: "cancelled" }] as ToolTranscriptUpdate[]) {
    assert.equal(formatToolDisplay(update).blocks.length, 0, update.tool)
  }
})

test("a web search result list is a table of titles and addresses", () => {
  const { blocks } = formatToolDisplay(shapes["Claude WebSearch, result list"]!.update)
  const table = blocks.find((block: ToolDisplayBlock) => block.kind === "table")
  assert.ok(table && table.kind === "table" && table.rows[0]![1] === "https://example.com/a")
})

// Review of #1326: OpenCode's web search gives excerpts, publication dates and (when long) a truncation notice; the opened row keeps them.
test("an OpenCode web search row keeps the excerpts, the publication dates and the truncation notice (LIVE record)", () => {
  const live = fixtures.find((fixture) => fixture.name === "opencode-websearch-completed-3.json")!
  const shown = textOf(formatToolDisplay(live.record).blocks)
  assert.match(shown, /Published/)
  assert.match(shown, /2025-02-17/)
  assert.match(shown, /2016-07-06/)
  assert.match(shown, /Declarada Patrimonio Mundial por la Unesco desde 1984/, "the first excerpt")
  assert.match(shown, /The Albayzin quarter is an indispensable complement/, "an excerpt further down")
  assert.match(shown, /output truncated, omitted 14181 bytes/, "the truncation notice")
})

test("an OpenCode web search with whole JSON keeps excerpts and dates too (FIXTURE)", () => {
  const output = JSON.stringify({ search_id: "s", results: [{ url: "https://example.com/a", title: "Alhambra", publish_date: "2024-01-02", excerpts: ["a palace", "built by the Nasrids"] }, { url: "https://example.com/g", title: "Granada", excerpts: [] }] }, null, 2)
  const blocks = formatToolDisplay({ id: "c", tool: "websearch", status: "completed", input: { query: "alhambra" }, output }).blocks
  const shown = textOf(blocks)
  assert.match(shown, /2024-01-02/)
  assert.match(shown, /a palace[\s\S]*built by the Nasrids/)
  assert.match(shown, /Granada\s+https:\/\/example\.com\/g/)
})
