import type { SendRequest } from "./kernel-files.js"
import { CommandFailure, parseArguments, usageOf, wantsHelp } from "./agent-command-table.js"
import { resolveAgentTarget } from "./agent-deadline-command.js"
import { reportFailure, withKernel } from "./kernels-command.js"
import { defaultKernelEndpoint } from "./cli-options.js"
import { LocalIpcClient } from "./ipc.js"
import { updateAgentConfigRequest } from "./ipc-requests.js"

/**
 * `agent resume AGENT` goes on with whatever holds the agent: a usage-limit stop resumes when usage
 * is back (at once if it is back), a blocked task is resumed. `agent resume cancel AGENT` takes
 * back an armed usage resume, or cancels the blocked task. The same words are the agent command an
 * orchestrator runs; the kernel decides which of the two the agent has.
 */
export async function runAgentResume(reference: string, cancel: boolean, room: string | undefined, send: SendRequest): Promise<unknown> {
  const target = await resolveAgentTarget(reference, room, send)
  return send(updateAgentConfigRequest({
    sessionId: target.room,
    agentId: target.agent.id,
    usageLimit: { action: cancel ? "cancel" : "resume_when_back" },
  }))
}

/** `agent resume [cancel] AGENT [--session ROOM] ...`: one registered command, `cancel` is its optional word. */
export function parseAgentResumeArgv(argv: readonly string[]): { cancel: boolean; flags: Map<string, string | boolean>; positionals: string[] } {
  const cancel = argv[0] === "cancel"
  const { flags, positionals } = parseArguments("agent resume", argv.slice(cancel ? 1 : 0))
  return { cancel, flags, positionals }
}

export async function runAgentResumeCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "agent" || argv[1] !== "resume") return false
  let kernel: string | undefined
  try {
    const rest = argv.slice(2)
    if (wantsHelp(rest)) { console.log(usageOf("agent resume")); return true }
    const { cancel, flags, positionals } = parseAgentResumeArgv(rest)
    kernel = flags.get("--kernel") as string | undefined
    const run = (send: SendRequest) => runAgentResume(positionals[0]!, cancel, flags.get("--session") as string | undefined, send)
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
