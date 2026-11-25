import type { SendRequest } from "./kernel-files.js"
import { CommandFailure, parseArguments, usageOf, wantsHelp } from "./agent-command-table.js"
import { reportFailure, withKernel } from "./kernels-command.js"
import { defaultKernelEndpoint } from "./cli-options.js"
import { LocalIpcClient } from "./ipc.js"
import { destroyAgentRequest } from "./ipc-requests.js"

type Agent = { id: string; agent_ref?: string; alias?: string | null; state?: string | undefined }
type Tree = { AgentTree: { tree: { nodes: { agent_id: string; alias?: string | null; state?: string; depth?: number; room_id?: string | null }[] } } }
export type Listed = { id: string; alias: string | null; running: boolean }

const isRunning = (state: string | undefined) => state === "Working" || state === "running"

type Activity = { SessionState: { agent_activity?: Record<string, { status?: string }> } }

/** The one Room that has the named agent; several matches need --session. Sub-agents are hidden from the
 *  Room's agent list, so they are found below the Room's agents. */
async function findAgent(reference: string, room: string | undefined, send: SendRequest): Promise<{ room: string; agent: Agent }> {
  const rooms = room ? [{ id: room }] : ((await send({ ListSessions: null })) as { SessionsListed: { sessions: { id: string }[] } }).SessionsListed.sessions
  const wanted = reference.replace(/^@/, "")
  const matches: { room: string; agent: Agent }[] = []
  for (const session of rooms) {
    const listed = await send({ ListAgents: { session_id: session.id } }) as { AgentsListed: { agents: Agent[] } }
    const activity = ((await send({ GetSessionState: { session_id: session.id } })) as Activity).SessionState.agent_activity ?? {}
    for (const agent of listed.AgentsListed.agents) {
      const state = agent.state === "Working" || activity[agent.id]?.status === "working" ? "Working" : agent.state
      if ([agent.id, agent.agent_ref, agent.alias].includes(wanted)) matches.push({ room: session.id, agent: { ...agent, state } })
      const below = (await send({ ListDescendants: { session_id: session.id, agent_id: agent.id } })) as Tree
      for (const node of below.AgentTree.tree.nodes) {
        // A sub-agent can live in another Room of the kernel: its own Room is the one DestroyAgent needs.
        if ([node.agent_id, node.alias].includes(wanted)) matches.push({ room: node.room_id ?? session.id, agent: { id: node.agent_id, alias: node.alias ?? null, state: node.state } })
      }
    }
  }
  const unique = [...new Map(matches.map(match => [match.agent.id, match])).values()]
  if (unique.length !== 1) throw new CommandFailure("usage", unique.length ? `multiple agents match '${reference}'; supply --session ROOM` : `agent '${reference}' not found`)
  return unique[0]!
}

/** What the owner's delete takes with the agent: itself and every agent below it, running or not. */
export async function subtreeOf(target: { room: string; agent: Agent }, send: SendRequest): Promise<Listed[]> {
  const tree = (await send({ ListDescendants: { session_id: target.room, agent_id: target.agent.id } })) as Tree
  return [
    { id: target.agent.id, alias: target.agent.alias ?? null, running: isRunning(target.agent.state) },
    ...tree.AgentTree.tree.nodes.map(node => ({ id: node.agent_id, alias: node.alias ?? null, running: isRunning(node.state) })),
  ]
}

export async function runAgentDelete(reference: string, room: string | undefined, send: SendRequest, options: { dryRun?: boolean } = {}): Promise<unknown> {
  const target = await findAgent(reference, room, send)
  const goes = await subtreeOf(target, send)
  if (options.dryRun) return { dry_run: true, would_delete: goes }
  const response = await send(destroyAgentRequest(target.room, target.agent.id))
  return { deleted: goes, response }
}

export async function runAgentDeleteCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "agent" || argv[1] !== "delete") return false
  let kernel: string | undefined
  try {
    if (wantsHelp(argv.slice(2))) { console.log(usageOf("agent delete")); return true }
    const { flags, positionals } = parseArguments("agent delete", argv.slice(2))
    kernel = flags.get("--kernel") as string | undefined
    const options = { dryRun: flags.has("--dry-run") }
    const run = (send: SendRequest) => runAgentDelete(positionals[0]!, flags.get("--session") as string | undefined, send, options)
    const socket = flags.get("--socket") as string | undefined
    let result: unknown
    if (socket || kernel) result = await withKernel({ socket, kernel, timeoutSeconds: 30 }, run)
    else {
      const client = new LocalIpcClient(defaultKernelEndpoint(), { controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 30000 })
      try { result = await run(body => client.send(body)) } finally { await client.close() }
    }
    console.log(JSON.stringify(result))
  } catch (error) { reportFailure(error, kernel ?? null) }
  return true
}
