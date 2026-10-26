import type { ApplyPatchFile, ToolDisplayIcon, ToolTranscriptUpdate } from "./types.js"

// Verb + object titles for tool rows ("Ran ls -la", "Read README.md",
// "Edited 2 files (+3 −1)"), shared by every client. The raw tool name is
// only a last resort, with "_" / "__" separators turned into spaces.
export function toolDisplayTitle(update: ToolTranscriptUpdate, patchFiles: readonly ApplyPatchFile[] = []): string {
  const active = toolTitleIsActive(update.status)
  const verb = (done: string, running: string) => active ? running : done
  const input = toolTitleRecord(update.input)
  const name = toolTitleCanonicalName(update.tool)

  if (patchFiles.length > 0) {
    return `${verb("Edited", "Editing")} ${toolTitlePatchObject(patchFiles)}`
  }
  const command = toolTitleCommand(input)
  if (command !== null || TOOL_TITLE_SHELL.has(name)) {
    return command ? toolTitleShellCommand(command, verb) : verb("Ran a command", "Running a command")
  }
  const path = toolTitleString(input, "file_path", "filePath", "path", "notebook_path", "notebookPath", "file")
  const pattern = toolTitleString(input, "pattern", "query", "q", "regex")
  switch (name) {
    case "read":
    case "readfile":
    case "view":
    case "charioxreadartifact":
    case "readartifact":
      return `${verb("Read", "Reading")} ${toolTitleShort(path) ?? "a file"}`
    case "edit":
    case "multiedit":
    case "strreplace":
    case "strreplacebasededittool":
    case "notebookedit":
    case "applypatch":
    case "patch":
    case "charioxeditartifact":
    case "charioxapplypatch":
    case "charioxpatchartifact":
    case "mcpcharioxpatchartifact":
    case "editartifact":
    case "patchartifact":
      return `${verb("Edited", "Editing")} ${toolTitleShort(path) ?? "files"}`
    case "write":
    case "writefile":
    case "create":
    case "charioxwriteartifact":
    case "writeartifact":
      return `${verb("Wrote", "Writing")} ${toolTitleShort(path) ?? "a file"}`
    case "charioxdeleteartifact":
    case "deleteartifact":
      return `${verb("Deleted", "Deleting")} ${toolTitleShort(path) ?? "a file"}`
    case "charioxmoveartifact":
    case "moveartifact":
      return `${verb("Moved", "Moving")} ${toolTitleShort(path) ?? "a file"}`
    case "grep":
    case "search":
    case "rg":
    case "codebasesearch":
    case "searchfiles":
      return `${verb("Searched", "Searching")} ${pattern ? `for ${toolTitleShort(pattern)}` : "files"}${path ? ` in ${toolTitleShort(path)}` : ""}`
    case "glob":
    case "find":
    case "findfiles":
      return `${verb("Searched", "Searching")} files${pattern ? ` matching ${toolTitleShort(pattern)}` : ""}${path ? ` in ${toolTitleShort(path)}` : ""}`
    case "ls":
    case "list":
    case "listdir":
    case "listfiles":
    case "listdirectory":
      return `${verb("Listed", "Listing")} ${toolTitleShort(path) ?? "files"}`
    case "websearch":
    case "webquery":
      return pattern ? `${verb("Searched", "Searching")} the web for ${toolTitleShort(pattern)}` : verb("Searched the web", "Searching the web")
    case "webfetch":
    case "fetch":
    case "fetchurl":
      return `${verb("Fetched", "Fetching")} ${toolTitleShort(toolTitleString(input, "url", "uri")) ?? "a page"}`
    case "todowrite":
    case "updateplan":
    case "todo":
      return verb("Updated plan", "Updating plan")
    case "todoread":
      return verb("Read plan", "Reading plan")
    case "task":
    case "agent":
    case "spawnagent":
      return `${verb("Delegated", "Delegating")} ${toolTitleShort(toolTitleString(input, "description", "prompt")) ?? "a task"}`
  }
  return `${verb("Called", "Calling")} ${toolTitleCalledName(update)}`
}

// One single-width glyph per icon for terminal clients.
export const TOOL_DISPLAY_ICON_GLYPHS: Readonly<Record<ToolDisplayIcon, string>> = {
  terminal: "$",
  read: "☰",
  edit: "✎",
  search: "⌕",
  web: "◎",
  browser: "▣",
  mcp: "⌘",
  agent: "✦",
  plan: "☐",
  tool: "•",
}

