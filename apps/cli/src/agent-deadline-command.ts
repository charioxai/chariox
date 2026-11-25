import type { SendRequest } from "./kernel-files.js"
import { CommandFailure, parseArguments, usageOf, wantsHelp } from "./agent-command-table.js"
import { reportFailure, withAgentKernel } from "./kernels-command.js"

function deadlineOf(value: string, now: number): number | null {
  if (value === "none") return null
  const match = /^(\d+)(s|m|h|d)$/.exec(value)
  const units: Record<string, number> = { s: 1000, m: 60000, h: 3600000, d: 86400000 }
  const duration = match ? Number(match[1]) * units[match[2]!]! : 0
  if (duration <= 0 || !Number.isSafeInteger(now + duration)) {
    throw new CommandFailure("usage", "a deadline is a positive duration such as 90s, 30m, 2h or 1d, or none")
  }
  return now + duration
}

/** The one agent `reference` names (id, ref or alias) in `room`, or in every room when none is given. A sub-agent is in no room's
 * agent list, so the agents below each room agent are read too, and an id given with its room is taken as it is. */
export async function resolveAgentTarget(reference: string, room: string | undefined, send: SendRequest): Promise<{ room: string; agent: { id: string; agent_ref?: string; alias?: string | null } }> {
  const rooms = room ? [{ id: room }] : ((await send({ ListSessions: null })) as { SessionsListed: { sessions: { id: string }[] } }).SessionsListed.sessions
  type Agent = { id: string; agent_ref?: string; alias?: string | null }
  const wanted = reference.replace(/^@/, "")
  const named = (agent: Agent) => [agent.id, agent.agent_ref, agent.alias].includes(wanted)
  const matches: { room: string; agent: Agent }[] = []
  const roomAgents: { room: string; agent: Agent }[] = []
  for (const session of rooms) {
    const listed = await send({ ListAgents: { session_id: session.id } }) as { AgentsListed: { agents: Agent[] } }
    for (const agent of listed.AgentsListed.agents) {
      roomAgents.push({ room: session.id, agent })
      if (named(agent)) matches.push({ room: session.id, agent })
    }
  }
  // A room agent of one room and a sub-agent of another can share a name: every match counts, so a name that is not one agent is refused.
  for (const { room: roomId, agent: parent } of roomAgents) {
    const tree = await send({ ListDescendants: { session_id: roomId, agent_id: parent.id } }).catch(() => null) as { AgentTree?: { tree: { nodes: { agent_id: string; alias?: string | null; room_id?: string | null }[] } } } | null
    for (const node of tree?.AgentTree?.tree.nodes ?? []) {
      if ([node.agent_id, node.alias].includes(wanted)) matches.push({ room: node.room_id ?? roomId, agent: { id: node.agent_id, alias: node.alias ?? null } })
    }
  }
  // An agent id with its room is enough: the kernel refuses an id it does not know.
  if (matches.length === 0 && room && /^agent-/.test(wanted)) matches.push({ room, agent: { id: wanted } })
  const unique = [...new Map(matches.map(match => [`${match.room}/${match.agent.id}`, match])).values()]
  if (unique.length !== 1) throw new CommandFailure("usage", unique.length ? `multiple agents match '${reference}'; supply --session ROOM` : `agent '${reference}' not found`)
  return unique[0]!
}

export async function runAgentDeadline(reference: string, value: string, room: string | undefined, send: SendRequest, now = Date.now()): Promise<unknown> {
  const deadlineMs = deadlineOf(value, now)
  const target = await resolveAgentTarget(reference, room, send)
  return send({ SetAgentDeadline: { session_id: target.room, agent_id: target.agent.id, deadline_ms: deadlineMs } })
}

export async function runAgentDeadlineCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "agent") return false
  let kernel: string | undefined
  try {
    if (argv[1] !== "deadline") throw new CommandFailure("usage", usageOf("agent deadline"))
    if (wantsHelp(argv.slice(2))) { console.log(usageOf("agent deadline")); return true }
    const { flags, positionals } = parseArguments("agent deadline", argv.slice(2))
    kernel = flags.get("--kernel") as string | undefined
    const run = (send: SendRequest) => runAgentDeadline(positionals[0]!, positionals[1]!, flags.get("--session") as string | undefined, send)
    const socket = flags.get("--socket") as string | undefined
    const result = await withAgentKernel({ socket, kernel }, run)
    console.log(JSON.stringify(result))
  } catch (error) { reportFailure(error, kernel ?? null) }
  return true
}
