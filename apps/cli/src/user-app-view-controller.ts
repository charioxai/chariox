import { openUserAppViewRequest, listUserAppViewsRequest, closeUserAppViewRequest,
  callUserAppViewRequest, subscribeUserAppViewsRequest, answerUserDomainInteractionRequest } from "@chariox/kernel-client/ipc-requests"
import { kernelBrowserRequest } from "@chariox/kernel-client/ipc-requests"
import type { UserAppView, KernelBrowserInput } from "@chariox/kernel-client/kernel-types"
import type { PasskeyPopupKey, PasskeyPopupPaste } from "./passkey-popup-controller.js"
import { appViewText, formatUserAppViewOutline } from "./user-app-view-outline.js"

export type UserAppViewClient = { send(request: any): Promise<any> }
export type UserAppViewPanel = { open: boolean; text: string }
type BoundView = { client: UserAppViewClient; view: UserAppView }
const usage = "/app views | /app view open INSTALLATION | show VIEW | close [VIEW] | call METHOD 'JSON' | approvals | answer INTERACTION CHOICE"
function field(response: any, name: string): any {
  if (!response || !(name in response)) throw new Error("The kernel did not confirm the App request")
  return response[name]
}
function viewValue(value: any): UserAppView {
  if (!value || [value.view_id, value.installation_id, value.generation, value.origin].some(v => typeof v !== "string" || !v)
    || (value.browser && (typeof value.browser.tab_id !== "string" || !value.browser.tab_id || (!Number.isSafeInteger(value.browser.generation) || value.browser.generation < 1))))
    throw new Error("The kernel returned an invalid App view")
  return value
}

/** Terminal projection only. Hosting, App calls and approvals stay in the kernel.
 * Each operation captures the admitted client, never the mutable kernel selector. */
