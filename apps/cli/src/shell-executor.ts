import { execFile } from "node:child_process"
import { stat } from "node:fs/promises"
import { basename, dirname, resolve as resolvePath } from "node:path"
import { promisify } from "node:util"

import type { AgentInstance, RuntimeSession } from "./cli-types.js"
import { formatAgentListSummary } from "./command-actions.js"
import {
  createSessionRequest,
  focusAgentRequest,
  listAgentsRequest,
  listSessionsRequest,
  resolveSessionRequest,
  spawnAgentRequest,
  cycleAgentFocusRequest,
} from "./ipc-requests.js"
import { formatSessionList } from "./sessions.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"

const execFileAsync = promisify(execFile)

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

type LocalGitWorktreeOptions = {
  baseDirectory: string
  targetDirectory?: string | undefined
  branch?: string | undefined
  fromRef?: string | undefined
}

type PlacementOptions = {
  positional: string[]
  directory?: string | undefined
  gitWorktree?: string | undefined
  branch?: string | undefined
  fromRef?: string | undefined
  machineRef?: string | undefined
}

export type ShellExecutorDeps = {
  client: ShellKernelClient
  prepareLocalGitWorktree?: ((options: LocalGitWorktreeOptions) => Promise<string>) | undefined
  resolveExistingDirectory?: ((directory: string, baseDirectory: string, label: string) => Promise<string>) | undefined
}

export async function executeShellCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  if (parsed.kind === "empty") {
    return { ok: true, message: "" }
  }
  if (parsed.kind === "invalid") {
    return { ok: false, message: parsed.reason ?? "invalid command" }
  }
  if (parsed.kind === "tui-only") {
    return { ok: false, message: parsed.reason ?? `${parsed.command ?? "command"} is only available in the TUI client` }
  }
  if (parsed.kind === "shell-local") {
    return executeShellLocalCommand(parsed, context)
  }
  switch (parsed.command) {
    case "session":
      return executeSessionCommand(parsed, context, deps)
    case "agent":
      return executeAgentCommand(parsed, context, deps)
    default:
      return {
        ok: false,
        message: `${parsed.command ?? "command"} is not implemented in arroba-shell yet`,
      }
  }
}

function executeShellLocalCommand(parsed: ParsedShellCommand, context: ShellContext): ShellCommandResult {
  const [first, second] = parsed.args
  switch (parsed.command) {
    case "help":
      return {
        ok: true,
        message: [
          "arroba-shell commands:",
          "session list|new|attach|use",
          "agent list|spawn|focus|cycle",
          "set provider|model|effort <value>",
          "use session|agent|workflow <ref>",
          "vars",
          "unset <name>",
          "exit",
        ].join("\n"),
      }
    case "set": {
      if (first !== "provider" && first !== "model" && first !== "effort") {
        return { ok: false, message: "usage: set provider|model|effort <value>" }
      }
      if (!second) {
        return { ok: false, message: `usage: set ${first} <value>` }
      }
      return {
        ok: true,
        message: `${first} = ${second}`,
        contextUpdates: { [first]: second },
      }
    }
    case "use": {
      if (first !== "session" && first !== "agent" && first !== "workflow") {
        return { ok: false, message: "usage: use session|agent|workflow <ref>" }
      }
      if (!second) {
        return { ok: false, message: `usage: use ${first} <ref>` }
      }
      const key = first === "session" ? "sessionId" : first === "agent" ? "agentId" : "workflowId"
      return {
        ok: true,
        message: `current ${first} = ${second}`,
        contextUpdates: { [key]: second },
      }
    }
    case "vars": {
      const entries = Object.entries(context.variables)
      return {
        ok: true,
        message: entries.length === 0
          ? "no variables bound"
          : entries.map(([name, value]) => `$${name} = ${value}`).join("\n"),
      }
    }
    case "unset": {
      if (!first) {
        return { ok: false, message: "usage: unset <name>" }
      }
      const nextVariables = { ...context.variables }
      delete nextVariables[first]
      return {
        ok: true,
        message: `unset $${first}`,
        variableRemovals: [first],
        data: { variables: nextVariables },
      }
    }
    case "exit":
    case "quit":
      return { ok: true, message: "exit", data: { exit: true } }
    case "source":
    case "run":
      return { ok: false, message: "script execution is not implemented yet" }
    default:
      return { ok: false, message: `${parsed.command ?? "command"} is not implemented in arroba-shell yet` }
  }
}

