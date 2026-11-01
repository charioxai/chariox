import { LocalIpcError } from "@chariox/kernel-client/ipc"
import type { GatewayKernelRecord } from "@chariox/kernel-client/kernel-types"

type FooterTone = "info" | "error"

type LinkedKernelGateway = {
  joinKernelLink: (link: string, relayToken: string | null) => Promise<GatewayKernelRecord>
  listGatewayKernels: () => Promise<GatewayKernelRecord[]>
  forgetGatewayKernel: (name: string) => Promise<void>
}

export type LinkedKernelCommandHandlerDeps = Partial<LinkedKernelGateway> & {
  flashFooter: (message: string, tone: FooterTone, display?: "toast") => void
}

const ADD_USAGE = "usage: /kernel add <link> [--relay-token-env VAR]"
const ENVIRONMENT_VARIABLE_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/

export function isLinkedKernelAdd(args: readonly string[]): boolean {
  return args[0] === "add"
}

export function isLinkedKernelCommand(args: readonly string[]): boolean {
  return args[0] === "add" || args[0] === "linked" || args[0] === "unlink"
}

export async function handleLinkedKernelSlashCommand(
  deps: LinkedKernelCommandHandlerDeps,
  args: readonly string[],
): Promise<void> {
  const { joinKernelLink, listGatewayKernels, forgetGatewayKernel } = deps
  if (!joinKernelLink || !listGatewayKernels || !forgetGatewayKernel) {
    deps.flashFooter("linked kernels are unavailable in this build", "error", "toast")
    return
  }
  const kernel = { joinKernelLink, listGatewayKernels, forgetGatewayKernel }
  try {
    if (args[0] === "add") await addLinkedKernel(deps, kernel, args.slice(1))
    else if (args[0] === "linked") await listLinkedKernels(deps, kernel)
    else await unlinkKernel(deps, kernel, args[1])
  } catch (error) {
    // The kernel's refusal already says what to do next; the transport wrapper around it is noise.
    const message = error instanceof LocalIpcError ? error.message.replace(/^kernel transport `[^`]*` failed: /, "") : error instanceof Error ? error.message : String(error)
    deps.flashFooter(message, "error", "toast")
  }
}

// Neither the link nor the token may appear in a message, so every usage and error below names neither.
async function addLinkedKernel(deps: LinkedKernelCommandHandlerDeps, kernel: LinkedKernelGateway, args: readonly string[]): Promise<void> {
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
      deps.flashFooter(`environment variable ${variable} is not set in this terminal; export it with the operator's client token and run /kernel add again`, "error", "toast")
      return
    }
  }
  const joined = await kernel.joinKernelLink(link, relayToken)
  deps.flashFooter(`Paired with ${joined.name} (${joined.status})`, "info", "toast")
}

async function listLinkedKernels(deps: LinkedKernelCommandHandlerDeps, kernel: LinkedKernelGateway): Promise<void> {
  const kernels = await kernel.listGatewayKernels()
  if (kernels.length === 0) {
    deps.flashFooter("no linked kernels; pair one with /kernel add <link>", "info", "toast")
    return
  }
  deps.flashFooter(kernels.map(entry => `${entry.name} ${entry.status} via=${entry.via}`).join("\n"), "info", "toast")
}

async function unlinkKernel(deps: LinkedKernelCommandHandlerDeps, kernel: LinkedKernelGateway, name: string | undefined): Promise<void> {
  if (!name) {
    deps.flashFooter("usage: /kernel unlink <name>", "error", "toast")
    return
  }
  await kernel.forgetGatewayKernel(name)
  deps.flashFooter(`Forgot linked kernel ${name}`, "info", "toast")
}
