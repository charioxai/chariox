import { LocalIpcError } from "@chariox/kernel-client/ipc"
import type { GatewayMachineRecord } from "@chariox/kernel-client/kernel-types"

type FooterTone = "info" | "error"

type LinkedMachineKernel = {
  joinMachineLink: (link: string, relayToken: string | null) => Promise<GatewayMachineRecord>
  listGatewayMachines: () => Promise<GatewayMachineRecord[]>
  forgetGatewayMachine: (name: string) => Promise<void>
}

export type LinkedMachineCommandHandlerDeps = Partial<LinkedMachineKernel> & {
  flashFooter: (message: string, tone: FooterTone, display?: "toast") => void
}

const ADD_USAGE = "usage: /machine add <link> [--relay-token-env VAR]"
const ENVIRONMENT_VARIABLE_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/

/** `/machine add ssh …` stays the SSH installer; anything else after `add` is a pairing link. */
export function isLinkedMachineAdd(args: readonly string[]): boolean {
  return args[0] === "add" && args[1] !== "ssh"
}

export async function handleLinkedMachineSlashCommand(
  deps: LinkedMachineCommandHandlerDeps,
  args: readonly string[],
): Promise<void> {
  const { joinMachineLink, listGatewayMachines, forgetGatewayMachine } = deps
  if (!joinMachineLink || !listGatewayMachines || !forgetGatewayMachine) {
    deps.flashFooter("linked machines are unavailable in this build", "error", "toast")
    return
  }
  const kernel = { joinMachineLink, listGatewayMachines, forgetGatewayMachine }
  try {
    if (args[0] === "add") await addLinkedMachine(deps, kernel, args.slice(1))
    else if (args[0] === "linked") await listLinkedMachines(deps, kernel)
    else await unlinkMachine(deps, kernel, args[1])
  } catch (error) {
    // The kernel's refusal already says what to do next; the transport wrapper around it is noise.
    const message = error instanceof LocalIpcError ? error.message.replace(/^kernel transport `[^`]*` failed: /, "") : error instanceof Error ? error.message : String(error)
    deps.flashFooter(message, "error", "toast")
  }
}

// Neither the link nor the token may appear in a message, so every usage and error below names neither.
async function addLinkedMachine(deps: LinkedMachineCommandHandlerDeps, kernel: LinkedMachineKernel, args: readonly string[]): Promise<void> {
  const [link, flag, variable, ...extra] = args
  if (!link || extra.length > 0 || (flag !== undefined && (flag !== "--relay-token-env" || variable === undefined))) {
    deps.flashFooter(ADD_USAGE, "error", "toast")
    return
  }
  let relayToken: string | null = null
  if (variable !== undefined) {
    // A pasted token is not a variable name; refusing it here keeps it out of every message.
    if (!ENVIRONMENT_VARIABLE_NAME.test(variable)) {
      deps.flashFooter("--relay-token-env needs the NAME of an environment variable of this terminal, not the token itself", "error", "toast")
      return
    }
    relayToken = process.env[variable] || null
    if (relayToken === null) {
      deps.flashFooter(`environment variable ${variable} is not set in this terminal; export it with the operator's client token and run /machine add again`, "error", "toast")
      return
    }
  }
  const machine = await kernel.joinMachineLink(link, relayToken)
  deps.flashFooter(`Paired with ${machine.name} (${machine.status})`, "info", "toast")
}

async function listLinkedMachines(deps: LinkedMachineCommandHandlerDeps, kernel: LinkedMachineKernel): Promise<void> {
  const machines = await kernel.listGatewayMachines()
  if (machines.length === 0) {
    deps.flashFooter("no linked machines; pair one with /machine add <link>", "info", "toast")
    return
  }
  deps.flashFooter(machines.map(machine => `${machine.name} ${machine.status} via=${machine.via}`).join("\n"), "info", "toast")
}

async function unlinkMachine(deps: LinkedMachineCommandHandlerDeps, kernel: LinkedMachineKernel, name: string | undefined): Promise<void> {
  if (!name) {
    deps.flashFooter("usage: /machine unlink <name>", "error", "toast")
    return
  }
  await kernel.forgetGatewayMachine(name)
  deps.flashFooter(`Forgot linked machine ${name}`, "info", "toast")
}
