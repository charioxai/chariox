import type { SendRequest } from "./kernel-files.js"
import { CommandFailure, parseArguments, usageOf, wantsHelp } from "./agent-command-table.js"
import { reportFailure, withKernel } from "./kernels-command.js"
import { defaultKernelEndpoint } from "./cli-options.js"
import { LocalIpcClient } from "./ipc.js"

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

export async function runAgentDeadline(reference: string, value: string, room: string | undefined, send: SendRequest, now = Date.now()): Promise<unknown> {
  const deadlineMs = deadlineOf(value, now)
  const rooms = room ? [{ id: room }] : ((await send({ ListSessions: null })) as { SessionsListed: { sessions: { id: string }[] } }).SessionsListed.sessions
  type Agent = { id: string; agent_ref?: string; alias?: string | null }
  const matches: { room: string; agent: Agent }[] = []
  for (const session of rooms) {
    const listed = await send({ ListAgents: { session_id: session.id } }) as { AgentsListed: { agents: Agent[] } }
    for (const agent of listed.AgentsListed.agents) {
      if ([agent.id, agent.agent_ref, agent.alias].includes(reference.replace(/^@/, ""))) matches.push({ room: session.id, agent })
    }
  }
  if (matches.length !== 1) throw new CommandFailure("usage", matches.length ? `multiple agents match '${reference}'; supply --session ROOM` : `agent '${reference}' not found`)
  const target = matches[0]!
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
