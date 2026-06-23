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
  | { kind: "undo"; raw: string; args: string[] }
  | { kind: "fork"; raw: string; args: string[] }
  | { kind: "agent"; raw: string; args: string[] }
  | { kind: "kernel"; raw: string; args: string[] }
  | { kind: "machine"; raw: string; args: string[] }
  | { kind: "slice"; raw: string; args: string[] }
  | { kind: "relay"; raw: string; args: string[] }
  | { kind: "cloud"; raw: string; args: string[] }
  | { kind: "collab"; raw: string; args: string[] }
  | { kind: "config"; raw: string; args: string[] }
  | { kind: "workspace"; raw: string; args: string[] }
  | { kind: "worktree"; raw: string; args: string[] }
  | { kind: "workflow"; raw: string; args: string[] }
  | { kind: "mcp"; raw: string; args: string[] }
  | { kind: "skill"; raw: string; args: string[] }
  | { kind: "env"; raw: string; args: string[] }
  | { kind: "script"; raw: string; args: string[] }
  | { kind: "credential"; raw: string; args: string[] }
  | { kind: "connector"; raw: string; args: string[] }
  | { kind: "extension"; raw: string; args: string[] }

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
  onUndo: (command: Extract<ParsedSlashCommand, { kind: "undo" }>) => Promise<unknown> | unknown
  onFork: (command: Extract<ParsedSlashCommand, { kind: "fork" }>) => Promise<unknown> | unknown
  onAgent: (command: Extract<ParsedSlashCommand, { kind: "agent" }>) => Promise<unknown> | unknown
  onKernel: (command: Extract<ParsedSlashCommand, { kind: "kernel" }>) => Promise<unknown> | unknown
  onMachine: (command: Extract<ParsedSlashCommand, { kind: "machine" }>) => Promise<unknown> | unknown
  onSlice: (command: Extract<ParsedSlashCommand, { kind: "slice" }>) => Promise<unknown> | unknown
  onRelay: (command: Extract<ParsedSlashCommand, { kind: "relay" }>) => Promise<unknown> | unknown
  onCloud: (command: Extract<ParsedSlashCommand, { kind: "cloud" }>) => Promise<unknown> | unknown
  onCollab: (command: Extract<ParsedSlashCommand, { kind: "collab" }>) => Promise<unknown> | unknown
  onConfig: (command: Extract<ParsedSlashCommand, { kind: "config" }>) => Promise<unknown> | unknown
  onWorkspace: (command: Extract<ParsedSlashCommand, { kind: "workspace" }>) => Promise<unknown> | unknown
  onWorktree: (command: Extract<ParsedSlashCommand, { kind: "worktree" }>) => Promise<unknown> | unknown
  onWorkflow: (command: Extract<ParsedSlashCommand, { kind: "workflow" }>) => Promise<unknown> | unknown
  onMcp: (command: Extract<ParsedSlashCommand, { kind: "mcp" }>) => Promise<unknown> | unknown
  onSkill: (command: Extract<ParsedSlashCommand, { kind: "skill" }>) => Promise<unknown> | unknown
  onEnv: (command: Extract<ParsedSlashCommand, { kind: "env" }>) => Promise<unknown> | unknown
  onScript: (command: Extract<ParsedSlashCommand, { kind: "script" }>) => Promise<unknown> | unknown
  onCredential: (command: Extract<ParsedSlashCommand, { kind: "credential" }>) => Promise<unknown> | unknown
  onConnector: (command: Extract<ParsedSlashCommand, { kind: "connector" }>) => Promise<unknown> | unknown
  onExtension: (command: Extract<ParsedSlashCommand, { kind: "extension" }>) => Promise<unknown> | unknown
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
  if (trimmed.startsWith("/undo")) {
    return {
      kind: "undo",
      raw: trimmed,
      args: trimmed.replace(/^\/undo\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/fork")) {
    return {
      kind: "fork",
      raw: trimmed,
      args: trimmed.replace(/^\/fork\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/agent")) {
    return {
      kind: "agent",
      raw: trimmed,
      args: trimmed.replace(/^\/agent\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/kernel")) {
    return {
      kind: "kernel",
      raw: trimmed,
      args: trimmed.replace(/^\/kernel\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/machine")) {
    return {
      kind: "machine",
      raw: trimmed,
      args: trimmed.replace(/^\/machine\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/slice")) {
    return {
      kind: "slice",
      raw: trimmed,
      args: trimmed.replace(/^\/slice\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/relay")) {
    return {
      kind: "relay",
      raw: trimmed,
      args: trimmed.replace(/^\/relay\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/cloud")) {
    return {
      kind: "cloud",
      raw: trimmed,
      args: trimmed.replace(/^\/cloud\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/collab")) {
    return {
      kind: "collab",
      raw: trimmed,
      args: trimmed.replace(/^\/collab\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/config")) {
    return {
      kind: "config",
      raw: trimmed,
      args: trimmed.replace(/^\/config\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/workspace")) {
    return {
      kind: "workspace",
      raw: trimmed,
      args: trimmed.replace(/^\/workspace\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/worktree")) {
    return {
      kind: "worktree",
      raw: trimmed,
      args: trimmed.replace(/^\/worktree\s*/, "").trim().split(/\s+/).filter(Boolean),
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
  if (trimmed.startsWith("/env") || trimmed.startsWith("/environment")) {
    return {
      kind: "env",
      raw: trimmed,
      args: trimmed.replace(/^\/(?:env|environment)\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/script")) {
    return {
      kind: "script",
      raw: trimmed,
      args: trimmed.replace(/^\/script\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/credential")) {
    return {
      kind: "credential",
      raw: trimmed,
      args: trimmed.replace(/^\/credential\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/connector")) {
    return {
      kind: "connector",
      raw: trimmed,
      args: trimmed.replace(/^\/connector\s*/, "").trim().split(/\s+/).filter(Boolean),
    }
  }
  if (trimmed.startsWith("/extension")) {
    return {
      kind: "extension",
      raw: trimmed,
      args: trimmed.replace(/^\/extension\s*/, "").trim().split(/\s+/).filter(Boolean),
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
    case "undo":
      await handlers.onUndo(command)
      break
    case "fork":
      await handlers.onFork(command)
      break
    case "agent":
      await handlers.onAgent(command)
      break
    case "kernel":
      await handlers.onKernel(command)
      break
    case "machine":
      await handlers.onMachine(command)
      break
    case "slice":
      await handlers.onSlice(command)
      break
    case "relay":
      await handlers.onRelay(command)
      break
    case "cloud":
      await handlers.onCloud(command)
      break
    case "collab":
      await handlers.onCollab(command)
      break
    case "config":
      await handlers.onConfig(command)
      break
    case "workspace":
      await handlers.onWorkspace(command)
      break
    case "worktree":
      await handlers.onWorktree(command)
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
    case "env":
      await handlers.onEnv(command)
      break
    case "script":
      await handlers.onScript(command)
      break
    case "credential":
      await handlers.onCredential(command)
      break
    case "connector":
      await handlers.onConnector(command)
      break
    case "extension":
      await handlers.onExtension(command)
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
    case "undo":
    case "fork":
    case "agent":
    case "kernel":
    case "machine":
    case "slice":
    case "relay":
    case "cloud":
    case "collab":
    case "config":
    case "workspace":
    case "worktree":
    case "workflow":
    case "mcp":
    case "skill":
    case "env":
    case "script":
    case "credential":
    case "connector":
    case "extension":
      return true
    default:
      return false
  }
}