async function executeSessionCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, ...args] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listSessionsRequest())
      const sessions = expectVariant<{ sessions: RuntimeSession[] }>(response, "SessionsListed").sessions
      return {
        ok: true,
        message: formatSessionList(sessions, context.sessionId),
        data: { sessions },
      }
    }
    case "new":
    case "create": {
      const placement = parsePlacementOptions(args, false)
      if (placement.error) {
        return { ok: false, message: placement.error }
      }
      if (placement.options.positional.length > 1) {
        return { ok: false, message: "usage: session new [directory] [--dir <directory>] [--worktree <directory> --branch <branch>]" }
      }
      const worktree = (await resolveShellPlacement(placement.options, context.worktree, "session working directory", deps))
        ?? context.worktree
      const response = await deps.client.send(createSessionRequest(context.workspace, worktree))
      const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionCreated")
      const session = payload.session
      return resourceResult(
        `created session ${session.alias ?? session.id} in ${session.worktree_id}`,
        parsed.assignment,
        session.id,
        { sessionId: session.id, agentId: session.focused_agent_id ?? undefined, workspace: session.workspace_id, worktree: session.worktree_id },
        { session },
      )
    }
    case "attach":
    case "use": {
      const sessionRef = args[0]
      if (!sessionRef) {
        return { ok: false, message: `usage: session ${action} <ref>` }
      }
      const response = await deps.client.send(resolveSessionRequest(sessionRef, context.workspace))
      const session = expectVariant<{ session: RuntimeSession }>(response, "SessionResolved").session
      return resourceResult(
        `current session = ${session.alias ?? session.id}`,
        parsed.assignment,
        session.id,
        { sessionId: session.id, agentId: session.focused_agent_id ?? undefined, workspace: session.workspace_id, worktree: session.worktree_id },
        { session },
      )
    }
    default:
      return { ok: false, message: "usage: session list|new|attach|use" }
  }
}

