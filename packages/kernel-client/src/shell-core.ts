export type ShellCommandKind = "shared" | "shell-local" | "tui-only" | "empty" | "invalid"

export type ShellContext = {
  workspace: string
  worktree: string
  sessionId?: string | undefined
  attachmentId?: string | undefined
  agentId?: string | undefined
  workflowId?: string | undefined
  provider: string
  model: string
  effort: string
  variables: Record<string, string>
}

export type ShellCommandResult = {
  ok: boolean
  message?: string | undefined
  data?: unknown | undefined
  format?: "text" | "table" | "json" | undefined
  bindings?: Record<string, string> | undefined
  variableRemovals?: string[] | undefined
  contextUpdates?: Partial<Omit<ShellContext, "variables">> | undefined
}

export type ParsedShellCommand = {
  kind: ShellCommandKind
  raw: string
  normalized: string
  command?: string | undefined
  args: string[]
  assignment?: string | undefined
  reason?: string | undefined
}

const TUI_ONLY_COMMANDS = new Set([
  "waiting",
  "view",
  "attachment",
])

const TUI_ONLY_PROVIDER_NAMESPACES = new Set([
  "opencode",
  "codex",
])

const SHELL_LOCAL_COMMANDS = new Set([
  "exit",
  "quit",
  "help",
  "pwd",
  "set",
  "use",
  "vars",
  "unset",
  "source",
  "run",
])

export function createDefaultShellContext(options: Partial<ShellContext> = {}): ShellContext {
  const workspace = options.workspace ?? process.cwd()
  return {
    workspace,
    worktree: options.worktree ?? workspace,
    sessionId: options.sessionId,
    attachmentId: options.attachmentId,
    agentId: options.agentId,
    workflowId: options.workflowId,
    provider: options.provider ?? "opencode",
    model: options.model ?? "default",
    effort: options.effort ?? "medium",
    variables: { ...(options.variables ?? {}) },
  }
}

export function parseShellCommand(input: string, context: Pick<ShellContext, "variables"> = { variables: {} }): ParsedShellCommand {
  const raw = input.trim()
  if (!raw || raw.startsWith("#")) {
    return { kind: "empty", raw: input, normalized: "", args: [] }
  }
  const withoutSlash = raw.startsWith("/") ? raw.slice(1).trimStart() : raw
  let tokens: string[]
  try {
    tokens = substituteVariables(tokenizeShellLine(withoutSlash), context.variables)
  } catch (error) {
    return {
      kind: "invalid",
      raw: input,
      normalized: withoutSlash,
      args: [],
      reason: error instanceof Error ? error.message : String(error),
    }
  }
  if (tokens.length === 0) {
    return { kind: "empty", raw: input, normalized: "", args: [] }
  }

  let assignment: string | undefined
  try {
    assignment = extractAssignment(tokens)
  } catch (error) {
    return {
      kind: "invalid",
      raw: input,
      normalized: withoutSlash,
      args: tokens,
      reason: error instanceof Error ? error.message : String(error),
    }
  }
  const command = tokens[0] ?? ""
  const args = tokens.slice(1)
  const normalized = [command, ...args].join(" ")

  if (TUI_ONLY_COMMANDS.has(command) || TUI_ONLY_PROVIDER_NAMESPACES.has(command)) {
    return {
      kind: "tui-only",
      raw: input,
      normalized,
      command,
      args,
      assignment,
      reason: `${command} is only available in the TUI client`,
    }
  }
  if (SHELL_LOCAL_COMMANDS.has(command)) {
    return { kind: "shell-local", raw: input, normalized, command, args, assignment }
  }
  return { kind: "shared", raw: input, normalized, command, args, assignment }
}

export function applyShellCommandResult(context: ShellContext, result: ShellCommandResult): ShellContext {
  const variables = { ...context.variables }
  for (const name of result.variableRemovals ?? []) {
    delete variables[name]
  }
  return {
    ...context,
    ...result.contextUpdates,
    variables: {
      ...variables,
      ...(result.bindings ?? {}),
    },
  }
}

export function renderShellCommandResult(result: ShellCommandResult): string {
  const lines: string[] = []
  if (result.message) {
    lines.push(result.message)
  } else if (result.data !== undefined) {
    lines.push(JSON.stringify(result.data, null, 2))
  } else if (result.ok) {
    lines.push("ok")
  }
  for (const [name, value] of Object.entries(result.bindings ?? {})) {
    lines.push(`bound $${name} = ${value}`)
  }
  if (!result.ok && lines.length === 0) {
    lines.push("error")
  }
  return lines.join("\n")
}

export function tokenizeShellLine(input: string): string[] {
  const tokens: string[] = []
  let current = ""
  let quote: "'" | '"' | null = null
  let escaping = false

  for (const char of input) {
    if (escaping) {
      current += char
      escaping = false
      continue
    }
    if (char === "\\" && quote !== "'") {
      escaping = true
      continue
    }
    if (quote) {
      if (char === quote) {
        quote = null
      } else {
        current += char
      }
      continue
    }
    if (char === "'" || char === '"') {
      quote = char
      continue
    }
    if (/\s/.test(char)) {
      if (current) {
        tokens.push(current)
        current = ""
      }
      continue
    }
    current += char
  }

  if (escaping) {
    current += "\\"
  }
  if (quote) {
    throw new Error("unterminated quote")
  }
  if (current) {
    tokens.push(current)
  }
  return tokens
}

function substituteVariables(tokens: string[], variables: Record<string, string>): string[] {
  return tokens.map((token) => token.replace(/\$([A-Za-z_][A-Za-z0-9_]*)/g, (_match, name: string) => {
    const value = variables[name]
    if (value === undefined) {
      throw new Error(`unknown variable $${name}`)
    }
    return value
  }))
}

function extractAssignment(tokens: string[]): string | undefined {
  if (tokens.length < 2) {
    return undefined
  }
  const markerIndex = tokens.lastIndexOf("as")
  if (markerIndex < 0) {
    return undefined
  }
  const name = tokens[markerIndex + 1]
  if (!name || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
    throw new Error("assignment must use `as <name>` with an identifier")
  }
  tokens.splice(markerIndex, 2)
  return name
}
