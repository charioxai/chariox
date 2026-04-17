export type SessionCommandAction = "create" | "new" | "attach" | "list" | "ls" | "delete"

export type ParsedSlashCommand =
  | { kind: "exit"; raw: string }
  | { kind: "waiting"; raw: string }
  | { kind: "stop"; raw: string }
  | { kind: "attachment"; raw: string }
  | {
      kind: "session"
      raw: string
      action: string | null
      args: string[]
      value: string
    }
  | { kind: "provider"; raw: string; value: string }
  | { kind: "model"; raw: string; value: string }
  | { kind: "variant"; raw: string; value: string }
  | { kind: "view"; raw: string; value: string }
  | { kind: "agent"; raw: string; args: string[] }
  | { kind: "machine"; raw: string; args: string[] }
  | { kind: "relay"; raw: string; args: string[] }
  | { kind: "workflow"; raw: string; args: string[] }
  | { kind: "mcp"; raw: string; args: string[] }
  | { kind: "skill"; raw: string; args: string[] }

export type SlashCommandHandlers = {
  onExit: () => Promise<unknown> | unknown
  onWaiting: () => Promise<unknown> | unknown
  onStop: () => Promise<unknown> | unknown
  onAttachment: (command: Extract<ParsedSlashCommand, { kind: "attachment" }>) => Promise<unknown> | unknown
  onSession: (command: Extract<ParsedSlashCommand, { kind: "session" }>) => Promise<unknown> | unknown
  onProvider: (command: Extract<ParsedSlashCommand, { kind: "provider" }>) => Promise<unknown> | unknown
  onModel: (command: Extract<ParsedSlashCommand, { kind: "model" }>) => Promise<unknown> | unknown
  onVariant: (command: Extract<ParsedSlashCommand, { kind: "variant" }>) => Promise<unknown> | unknown
  onView: (command: Extract<ParsedSlashCommand, { kind: "view" }>) => Promise<unknown> | unknown
  onAgent: (command: Extract<ParsedSlashCommand, { kind: "agent" }>) => Promise<unknown> | unknown
  onMachine: (command: Extract<ParsedSlashCommand, { kind: "machine" }>) => Promise<unknown> | unknown
  onRelay: (command: Extract<ParsedSlashCommand, { kind: "relay" }>) => Promise<unknown> | unknown
  onWorkflow: (command: Extract<ParsedSlashCommand, { kind: "workflow" }>) => Promise<unknown> | unknown
  onMcp: (command: Extract<ParsedSlashCommand, { kind: "mcp" }>) => Promise<unknown> | unknown
  onSkill: (command: Extract<ParsedSlashCommand, { kind: "skill" }>) => Promise<unknown> | unknown
}

export function parseSlashCommand(input: string): ParsedSlashCommand | null {
  const trimmed = input.trim()
  if (!trimmed.startsWith("/")) {
    return null
  }
  if (trimmed === "/exit") {
    return { kind: "exit", raw: trimmed }
  }
  if (trimmed === "/waiting") {
    return { kind: "waiting", raw: trimmed }
  }
  if (trimmed === "/stop") {
    return { kind: "stop", raw: trimmed }
  }
  if (trimmed.startsWith("/attach")) {
    return { kind: "attachment", raw: input }
  }
  if (trimmed.startsWith("/session")) {
    const [_, action, ...args] = trimmed.split(/\s+/)
    return {
      kind: "session",
      raw: trimmed,
      action: action ?? null,
      args,
      value: args.join(" ").trim(),
    }
  }
  if (trimmed.startsWith("/provider")) {
    return {
      kind: "provider",
      raw: trimmed,
      value: trimmed.replace(/^\/provider\s*/, "").trim(),
    }
  }
  if (trimmed.startsWith("/model")) {
    return {
      kind: "model",
      raw: trimmed,
      value: trimmed.replace(/^\/model\s*/, "").trim(),
    }
  }
  if (trimmed.startsWith("/variant")) {
    return {
      kind: "variant",
      raw: trimmed,
      value: trimmed.replace(/^\/variant\s*/, "").trim(),
    }
  }
  if (trimmed.startsWith("/view")) {
    return {
      kind: "view",
      raw: trimmed,
      value: trimmed.replace(/^\/view\s*/, "").trim(),
    }
  }
  if (trimmed.startsWith("/agent")) {
    return {
      kind: "agent",
      raw: trimmed,
      args: trimmed.replace(/^\/agent\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/machine")) {
    return {
      kind: "machine",
      raw: trimmed,
      args: trimmed.replace(/^\/machine\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/relay")) {
    return {
      kind: "relay",
      raw: trimmed,
      args: trimmed.replace(/^\/relay\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/workflow")) {
    return {
      kind: "workflow",
      raw: trimmed,
      args: trimmed.replace(/^\/workflow\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }

  if (trimmed.startsWith("/mcp")) {
    return {
      kind: "mcp",
      raw: trimmed,
      args: trimmed.replace(/^\/mcp\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/skill") || trimmed.startsWith("/skills")) {
    return {
      kind: "skill",
      raw: trimmed,
      args: trimmed.replace(/^\/skills?\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  return null
}

export async function executeSlashCommand(
  input: string,
  handlers: SlashCommandHandlers,
): Promise<ParsedSlashCommand | null> {
  const command = parseSlashCommand(input)
  if (!command) {
    return null
  }
  switch (command.kind) {
    case "exit":
      await handlers.onExit()
      break
    case "waiting":
      await handlers.onWaiting()
      break
    case "stop":
      await handlers.onStop()
      break
    case "attachment":
      await handlers.onAttachment(command)
      break
    case "session":
      await handlers.onSession(command)
      break
    case "provider":
      await handlers.onProvider(command)
      break
    case "model":
      await handlers.onModel(command)
      break
    case "variant":
      await handlers.onVariant(command)
      break
    case "view":
      await handlers.onView(command)
      break
    case "agent":
      await handlers.onAgent(command)
      break
    case "machine":
      await handlers.onMachine(command)
      break
    case "relay":
      await handlers.onRelay(command)
      break
    case "workflow":
      await handlers.onWorkflow(command)
      break
    case "mcp":
      await handlers.onMcp(command)
      break
    case "skill":
      await handlers.onSkill(command)
      break
  }
  return command
}

export function shouldClearCommandCenterForSlashCommand(command: ParsedSlashCommand): boolean {
  switch (command.kind) {
    case "provider":
    case "model":
    case "variant":
    case "view":
    case "agent":
    case "machine":
    case "relay":
    case "workflow":
    case "mcp":
    case "skill":
      return true
    default:
      return false
  }
}