async function executeAgentCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [action, ...args] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listAgentsRequest(sessionId))
      const agents = expectVariant<{ agents: AgentInstance[] }>(response, "AgentsListed").agents
      return { ok: true, message: formatAgentListSummary(agents), data: { agents } }
    }
    case "spawn": {
      const parsedSpawn = parsePlacementOptions(args, true)
      if (parsedSpawn.error) {
        return { ok: false, message: parsedSpawn.error }
      }
      const [alias, model] = parsedSpawn.options.positional
      if (parsedSpawn.options.positional.length > 2) {
        return { ok: false, message: "usage: agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>]" }
      }
      const worktree = await resolveShellPlacement(parsedSpawn.options, context.worktree, "agent working directory", deps)
      const remotePlacement = parsedSpawn.options.machineRef && (parsedSpawn.options.gitWorktree || parsedSpawn.options.branch || parsedSpawn.options.fromRef)
        ? {
            target_directory: parsedSpawn.options.gitWorktree ?? null,
            branch: parsedSpawn.options.branch ?? null,
            from_ref: parsedSpawn.options.fromRef ?? null,
          }
        : undefined
      const response = await deps.client.send(spawnAgentRequest(
        sessionId,
        context.provider,
        alias,
        model ?? context.model,
        worktree,
        context.effort,
        parsedSpawn.options.machineRef,
        remotePlacement,
      ))
      const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentSpawned").agent
      const placement = agent.remote_execution
        ? ` on ${parsedSpawn.options.machineRef ?? agent.remote_execution.worker_machine_id}`
        : agent.worktree_id ? ` in ${agent.worktree_id}` : ""
      return resourceResult(
        `spawned agent ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}${placement}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    case "focus": {
      const agentRef = args[0]
      if (!agentRef) {
        return { ok: false, message: "usage: agent focus <agent-id>" }
      }
      const response = await deps.client.send(focusAgentRequest(sessionId, agentRef))
      const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentFocused").agent
      return resourceResult(
        `current agent = ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    case "cycle": {
      const response = await deps.client.send(cycleAgentFocusRequest(sessionId))
      const agent = expectVariant<{ agent: AgentInstance | null }>(response, "AgentFocusCycled").agent
      if (!agent) {
        return { ok: true, message: "no agents to cycle" }
      }
      return resourceResult(
        `current agent = ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    default:
      return { ok: false, message: "usage: agent list|spawn|focus|cycle" }
  }
}

function resourceResult(
  message: string,
  assignment: string | undefined,
  value: string,
  contextUpdates: ShellCommandResult["contextUpdates"],
  data: unknown,
): ShellCommandResult {
  return {
    ok: true,
    message,
    data,
    bindings: assignment ? { [assignment]: value } : undefined,
    contextUpdates,
  }
}

function parsePlacementOptions(args: string[], allowMachine: boolean): { options: PlacementOptions; error?: string } {
  const options: PlacementOptions = { positional: [] }
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    const next = args[index + 1]
    if ((arg === "--dir" || arg === "--directory") && next) {
      options.directory = next
      index += 1
    } else if (arg === "--worktree" && next) {
      options.gitWorktree = next
      index += 1
    } else if (arg === "--branch" && next) {
      options.branch = next
      index += 1
    } else if (arg === "--from" && next) {
      options.fromRef = next
      index += 1
    } else if (arg === "--machine" && next && allowMachine) {
      options.machineRef = next
      index += 1
    } else if (arg?.startsWith("--")) {
      return { options, error: `unknown or incomplete option: ${arg}` }
    } else if (arg) {
      options.positional.push(arg)
    }
  }
  if (options.directory && options.gitWorktree) {
    return { options, error: "use either --dir or --worktree, not both" }
  }
  if ((options.branch || options.fromRef) && !options.gitWorktree) {
    return { options, error: "--branch/--from require --worktree" }
  }
  return { options }
}

async function resolveShellPlacement(
  options: PlacementOptions,
  baseDirectory: string,
  label: string,
  deps: ShellExecutorDeps,
): Promise<string | undefined> {
  if (options.machineRef) {
    return options.directory ?? options.gitWorktree ?? undefined
  }
  const positionalDirectory = options.positional.length === 1 && !options.directory && !options.gitWorktree
    ? options.positional[0]
    : undefined
  if (positionalDirectory || options.directory) {
    const directory = positionalDirectory ?? options.directory!
    const resolver = deps.resolveExistingDirectory ?? defaultResolveExistingDirectory
    return resolver(directory, baseDirectory, label)
  }
  if (options.gitWorktree) {
    const prepare = deps.prepareLocalGitWorktree ?? defaultPrepareLocalGitWorktree
    return prepare({
      baseDirectory,
      targetDirectory: options.gitWorktree,
      branch: options.branch,
      fromRef: options.fromRef,
    })
  }
  return undefined
}

async function defaultResolveExistingDirectory(directory: string, baseDirectory: string, label: string): Promise<string> {
  const resolved = resolvePath(baseDirectory, directory)
  const details = await stat(resolved)
  if (!details.isDirectory()) {
    throw new Error(`${label} is not a directory: ${resolved}`)
  }
  return resolved
}

async function defaultPrepareLocalGitWorktree(options: LocalGitWorktreeOptions): Promise<string> {
  const baseDirectory = resolvePath(options.baseDirectory)
  const repoRoot = (await runGit(baseDirectory, ["rev-parse", "--show-toplevel"])).trim()
  if (!repoRoot) {
    throw new Error(`git did not report a repository root for ${baseDirectory}`)
  }
  const fromRef = options.fromRef ?? "HEAD"
  const targetDirectory = options.targetDirectory
    ? resolvePath(baseDirectory, options.targetDirectory)
    : resolvePath(dirname(repoRoot), `${basename(repoRoot)}-${slugifyGitBranch(options.branch ?? fromRef)}`)
  const args = options.branch
    ? ["worktree", "add", "-b", options.branch, targetDirectory, fromRef]
    : ["worktree", "add", targetDirectory, fromRef]
  await runGit(repoRoot, args)
  const details = await stat(targetDirectory)
  if (!details.isDirectory()) {
    throw new Error(`created git worktree is not a directory: ${targetDirectory}`)
  }
  return targetDirectory
}

async function runGit(cwd: string, args: string[]): Promise<string> {
  try {
    const { stdout } = await execFileAsync("git", args, { cwd })
    return stdout
  } catch (error) {
    const detail = error && typeof error === "object" && "stderr" in error
      ? String((error as { stderr?: unknown }).stderr ?? "").trim()
      : ""
    const message = detail || (error instanceof Error ? error.message : String(error))
    throw new Error(`git ${args.join(" ")} failed in ${cwd}: ${message}`)
  }
}

function slugifyGitBranch(value: string): string {
  return value
    .trim()
    .replace(/[^A-Za-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    || "worktree"
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
