import { compactToolName } from "./tool-names.js"
import { isObjectValue, normalizeJsonLike } from "./strings.js"
import type { ToolDisplayBlock, ToolTranscriptUpdate } from "./types.js"

// What a tool row opens to when no kind-specific view (a command, a file, a diff) claims the call: a row with a chevron
// must never open to nothing. A web search shows its query and result list, a fetch its address, a delegated task its prompt,
// a plan its list, and any other call (an MCP tool, a tool no one wrote a view for) its arguments, then what came back.

const SELF_PRESENTING = new Set([
  "bash", "shell", "sh", "shellcommand", "execcommand", "runcommand", "runshellcommand", "localshell", "terminal", "commandexecution", "exec", "unifiedexec",
  "read", "readfile", "view", "charioxreadartifact", "readartifact",
  "edit", "multiedit", "strreplace", "strreplacebasededittool", "notebookedit", "applypatch", "patch", "write", "writefile", "create",
  "charioxeditartifact", "charioxapplypatch", "charioxpatchartifact", "editartifact", "patchartifact",
  "charioxwriteartifact", "writeartifact", "charioxdeleteartifact", "deleteartifact", "charioxmoveartifact", "moveartifact",
  "grep", "search", "rg", "codebasesearch", "searchfiles", "glob", "find", "findfiles", "ls", "list", "listdir", "listfiles", "listdirectory",
])
const WEB_SEARCH = new Set(["websearch", "webquery", "codesearch"])
const WEB_FETCH = new Set(["webfetch", "fetch", "fetchurl"])
const TODO = new Set(["todowrite", "updateplan", "todo", "todoread"])
const DELEGATION = new Set(["task", "agent", "spawnagent"])

type ToolContentKind = "plain" | "web_search" | "web_fetch" | "todo" | "delegation" | "other"

function contentKind(tool: unknown): ToolContentKind {
  const name = compactToolName(tool)
  if (WEB_SEARCH.has(name)) return "web_search"
  if (WEB_FETCH.has(name)) return "web_fetch"
  if (TODO.has(name)) return "todo"
  if (DELEGATION.has(name)) return "delegation"
  return SELF_PRESENTING.has(name) ? "plain" : "other"
}

const textBlock = (value: string): ToolDisplayBlock => ({ kind: "code", language: "text", text: value })
const labelBlock = (value: string): ToolDisplayBlock => ({ kind: "text", text: `**${value}**` })

function stringsOf(...values: unknown[]): string[] {
  const found: string[] = []
  for (const value of values) {
    for (const item of Array.isArray(value) ? value : [value]) {
      if (typeof item === "string" && item.trim() && !found.includes(item.trim())) found.push(item.trim())
    }
  }
  return found
}

function prettyInput(input: unknown): string | null {
  const value = normalizeJsonLike(input)
  if (value == null) return null
  if (typeof value === "string") return value.trim() ? value : null
  if (Array.isArray(value)) return value.length > 0 ? JSON.stringify(value, null, 2) : null
  if (isObjectValue(value)) return Object.keys(value).length > 0 ? JSON.stringify(value, null, 2) : null
  return String(value)
}

/** The call's own words about itself, shown before what came back. `labelled`: both input and output are generic text, so each is named. */
export function toolInputBlocks(update: ToolTranscriptUpdate, hasResult: boolean): { blocks: ToolDisplayBlock[]; labelled: boolean } {
  const kind = contentKind(update.tool)
  const input = isObjectValue(normalizeJsonLike(update.input)) ? normalizeJsonLike(update.input) as Record<string, unknown> : null
  if (kind === "web_search") {
    const queries = stringsOf(input?.query, input?.queries, input?.q)
    const url = stringsOf(input?.url)[0]
    const pattern = stringsOf(input?.pattern)[0]
    const lines = [...queries, ...url ? [url] : [], ...pattern ? [`Find: ${pattern}`] : []]
    return { blocks: lines.length > 0 ? [textBlock(lines.join("\n"))] : [], labelled: false }
  }
  if (kind === "web_fetch") {
    const url = stringsOf(input?.url, input?.uri)[0]
    return { blocks: url ? [textBlock(url)] : [], labelled: false }
  }
  if (kind === "todo") {
    const list = todoLines(update.input)
    return { blocks: list ? [{ kind: "text", text: list }] : [], labelled: false }
  }
  if (kind === "delegation") {
    const prompt = stringsOf(input?.prompt)[0]
    return { blocks: prompt ? [{ kind: "text", text: prompt }] : [], labelled: false }
  }
  // A plain call's title carries its arguments and its output is the content; it shows its input only when nothing else would.
  if (kind === "plain" && hasResult) return { blocks: [], labelled: false }
  const pretty = prettyInput(update.input)
  if (!pretty) return { blocks: [], labelled: false }
  return { blocks: hasResult ? [labelBlock("Input"), { kind: "code", language: "json", text: pretty }] : [{ kind: "code", language: "json", text: pretty }], labelled: hasResult }
}

export function toolOutputLabel(): ToolDisplayBlock {
  return labelBlock("Output")
}

