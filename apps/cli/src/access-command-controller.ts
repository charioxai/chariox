import { UserDomainAccessController, userDomainGrantExpiry, userDomainResourceLabel, userDomainUseNotice, type UserDomainAccessClient } from "@chariox/kernel-client/user-domain-access"

export function createAccessCommandController(deps: { client: UserDomainAccessClient; appendNotice(message: string): void }) {
  const access = new UserDomainAccessController({ client: () => deps.client, bindingKey: () => "owner-terminal" })
  let lastNotice = ""
  const unsubscribe = access.subscribe(() => {
    const notice = access.snapshot?.notice
    const key = notice ? JSON.stringify(notice) : ""
    if (key && key !== lastNotice) deps.appendNotice(userDomainUseNotice(notice!)!)
    lastNotice = key
  })
  return {
    start: () => access.sync(),
    stop: () => { unsubscribe(); access.stop() },
    async handle(args: string[]): Promise<void> {
      if (args.length && !(args.length === 1 && args[0] === "list") && !(args.length === 2 && args[0] === "revoke" && args[1])) {
        throw new Error("Usage: /access [list | revoke <agent-id>|all]")
      }
      access.sync()
      // A command must get fresh authority, including when an earlier feed failed.
      const response = await deps.client.send<{ KernelBrowser?: { result?: import("@chariox/kernel-client/kernel-types").UserDomainGrantEvent }; Error?: { message?: string } }>({ KernelBrowser: { command: args[0] === "revoke"
        ? { op: "revoke_grants", agent_id: args[1] === "all" ? null : args[1] }
        : { op: "list_grants" } } })
      const snapshot = response.KernelBrowser?.result
      if (!snapshot || snapshot.event !== "user_domain_grants_changed") throw new Error(response.Error?.message ?? "Access grants require kernel protocol 432.")
      deps.appendNotice([
        args[0] === "revoke" ? `Revoked access for ${args[1]}.` : "User-domain access:",
        ...snapshot.grants.map(g => `${g.agent_id} · session ${g.session_id || "pending"} · kernel ${g.kernel_id} · ${g.focused ? "focused" : "retained"}\n  resources: ${g.resources.map(userDomainResourceLabel).join(", ") || "none touched"}\n  expiry: ${userDomainGrantExpiry(g)}`),
        ...(snapshot.grants.length ? [] : ["No current grants."]),
      ].join("\n"))
    },
  }
}