export function createUserAppViewController(deps: {
  client(): UserAppViewClient
  onView(view: UserAppViewPanel): void
  onOpen(): void
  onClose(): void
  scroll(direction: -1 | 1): void
  notify(message: string): void
}) {
  let open = false, disposed = false, text = "", active: BoundView | undefined
  const created = new Set<BoundView>()
  let work: Promise<void> = Promise.resolve(), queued = 0
  let refreshPending = false
  const emit = () => deps.onView({ open, text })
  const current = (client: UserAppViewClient) => {
    if (disposed || deps.client() !== client) throw new Error("This App view belongs to the previous kernel; select a view on the current kernel")
  }
  const show = (value: string) => { text = value; if (!open) { open = true; deps.onOpen() }; emit() }
  const hide = () => { if (open) { open = false; deps.onClose(); emit() } }
  async function list(client: UserAppViewClient): Promise<UserAppView[]> {
    current(client)
    const result = field(await client.send(listUserAppViewsRequest()), "UserAppViewsListed")
    current(client)
    if (!Array.isArray(result.views)) throw new Error("The kernel returned an invalid App view list")
    return result.views.map(viewValue)
  }
  async function owned(bound: BoundView) {
    const found = (await list(bound.client)).find(view => view.view_id === bound.view.view_id)
    if (!found || found.generation !== bound.view.generation
      || found.browser?.tab_id !== bound.view.browser?.tab_id || found.browser?.generation !== bound.view.browser?.generation)
      throw new Error("This App view has ended; open a new view")
    return found
  }
  async function refresh(bound: BoundView) {
    const view = await owned(bound)
    if (!view.browser) throw new Error("This view uses native rendering. Open a new TUI view with /app view open INSTALLATION")
    const result = field(await bound.client.send(kernelBrowserRequest({ op: "snapshot", ...view.browser })), "KernelBrowser").result
    current(bound.client)
    if (result?.generation !== view.browser.generation) throw new Error("The App browser generation has ended")
    if (active === bound && open) { text = formatUserAppViewOutline(view, result.snapshot); emit() }
  }
  function enqueue(task: () => Promise<void>) {
    if (queued >= 32) { deps.notify("App input is busy; wait for the view to catch up"); return }
    queued++
    work = work.then(task).catch(() => deps.notify("App action failed; check the current view with /app views")).finally(() => { queued-- })
  }
  async function input(bound: BoundView, value: KernelBrowserInput) {
    if (!open || active !== bound || disposed) return
    const view = await owned(bound)
    if (!open || active !== bound || !view.browser) return
    field(await bound.client.send(kernelBrowserRequest({ op: "input", ...view.browser, input: value })), "KernelBrowser")
    current(bound.client)
    await refresh(bound)
  }
  async function close(id?: string) {
    const bound = id ? { client: deps.client(), view: (await list(deps.client())).find(v => v.view_id === id)! } : active
    if (!bound?.view) throw new Error("Select an App view first")
    await owned(bound)
    const result = field(await bound.client.send(closeUserAppViewRequest(bound.view.view_id)), "UserAppViewClosed")
    if (result.view_id !== bound.view.view_id) throw new Error("The kernel did not confirm the closed view")
    for (const item of created) if (item.client === bound.client && item.view.view_id === bound.view.view_id) created.delete(item)
    if (active?.view.view_id === bound.view.view_id && active.client === bound.client) { active = undefined; hide() }
    deps.notify("App view closed")
  }
  async function handle(args: string[], sessionId?: string): Promise<boolean> {
    const waitingOpen = !sessionId && args[0] === "open" && args.length === 2
    if (!waitingOpen && args[0] !== "view" && args[0] !== "views") return false
    const client = deps.client()
    const [action, ...rest] = waitingOpen ? ["open", args[1]!] : args[0] === "views" ? ["list", ...args.slice(1)] : args.slice(1)
    if (action === "list" && !rest.length) {
      const views = await list(client); active = undefined
      show(["User-domain App views", ...views.map(v => `${appViewText(v.view_id)}  ${appViewText(v.installation_id)}  ${v.browser ? "text projection" : "client native"}`),
        ...(views.length ? [] : ["No open views"]), "Use /app view show VIEW or /app view open INSTALLATION"].join("\n"))
    } else if (action === "open" && rest.length === 1) {
      const bound = { client, view: viewValue(field(await client.send(openUserAppViewRequest(rest[0]!, "kernel_browser")), "UserAppViewOpened").view) }
      created.add(bound) // Original client owns cleanup even if selection changed while opening.
      try { current(client); active = bound; show("Opening App text projection…"); await refresh(bound) }
      catch (error) {
        await client.send(closeUserAppViewRequest(bound.view.view_id)).catch(() => {})
        created.delete(bound)
        if (active === bound) { active = undefined; hide() }
        throw error
      }
    } else if (action === "show" && rest.length === 1) {
      const view = (await list(client)).find(v => v.view_id === rest[0])
      if (!view) throw new Error("No such App view on this kernel")
      if (!view.browser) throw new Error("This view uses native rendering. Open a new TUI view with /app view open INSTALLATION")
      active = { client, view }; show("Reading App text projection…"); await refresh(active)
    } else if (action === "close" && rest.length <= 1) { await close(rest[0])
    } else if (action === "call" && rest.length === 2) {
      const bound = active
      if (!bound) throw new Error("Select an App view first")
      if (rest[0]!.length > 256 || rest[1]!.length > 16384) throw new Error("App action is too large")
      await owned(bound)
      const result = field(await bound.client.send(callUserAppViewRequest(bound.view.view_id, rest[0]!, JSON.parse(rest[1]!))), "UserAppViewCallResult")
      current(bound.client)
      if (result.error) throw new Error("The App refused this action")
      deps.notify(appViewText(JSON.stringify(result.result), 2048))
    } else if (action === "approvals" && !rest.length) {
      const result = field(await client.send(subscribeUserAppViewsRequest(undefined, 0)), "UserAppViewsChanged")
      current(client); active = undefined
      if (!Array.isArray(result.interactions)) throw new Error("Invalid App approval list")
      show(["Kernel approvals · user domain", ...result.interactions.map((item: any) => `${appViewText(item.id)}: ${appViewText(item.title)}\n${appViewText(item.message)}\n${item.level === "critical" ? "Critical: use F8 for the trusted passkey popup" : (item.choices ?? []).map((c: any) => `${appViewText(c.id)}: ${appViewText(c.label)}`).join(" · ")}`)].join("\n\n"))
    } else if (action === "answer" && rest.length === 2) {
      const result = field(await client.send(subscribeUserAppViewsRequest(undefined, 0)), "UserAppViewsChanged")
      current(client)
      const interaction = result.interactions?.find((item: any) => item.id === rest[0])
      if (!interaction || !interaction.choices?.some((c: any) => c.id === rest[1])) throw new Error("No such pending App approval choice")
      if (interaction.level === "critical") throw new Error("Use F8: passkeys belong in the trusted Chariox popup")
      const answer = field(await client.send(answerUserDomainInteractionRequest({ interactionId: rest[0]!, choiceId: rest[1]! })), "UserDomainInteractionAnswered")
      if (answer.interaction_id !== rest[0]) throw new Error("The kernel did not confirm the approval answer")
      deps.notify("App approval answered")
    } else throw new Error(`usage: ${usage}`)
    return true
  }
  const keys: Record<string, string> = { tab: "Tab", return: "Enter", enter: "Enter", backspace: "Backspace", delete: "Delete", left: "ArrowLeft", right: "ArrowRight", up: "ArrowUp", down: "ArrowDown", home: "Home", end: "End" }
  function handleKey(event: PasskeyPopupKey) {
    if (!open || event.defaultPrevented) return false
    event.preventDefault(); event.stopPropagation()
    if (event.eventType === "release") return true
    if (event.name === "escape") hide()
    else if (event.ctrl && event.name === "w") enqueue(() => close())
    else if (event.name === "pageup" || event.name === "pagedown") deps.scroll(event.name === "pageup" ? -1 : 1)
    else if (!event.ctrl && !event.meta && !event.alt && active) {
      const bound = active, key = keys[event.name]
      if (event.shift && key) { deps.notify("Use unmodified navigation keys in this prototype"); return true }
      if (key) enqueue(() => input(bound, { kind: "key", key }))
      else {
        const value = event.sequence || (event.name === "space" ? " " : Array.from(event.name).length === 1 ? (event.shift ? event.name.toUpperCase() : event.name) : "")
        if (value && value.length <= 16384 && !/[\u0000-\u001f\u007f-\u009f]/.test(value)) enqueue(() => input(bound, { kind: "text", text: value }))
      }
    }
    return true
  }
  function handlePaste(event: PasskeyPopupPaste) {
    if (!open) return false
    event.preventDefault(); event.stopPropagation()
    const value = event.rawText ?? event.text, bound = active
    if (bound && value.length <= 16384 && !/[\u0000-\u001f\u007f-\u009f]/.test(value)) enqueue(() => input(bound, { kind: "text", text: value }))
    else deps.notify("App paste refused: use one line without terminal control characters")
    return true
  }
  // Read-only accessibility updates while visible; no restore after disconnect/restart.
  const timer = setInterval(() => {
    if (!open || !active || refreshPending || queued) return
    refreshPending = true
    void refresh(active).catch(() => { hide(); deps.notify("App view unavailable; select a view on the current kernel") }).finally(() => { refreshPending = false })
  }, 750)
  timer.unref()
  return { handle, handleKey, handlePaste, hide, ownsInput: () => open, view: () => ({open,text}),
    async idle() { await work },
    async dispose() {
      disposed = true; clearInterval(timer); hide(); active = undefined
      await work
      await Promise.allSettled([...created].map(bound => bound.client.send(closeUserAppViewRequest(bound.view.view_id))))
      created.clear()
    },
  }
}