// The icon shown left of a tool row's title; same classification as the title.
export function toolDisplayIcon(update: ToolTranscriptUpdate, patchFiles: readonly ApplyPatchFile[] = []): ToolDisplayIcon {
  const input = toolTitleRecord(update.input)
  const name = toolTitleCanonicalName(update.tool)
  const raw = typeof update.tool === "string" ? update.tool : ""
  const browser = (value: string) => /browser|computer|playwright|chrome|screenshot|navigate|webview/i.test(value)
  if (patchFiles.length > 0) {
    return "edit"
  }
  if (browser(raw)) {
    return "browser"
  }
  const command = toolTitleCommand(input)
  if (command !== null || TOOL_TITLE_SHELL.has(name)) {
    const program = command ? toolTitleSimpleProgram(toolTitleUnwrapShell(command)) : null
    if (program === "cat" || program === "bat" || program === "less" || program === "ls" || program === "tree") return "read"
    if (program === "rg" || program === "grep") return "search"
    return "terminal"
  }
  switch (name) {
    case "read": case "readfile": case "view": case "charioxreadartifact": case "readartifact":
    case "ls": case "list": case "listdir": case "listfiles": case "listdirectory": case "todoread":
      return name === "todoread" ? "plan" : "read"
    case "edit": case "multiedit": case "strreplace": case "strreplacebasededittool": case "notebookedit":
    case "applypatch": case "patch": case "charioxeditartifact": case "charioxapplypatch": case "charioxpatchartifact":
    case "mcpcharioxpatchartifact": case "editartifact": case "patchartifact": case "write": case "writefile":
    case "create": case "charioxwriteartifact": case "writeartifact": case "charioxdeleteartifact":
    case "deleteartifact": case "charioxmoveartifact": case "moveartifact":
      return "edit"
    case "grep": case "search": case "rg": case "codebasesearch": case "searchfiles": case "glob": case "find": case "findfiles":
      return "search"
    case "websearch": case "webquery": case "webfetch": case "fetch": case "fetchurl":
      return "web"
    case "todowrite": case "updateplan": case "todo":
      return "plan"
    case "task": case "agent": case "spawnagent":
      return "agent"
  }
  // Codex MCP calls carry the server in `title`.
  const server = typeof update.title === "string" && /^[\w.-]+$/.test(update.title.trim()) ? update.title.trim() : ""
  if (browser(server)) {
    return "browser"
  }
  // OpenCode exposes the Chariox runtime MCP namespace as an ordinary tool name.
  const charioxRuntimeTool = /^chariox[_.:]/i.test(raw)
  return raw.startsWith("mcp__") || charioxRuntimeTool || server ? "mcp" : "tool"
}

// The program of a one-line command without pipes or substitutions.
function toolTitleSimpleProgram(script: string) {
  const line = script.trim()
  return /^[^|;&<>`$()\n]+$/.test(line) ? line.split(/\s+/)[0]!.split("/").pop() ?? null : null
}

const TOOL_TITLE_SHELL = new Set([
  "bash", "shell", "sh", "shellcommand", "execcommand", "runcommand", "runshellcommand",
  "localshell", "terminal", "commandexecution", "exec", "unifiedexec",
])

const TOOL_TITLE_ACTIVE = new Set(["running", "pending", "in_progress", "inprogress", "started", "queued"])

function toolTitleIsActive(status: unknown) {
  return typeof status === "string" && TOOL_TITLE_ACTIVE.has(status.trim().toLowerCase())
}

function toolTitleCanonicalName(tool: unknown) {
  return typeof tool === "string" ? tool.trim().replace(/[\s._-]/g, "").toLowerCase() : ""
}

function toolTitleRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null
}

function toolTitleString(record: Record<string, unknown> | null, ...keys: string[]) {
  for (const key of keys) {
    const value = record?.[key]
    if (typeof value === "string" && value.trim()) {
      return value.trim()
    }
  }
  return null
}

function toolTitleCommand(input: Record<string, unknown> | null) {
  const command = input?.command ?? input?.cmd
  if (Array.isArray(command)) {
    const parts = command.filter((part): part is string => typeof part === "string")
    return parts.length > 0 ? parts.join(" ") : null
  }
  return typeof command === "string" && command.trim() ? command.trim() : null
}

// Codex runs commands as `/bin/bash -lc '<script>'`; title the script itself.
function toolTitleUnwrapShell(command: string) {
  const match = command.match(/^(?:\S*\/)?(?:bash|sh|zsh)\s+-l?c\s+([\s\S]+)$/)
  if (!match) {
    return command
  }
  const script = match[1]!.trim()
  if (script.length >= 2 && script[0] === "'" && script.endsWith("'")) {
    return script.slice(1, -1).replace(/'\\''/g, "'").trim()
  }
  if (script.length >= 2 && script[0] === "\"" && script.endsWith("\"")) {
    return script.slice(1, -1).replace(/\\(["\\$`])/g, "$1").trim()
  }
  return script
}

