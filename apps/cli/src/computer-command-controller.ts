// MP-08 / MP-11: human desktop commands share kernel authority and transport.
import type { UserDomainAccessClient } from "@chariox/kernel-client/user-domain-access"
type ComputerClient = Pick<UserDomainAccessClient, "send">
type Transport = ComputerClient & { currentClient?(): ComputerClient }
export function createComputerCommandController(deps: {client: Transport; appendNotice(message: string): void}) {
  return {async handle(args: string[]): Promise<void> {
    const [op, ...rest] = args
    if (!["start", "state", "takeover", "release", "type"].includes(op ?? "") || (op !== "type" && rest.length) || (op === "type" && !rest.length)) throw new Error("Usage: /computer start|state|takeover|release|type <text>")
    const selected = () => deps.client.currentClient?.() ?? deps.client
    const client = selected()
    const send = async (command: object) => {
      const response = await client.send<{KernelBrowser?:{result?:Record<string, unknown>};Error?:{message?:string}}>({KernelBrowser:{command:{op:"computer", command}}})
      if (client !== selected()) throw new Error("Kernel changed during desktop command; refresh desktop state.")
      if (!response.KernelBrowser?.result) throw new Error(response.Error?.message ?? "Desktop command failed.")
      return response.KernelBrowser.result
    }
    if (op === "start" || op === "state") {
      const state = await send({op})
      deps.appendNotice(`Desktop ${state.surface_id} · generation ${state.generation} · ${state.width}×${state.height}`)
      return
    }
    const state = await send({op:"state"})
    if (typeof state.surface_id !== "string" || typeof state.generation !== "string") throw new Error("Desktop is unavailable; start it first.")
    const target = {surface_id:state.surface_id, generation:state.generation}
    await send(op === "type" ? {op:"input",target,input:{kind:"text",text:rest.join(" ")}} : {op,target})
    deps.appendNotice(op === "release" ? "Desktop released to agents." : op === "type" ? "Text sent; agent desktop input is paused." : "Desktop taken over; agent desktop input is paused.")
  }}
}
