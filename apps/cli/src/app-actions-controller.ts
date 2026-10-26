import { inspectAppViewRequest } from "@chariox/kernel-client/ipc-requests"
import { appViewCatalogJson } from "@chariox/kernel-client/shell-app-command"
import type { AppViewInspection } from "@chariox/kernel-client/kernel-types"

type Client = { send(request: any): Promise<any> }
/** The DEV toggle projects the shared catalog. It does not invoke App actions. */
export function createAppActionsController(deps: {
  client(): Client; session(): string | undefined
  show(text: string): void; hide(): void; scroll(direction: -1 | 1): void
  notify(message: string): void
}) {
  let open = false, revision = 0
  let bound: { client: Client; session: string; target: string; generation: string } | undefined
  const hide = () => { revision++; open = false; bound = undefined; deps.hide() }
  const current = () => {
    if (bound && (bound.client !== deps.client() || bound.session !== deps.session())) hide()
  }
  async function refresh() {
    current()
    const client = deps.client(), session = deps.session(), previous = bound
    if (!session) throw new Error("Attach to the DEV Room before toggling the agent view")
    const epoch = ++revision
    const response = await client.send(inspectAppViewRequest(session, previous?.target))
    if (epoch !== revision || client !== deps.client() || session !== deps.session()) return
    const view = response.AppViewInspected as AppViewInspection | undefined
    if (!view) throw new Error(response.Error?.message ?? "The kernel did not return an App view inspection")
    if (!view.development) throw new Error("The agent-view panel is available only on a DEV installation")
    if (previous && view.generation !== previous.generation) {
      hide(); throw new Error("The DEV view changed generation; toggle the panel again")
    }
    bound = { client, session, target: view.target_id, generation: view.generation }
    open = true
    deps.show(`As the agent sees it\n${appViewCatalogJson(view)}`)
  }
  return {
    hide, current,
    ownsInput() { current(); return open },
    async handle(args: string[]) {
      if (args[0] !== "actions" || args[1] !== "toggle") return false
      if (args.length !== 2) throw new Error("usage: /app actions toggle")
      current()
      if (open) hide()
      else await refresh()
      return true
    },
    handleKey(event: { name: string; ctrl?: boolean; meta?: boolean; preventDefault?(): void }) {
      current()
      if (!open || event.name === "f8") return false
      event.preventDefault?.()
      if (event.name === "escape" || event.name === "a") hide()
      else if (event.name === "pageup" || event.name === "pagedown") deps.scroll(event.name === "pageup" ? -1 : 1)
      else if (event.name === "r") void refresh().catch(error => { hide(); deps.notify(error instanceof Error ? error.message : "App inspection failed; toggle it again") })
      return true
    },
  }
}
