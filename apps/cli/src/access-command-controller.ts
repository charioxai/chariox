import { kernelBrowserAgentTabsMinimumProtocolVersion, LOCAL_DAEMON_PROTOCOL_VERSION } from "@chariox/kernel-client"
import { leasedComputerMinimumProtocolVersion } from "@chariox/kernel-client/ipc-requests"

type AccessClient = {
  readonly localDaemonProtocolVersion?: number
  send<T>(request: unknown): Promise<T>
}
type AccessTransport = AccessClient & { currentClient?(): AccessClient }

// The owner's agents need no grant for the kernel browser, the Computer or an App, so `/access` only lists the
// kernel browser's tabs and gives or takes back a leased worker's desktop for one agent.
export function createAccessCommandController(deps: { client: AccessTransport; appendNotice(message: string): void }) {
  const selectedClient = () => deps.client.currentClient?.() ?? deps.client
  return {
    async handle(args: string[]): Promise<void> {
      // MP-08/MP-11: explicitly grant the observed desktop, never Room authority.
      if (args[0] === "grant-computer" || args[0] === "revoke-computer") {
        const granting = args[0] === "grant-computer"
        if (args.length !== (granting ? 4 : 2) || args.slice(1).some(a => !a || a === "all")) throw new Error("Usage: /access tabs | grant-computer <agent-id> <surface-id> <generation> | revoke-computer <agent-id>")
        const client = selectedClient()
        if ((client.localDaemonProtocolVersion ?? LOCAL_DAEMON_PROTOCOL_VERSION) < leasedComputerMinimumProtocolVersion) throw new Error(`Leased Computer grants require kernel protocol ${leasedComputerMinimumProtocolVersion}.`)
        const command = granting ? { op: "grant_leased_computer" as const, agent_id: args[1]!, target: { surface_id: args[2]!, generation: args[3]! } } : { op: "revoke_leased_computer" as const, agent_id: args[1]! }
        const response = await client.send<{ KernelBrowser?: { result?: unknown }; Error?: { message?: string } }>({ KernelBrowser: { command } })
        if (client !== selectedClient()) throw new Error("Kernel changed during access command; refresh access.")
        if (!response.KernelBrowser) throw new Error(response.Error?.message ?? "Leased Computer grant failed.")
        deps.appendNotice(`${granting ? "Granted" : "Revoked"} leased desktop Computer access for ${args[1]}.`)
        return
      }
      if (!(args.length === 1 && args[0] === "tabs")) throw new Error("Usage: /access tabs | grant-computer <agent-id> <surface-id> <generation> | revoke-computer <agent-id>")
      const client = selectedClient()
      if ((client.localDaemonProtocolVersion ?? LOCAL_DAEMON_PROTOCOL_VERSION) < kernelBrowserAgentTabsMinimumProtocolVersion) throw new Error(`Agent browser tabs require kernel protocol ${kernelBrowserAgentTabsMinimumProtocolVersion} or newer.`)
      const response = await client.send<{KernelBrowser?: {result?: {tabs?: import("@chariox/kernel-client/kernel-types").KernelBrowserTab[]}}; Error?: {message?: string}}>({KernelBrowser: {command: {op: "state"}}})
      if (client !== selectedClient()) throw new Error("Kernel changed during tab command; refresh tabs.")
      const tabs = response.KernelBrowser?.result?.tabs
      if (!Array.isArray(tabs)) throw new Error(response.Error?.message ?? "Kernel browser tab inventory unavailable.")
      deps.appendNotice(["Kernel browser tabs:", ...tabs.map(tab => `${tab.tab_id} · ${tab.title || "Untitled"} · ${tab.url} · opened by ${tab.opened_by?.display_label ?? "unknown"}`), ...(tabs.length ? [] : ["No current tabs."])].join("\n"))
    },
  }
}