function toolTitleShellCommand(command: string, verb: (done: string, running: string) => string) {
  const script = toolTitleUnwrapShell(command)
  const firstLine = script.split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? script
  const simple = /^[^|;&<>`$()]+$/.test(firstLine) && !script.includes("\n")
  const words = firstLine.match(/'[^']*'|"(?:\\.|[^"])*"|[^\s]+/g) ?? []
  const operands = words.slice(1).filter((word) => !word.startsWith("-"))
  if (simple) {
    const program = words[0]!.split("/").pop()
    if ((program === "cat" || program === "bat" || program === "less") && operands.length > 0) {
      return `${verb("Read", "Reading")} ${operands.map(toolTitleUnquote).join(", ")}`
    }
    if (program === "ls" || program === "tree") {
      return `${verb("Listed", "Listing")} ${operands.length > 0 ? operands.map(toolTitleUnquote).join(", ") : "files"}`
    }
    if (program === "rg" || program === "grep") {
      const explicitPatterns: string[] = []
      const patternFiles: string[] = []
      const positional: string[] = []
      for (let index = 1; index < words.length; index += 1) {
        const word = words[index]!
        if (word === "-e" || word === "--regexp") {
          if (words[index + 1]) explicitPatterns.push(toolTitleUnquote(words[++index]!))
        } else if (word.startsWith("--regexp=")) {
          explicitPatterns.push(toolTitleUnquote(word.slice(9)))
        } else if (word === "-f" || word === "--file") {
          if (words[index + 1]) patternFiles.push(toolTitleUnquote(words[++index]!))
        } else if (word.startsWith("--file=")) {
          patternFiles.push(toolTitleUnquote(word.slice(7)))
        } else if (TOOL_TITLE_VALUE_OPTIONS.has(word)) {
          index += 1
        } else if (!word.startsWith("-")) {
          positional.push(toolTitleUnquote(word))
        }
      }
      if (patternFiles.length) {
        const label = explicitPatterns.length ? `for ${explicitPatterns.join(", ")} and` : "using"
        return `${verb("Searched", "Searching")} ${label} patterns from ${patternFiles.join(", ")}${positional.length ? ` in ${positional.join(", ")}` : ""}`
      }
      const [pattern, ...paths] = explicitPatterns.length
        ? [explicitPatterns.join(", "), ...positional]
        : positional
      if (!pattern) return `${verb("Ran", "Running")} ${toolTitleShort(firstLine)}`
      return `${verb("Searched", "Searching")} for ${toolTitleShort(pattern!)}${paths.length > 0 ? ` in ${paths.join(", ")}` : ""}`
    }
  }
  return `${verb("Ran", "Running")} ${toolTitleShort(firstLine)}${firstLine !== script.trim() ? " …" : ""}`
}

// Search options whose value is the next word, so it is not the pattern.
const TOOL_TITLE_VALUE_OPTIONS = new Set([
  "-e", "-f", "-g", "-m", "-A", "-B", "-C", "-t", "-T", "-j", "--glob", "--iglob", "--max-count", "--type",
  "--type-not", "--context", "--after-context", "--before-context", "--regexp", "--file", "--max-depth",
  "--max-filesize", "--threads", "--include", "--exclude", "--exclude-dir",
])

function toolTitleUnquote(value: string) {
  return value.replace(/^(['"])(.*)\1$/, "$2")
}

function toolTitlePatchObject(files: readonly ApplyPatchFile[]) {
  let added = 0
  let removed = 0
  for (const file of files) {
    for (const line of file.diff?.split(/\r?\n/) ?? []) {
      if (line.startsWith("+") && !line.startsWith("+++")) added += 1
      else if (line.startsWith("-") && !line.startsWith("---")) removed += 1
    }
  }
  const counts = added || removed ? ` (+${added} −${removed})` : ""
  return files.length === 1 ? `${toolTitleShort(files[0]!.filePath)}${counts}` : `${files.length} files${counts}`
}

function toolTitleCalledName(update: ToolTranscriptUpdate) {
  const raw = typeof update.tool === "string" ? update.tool.trim() : ""
  const mcp = raw.match(/^mcp__(.+?)__(.+)$/)
  if (mcp) {
    return `${toolTitleWords(mcp[1]!)}.${toolTitleWords(mcp[2]!)}`
  }
  // Codex MCP calls carry the server in `title`.
  const server = typeof update.title === "string" && /^[\w.-]+$/.test(update.title.trim()) ? update.title.trim() : null
  const tool = toolTitleWords(raw) || "a tool"
  return server && raw ? `${server}.${tool}` : tool
}

function toolTitleWords(value: string) {
  return value.replace(/_+/g, " ").replace(/\s+/g, " ").trim()
}

function toolTitleShort(value: string | null | undefined, max = 80) {
  if (!value) {
    return null
  }
  const line = value.replace(/\s+/g, " ").trim()
  return line.length > max ? `${line.slice(0, max - 1)}…` : line
}
