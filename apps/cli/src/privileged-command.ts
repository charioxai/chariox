import { CommandFailure, parseArguments, usageOf } from "./agent-command-table.js"
import { reportFailure, withAgentKernel } from "./kernels-command.js"

export function parsePrivilegedRun(argv: readonly string[]) {
  const separator = argv.indexOf("--")
  if (separator < 0 || separator === argv.length - 1) throw new CommandFailure("usage", `${usageOf("privileged run")} -- argv...`)
  const { flags } = parseArguments("privileged run", argv.slice(0, separator))
  const id = flags.get("--entry")
  if (typeof id !== "string" || !id.trim()) throw new CommandFailure("usage", "--entry needs a Vault entry id or unique name")
  const timeout = flags.get("--timeout")
  const timeout_seconds = timeout === undefined ? undefined : Number(timeout)
  if (timeout_seconds !== undefined && (!Number.isSafeInteger(timeout_seconds) || timeout_seconds < 1 || timeout_seconds > 240)) {
    throw new CommandFailure("usage", "--timeout needs an integer from 1 to 240 seconds")
  }
  return {
    request: { OwnerVault: { action: "run_privileged", id, argv: argv.slice(separator + 1),
      ...(flags.has("--machine") ? { machine: flags.get("--machine") } : {}),
      ...(timeout_seconds === undefined ? {} : { timeout_seconds }),
    } },
    options: { socket: flags.get("--socket") as string | undefined, commandId: flags.get("--command-id") as string | undefined },
  }
}

export async function runPrivilegedCommand(argv: readonly string[]): Promise<boolean> {
  if (argv[0] !== "privileged") return false
  try {
    if ((argv.length === 2 || argv.length === 3) && argv.at(-1) === "--help") {
      console.log(`${usageOf("privileged run")} -- argv...\nThe kernel supplies the machine-bound sudo password from the Vault. Output is JSON with masked stdout and stderr.`)
      return true
    }
    if (argv[1] !== "run") throw new CommandFailure("usage", `${usageOf("privileged run")} -- argv...`)
    const { request, options } = parsePrivilegedRun(argv.slice(2))
    const reply = await withAgentKernel(options, send => send(request)) as { OwnerVault?: { result?: { action?: string; outcome?: { exit_code: number | null } } } }
    const result = reply.OwnerVault?.result
    if (result?.action !== "privileged_run" || !result.outcome) throw new Error("kernel returned no privileged run outcome")
    console.log(JSON.stringify(result.outcome))
    process.exitCode = result.outcome.exit_code ?? 1
  } catch (error) { reportFailure(error) }
  return true
}
