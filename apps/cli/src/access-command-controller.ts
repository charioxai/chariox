import { kernelBrowserAgentTabsMinimumProtocolVersion, LOCAL_DAEMON_PROTOCOL_VERSION } from "@chariox/kernel-client"
import { leasedComputerMinimumProtocolVersion } from "@chariox/kernel-client/ipc-requests"
import { UserDomainAccessController, roomComputerAccessMinimumProtocol, roomComputerBulkGrantMinimumProtocol, userDomainAccessMinimumProtocol, userDomainGrantExpiry, userDomainResourceLabel, userDomainUseNotice, type UserDomainAccessClient } from "@chariox/kernel-client/user-domain-access"

type AccessTransport = UserDomainAccessClient & {
  readonly socketPath?: string
  currentClient?(): UserDomainAccessClient
  onClientChanged?(handler: () => void): () => void
}

export function createAccessCommandController(deps: { client: AccessTransport; appendNotice(message: string): void }) {
  const selectedClient = () => deps.client.currentClient?.() ?? deps.client
  let selected: UserDomainAccessClient | null = null
  let adapter: UserDomainAccessClient | null = null
  const access = new UserDomainAccessController({
    client: () => {
      const client = selectedClient()
      if (client !== selected) {
        selected = client
        // Pin reads to this transport, rather than a facade that can pivot mid-request.
        adapter = {
          localDaemonProtocolVersion: client.localDaemonProtocolVersion ?? LOCAL_DAEMON_PROTOCOL_VERSION,
          send: request => client.send(request),
        }
      }
      return adapter
    },
    bindingKey: () => deps.client.socketPath ?? "owner-terminal",
  })
  let lastNotice = ""
  const unsubscribe = access.subscribe(() => {
    const notice = access.snapshot?.notice
    const key = notice ? JSON.stringify(notice) : ""
    if (key && key !== lastNotice) deps.appendNotice(userDomainUseNotice(notice!)!)
    lastNotice = key
  })
  const unsubscribeClient = deps.client.onClientChanged?.(() => access.sync())
  return {
    start: () => access.sync(),
    stop: () => { unsubscribeClient?.(); unsubscribe(); access.stop() },
    async handle(args: string[]): Promise<void> {
      // MP-08/MP-11: explicitly grant the observed desktop, never Room authority.
      if (args[0] === "grant-computer" || args[0] === "revoke-computer") {
        const granting = args[0] === "grant-computer"
        if (args.length !== (granting ? 4 : 2) || args.slice(1).some(a => !a || a === "all")) throw new Error("Usage: /access grant-computer <agent-id> <surface-id> <generation> | revoke-computer <agent-id>")
        const client = selectedClient()
        if ((client.localDaemonProtocolVersion ?? LOCAL_DAEMON_PROTOCOL_VERSION) < leasedComputerMinimumProtocolVersion) throw new Error(`Leased Computer grants require kernel protocol ${leasedComputerMinimumProtocolVersion}.`)
        const command = granting ? { op: "grant_leased_computer" as const, agent_id: args[1]!, target: { surface_id: args[2]!, generation: args[3]! } } : { op: "revoke_leased_computer" as const, agent_id: args[1]! }
        const response = await client.send<{ KernelBrowser?: { result?: unknown }; Error?: { message?: string } }>({ KernelBrowser: { command } })
        if (client !== selectedClient()) throw new Error("Kernel changed during access command; refresh access.")
        if (!response.KernelBrowser) throw new Error(response.Error?.message ?? "Leased Computer grant failed.")
        deps.appendNotice(`${granting ? "Granted" : "Revoked"} leased desktop Computer access for ${args[1]}.`)
        return
      }
      if (args.length && !(args.length === 1 && ["list", "tabs"].includes(args[0]!)) && !(args.length === 2 && ["revoke", "grant"].includes(args[0]!) && args[1])) {
        throw new Error("Usage: /access [list | tabs | revoke <agent-id>|all | grant <agent-id>|all]")
      }
      if ((selectedClient().localDaemonProtocolVersion ?? LOCAL_DAEMON_PROTOCOL_VERSION) < userDomainAccessMinimumProtocol) throw new Error(`Access grants require kernel protocol ${userDomainAccessMinimumProtocol}.`)
      access.sync()
      // A command must get fresh authority, including when an earlier feed failed.
      const client = selectedClient()
      if (args[0] === "tabs") {
        if ((client.localDaemonProtocolVersion ?? LOCAL_DAEMON_PROTOCOL_VERSION) < kernelBrowserAgentTabsMinimumProtocolVersion) throw new Error(`Agent browser tabs require kernel protocol ${kernelBrowserAgentTabsMinimumProtocolVersion} or newer.`)
        const response = await client.send<{KernelBrowser?: {result?: {tabs?: import("@chariox/kernel-client/kernel-types").KernelBrowserTab[]}}; Error?: {message?: string}}>({KernelBrowser: {command: {op: "state"}}})
        if (client !== selectedClient()) throw new Error("Kernel changed during tab command; refresh tabs.")
        const tabs = response.KernelBrowser?.result?.tabs
        if (!Array.isArray(tabs)) throw new Error(response.Error?.message ?? "Kernel browser tab inventory unavailable.")
        deps.appendNotice(["Kernel browser tabs:", ...tabs.map(tab => `${tab.tab_id} · ${tab.title || "Untitled"} · ${tab.url} · opened by ${tab.opened_by?.display_label ?? "unknown"}`), ...(tabs.length ? [] : ["No current tabs."])].join("\n"))
        return
      }
      const agentId = args[1] === "all" ? null : args[1]
      const minimum = agentId === null ? roomComputerBulkGrantMinimumProtocol : roomComputerAccessMinimumProtocol
      if (args[0] === "grant" && (client.localDaemonProtocolVersion ?? LOCAL_DAEMON_PROTOCOL_VERSION) < minimum) throw new Error(`Room Computer grants require kernel protocol ${minimum}.`)
      const response = await client.send<{ KernelBrowser?: { result?: import("@chariox/kernel-client/kernel-types").UserDomainGrantEvent }; Error?: { message?: string } }>({ KernelBrowser: { command: args[0] === "revoke"
        ? { op: "revoke_grants", agent_id: agentId }
        : args[0] === "grant" ? { op: "grant_room_computer", agent_id: agentId } : { op: "list_grants" } } })
      if (client !== selectedClient()) throw new Error("Kernel changed during access command; refresh access.")
      const snapshot = response.KernelBrowser?.result
      if (!snapshot || snapshot.event !== "user_domain_grants_changed") throw new Error(response.Error?.message ?? "Access grants require kernel protocol 494.")
      deps.appendNotice([
        args[0] === "revoke" ? `Revoked access and Room Computer control for ${agentId ?? "all agents"}.`
          : args[0] === "grant" ? (agentId === null ? "Restored Room Computer control for all agents." : `Granted Room Computer control for ${agentId}.`) : "User-domain access:",
        ...(snapshot.room_computer ?? []).map(g => `${g.agent_id} · session ${g.session_id} · Room Computer ${g.allowed ? "allowed" : "revoked"}`),
        ...snapshot.grants.map(g => `${g.agent_id} · session ${g.session_id || "pending"} · kernel ${g.kernel_id} · ${g.focused ? "focused" : "retained"}\n  resources: ${g.resources.map(userDomainResourceLabel).join(", ") || "none touched"}\n  expiry: ${userDomainGrantExpiry(g)}`),
        ...(snapshot.grants.length ? [] : ["No current grants."]),
      ].join("\n"))
    },
  }
}
