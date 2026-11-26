import type { SendRequest } from "./kernel-files.js"
import { CommandFailure, parseArguments, usageOf, wantsHelp } from "./agent-command-table.js"
import { resolveAgentTarget } from "./agent-deadline-command.js"
import { reportFailure, withAgentKernel } from "./kernels-command.js"
import { setAgentParentRequest } from "./ipc-requests.js"

/**
 * `agent parent AGENT PARENT` moves an agent under a new parent, `none` makes it a Room agent: the
 * owner's move in the agent tree, with its sub-agents, history and context. The same words are the
 * agent command an orchestrator runs; the kernel refuses a cycle and an agent of another Room.
 */
export async function runAgentParent(reference: string, parent: string, room: string | undefined, send: SendRequest): Promise<unknown> {
  const target = await resolveAgentTarget(reference, room, send)
  const under = parent === "none" ? null : (await resolveAgentTarget(parent, target.room, send)).agent.id
  return send(setAgentParentRequest(target.room, target.agent.id, under))
}

export async function runAgentParentCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "agent" || argv[1] !== "parent") return false
  let kernel: string | undefined
  try {
    if (wantsHelp(argv.slice(2))) { console.log(usageOf("agent parent")); return true }
    const { flags, positionals } = parseArguments("agent parent", argv.slice(2))
    if (positionals.length !== 2) throw new CommandFailure("usage", usageOf("agent parent"))
    kernel = flags.get("--kernel") as string | undefined
    const run = (send: SendRequest) => runAgentParent(positionals[0]!, positionals[1]!, flags.get("--session") as string | undefined, send)
    const socket = flags.get("--socket") as string | undefined
    console.log(JSON.stringify(await withAgentKernel({ socket, kernel }, run)))
  } catch (error) { reportFailure(error, kernel ?? null) }
  return true
}