function todoLines(value: unknown): string | null {
  const parsed = normalizeJsonLike(value)
  const todos = Array.isArray(parsed) ? parsed : isObjectValue(parsed) && Array.isArray(parsed.todos) ? parsed.todos : isObjectValue(parsed) && Array.isArray(parsed.plan) ? parsed.plan : null
  if (!todos) return null
  const lines = todos.flatMap((item): string[] => {
    if (!isObjectValue(item)) return []
    const content = [item.content, item.step, item.text].find((entry): entry is string => typeof entry === "string" && entry.trim() !== "")?.trim()
    if (!content) return []
    const status = typeof item.status === "string" ? item.status : ""
    return [`${status === "completed" ? "- [x]" : status === "cancelled" ? "- [-]" : "- [ ]"} ${content}`]
  })
  return lines.length > 0 ? lines.join("\n") : null
}

type SearchResult = { readonly title: string; readonly url: string; readonly published?: string; readonly excerpts: readonly string[] }

const decoded = (quoted: string) => {
  try {
    return JSON.parse(`"${quoted}"`) as string
  } catch {
    return quoted
  }
}

const TRUNCATION_NOTICE = /\[chariox: output truncated[^\]]*\]/

// A result the kernel cut short: its excerpts are the quoted strings after `"excerpts": [`, the last one possibly unterminated.
function cutExcerpts(segment: string): string[] {
  const start = segment.search(/"excerpts":\s*\[/)
  if (start < 0) return []
  const strings: string[] = []
  const quoted = /"((?:[^"\\]|\\.)*)("?)/g
  quoted.lastIndex = segment.indexOf("[", start) + 1
  for (let match = quoted.exec(segment); match && match[1] !== undefined; match = quoted.exec(segment)) {
    strings.push(decoded(match[1]))
    if (!match[2]) break
  }
  return strings
}

// Claude: `Links: [{"title":..,"url":..}, ...]` on its own line, around a written summary. OpenCode: JSON `{"results":[{"url","title","publish_date","excerpts"}]}`,
// which the kernel may have cut short, so a cut list is read entry by entry. The kernel's truncation notice is kept.
function parseSearchResults(output: string): { results: SearchResult[]; rest: string; notice: string | null } | null {
  const entries = (value: unknown): SearchResult[] => Array.isArray(value) ? value.flatMap((entry): SearchResult[] => isObjectValue(entry) && typeof entry.url === "string" ? [{
    title: typeof entry.title === "string" && entry.title.trim() ? entry.title : entry.url,
    url: entry.url,
    ...typeof entry.publish_date === "string" && entry.publish_date.trim() ? { published: entry.publish_date } : {},
    excerpts: Array.isArray(entry.excerpts) ? entry.excerpts.filter((excerpt): excerpt is string => typeof excerpt === "string" && excerpt.trim() !== "") : [],
  }] : []) : []
  const links = output.match(/^Links:\s*(\[.*\])\s*$/m)
  if (links) {
    try {
      const results = entries(JSON.parse(links[1]!))
      if (results.length > 0) return { results, rest: output.replace(links[0], "").replace(/\n{3,}/g, "\n\n").trim(), notice: null }
    } catch {
      // Not the list shape: the raw text is shown.
    }
  }
  const notice = output.match(TRUNCATION_NOTICE)?.[0] ?? null
  const body = notice ? output.replace(notice, "").trimEnd() : output
  const json = normalizeJsonLike(body)
  const listed = isObjectValue(json) ? entries(json.results) : []
  if (listed.length > 0) return { results: listed, rest: "", notice }
  if (body.includes('"results"')) {
    const found: SearchResult[] = []
    const pair = /"url":\s*"((?:[^"\\]|\\.)*)",\s*"title":\s*"((?:[^"\\]|\\.)*)"/g
    const matches = [...body.matchAll(pair)]
    matches.forEach((match, index) => {
      const segment = body.slice(match.index! + match[0].length, matches[index + 1]?.index ?? body.length)
      const published = segment.match(/"publish_date":\s*"((?:[^"\\]|\\.)*)"/)?.[1]
      found.push({ title: decoded(match[2]!), url: decoded(match[1]!), ...published ? { published: decoded(published) } : {}, excerpts: cutExcerpts(segment).filter((excerpt) => excerpt.trim() !== "") })
    })
    if (found.length > 0) return { results: found, rest: "", notice }
  }
  return null
}

/** A web search's result list as a table of titles and addresses (and the written summary around it); null when the output is not a list. */
export function searchResultBlocks(update: ToolTranscriptUpdate, output: string): ToolDisplayBlock[] | null {
  if (contentKind(update.tool) !== "web_search") return null
  const parsed = parseSearchResults(output)
  if (!parsed) return null
  const dated = parsed.results.some((result) => result.published)
  const excerpts = parsed.results.filter((result) => result.excerpts.length > 0).map((result) => [`**${result.title}**${result.published ? ` · ${result.published}` : ""}`, ...result.excerpts].join("\n\n"))
  return [
    { kind: "table", columns: dated ? ["Title", "URL", "Published"] : ["Title", "URL"], rows: parsed.results.map((result) => dated ? [result.title, result.url, result.published ?? ""] : [result.title, result.url]) },
    ...[parsed.rest, ...excerpts, parsed.notice ?? ""].filter(Boolean).map((text): ToolDisplayBlock => ({ kind: "text", text })),
  ]
}
