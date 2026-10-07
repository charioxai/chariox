import { getLogger } from "./cli-runtime-singletons.js"
import { createEffect, createSignal, onCleanup, untrack } from "solid-js"
import { roomWorkflowInventoryPayload } from "@chariox/kernel-client/room-workflows"
import { RoomWorkflowsPaneController } from "./room-workflows-pane-controller.js"
import { saveRoomWorkflowPaneDismissed, type CharioxPreferences } from "./preferences.js"
import type { LocalIpcClient } from "./ipc.js"
import type { RuntimeSession } from "./cli-types.js"

type Key = { name: string; ctrl?: boolean; meta?: boolean; shift?: boolean; preventDefault(): void; stopPropagation(): void }
export function createCliRoomWorkflowsComposition(deps: {
 client: LocalIpcClient; clientId: string; preferences: CharioxPreferences
 session(): RuntimeSession; attached(): boolean; connected(): boolean
 manage(id: string): void; focusAgents(): void; blurAgents(): void
 narrow(): boolean; workflowScreenActive(): boolean; showRoom(): void; dialogOverlayOpen(): boolean
}) {
 const [revision, setRevision] = createSignal(0)
 let lastInventory: unknown
 const closed = { ...deps.preferences.roomWorkflowPaneDismissed }
 const controller = new RoomWorkflowsPaneController({ clientId: deps.clientId, send: request => deps.client.send(request), changed: () => setRevision(value => value + 1), dismissed: key => closed[key] ?? false,
   trace: guard => getLogger("cli.room_workflows")?.info("workflow request guard", { guard }),
   saveDismissed: (key, value) => { closed[key] = value; void saveRoomWorkflowPaneDismissed(key, value).catch(() => {}) }, manage: deps.manage })
 createEffect(() => {
   const session = deps.session(); const attached = deps.attached(); const connected = deps.connected()
   untrack(() => {
     if (!attached) { controller.deactivate(); return }
     const inventory = roomWorkflowInventoryPayload(session.room_workflows, session.id)
     if (inventory && (inventory !== lastInventory || (connected && session.room_workflows_fresh === true && controller.record?.fresh === false))) { lastInventory = inventory; controller.apply(inventory, connected && (session.room_workflows_fresh === true || controller.record?.fresh !== false)) }
     else if (controller.record?.inventory.session_id !== session.id) controller.deactivate()
     else if (!connected) controller.stale()
   })
 })
 const visible = () => { revision(); return deps.attached() && controller.visible && (!deps.narrow() || controller.screenActive) && !deps.workflowScreenActive() }
 const ownsInput = () => visible() && controller.focused && !deps.dialogOverlayOpen()
 const open = () => {
   if (deps.dialogOverlayOpen() || !deps.attached() || !controller.available) return
   deps.showRoom(); controller.open(); deps.blurAgents()
 }
 // When a narrow room screen replaces agents, its visible composer must own focus.
 createEffect(() => {
   const transfer = visible() && deps.narrow() && !deps.dialogOverlayOpen()
   untrack(() => {
     if (transfer && !controller.focused) { controller.focus(true); deps.blurAgents() }
   })
 })
 onCleanup(deps.client.onKernelEvent(event => {
   if (!deps.attached()) return
   if (event.event === "room_workflows_changed") {
     const inventory = roomWorkflowInventoryPayload(event.inventory, deps.session().id)
     if (inventory) controller.apply(inventory, controller.record?.fresh !== false && deps.connected())
   } else if (event.event === "transport_closed" || event.event === "replay_gap" || event.event === "transport_resumed") controller.stale()
 }))
 return {
   controller, revision, visible, ownsInput, open,
   handleKey(event: Key): boolean {
     if (deps.dialogOverlayOpen()) return false
     if (event.ctrl && event.name === "w" && deps.attached() && controller.available) {
       event.preventDefault(); event.stopPropagation()
       if (!visible() || !controller.focused) open()
       else { controller.focus(false); deps.focusAgents() }
       return true
     }
     if (!ownsInput()) return false
     let action = true
     if (event.name === "escape") { controller.close(); deps.focusAgents() }
     else if (event.name === "tab") { controller.focus(false); deps.focusAgents() }
     else if ((event.ctrl || event.meta) && event.name === "up") controller.cycle(-1)
     else if ((event.ctrl || event.meta) && event.name === "down") controller.cycle(1)
     else if (event.ctrl && event.name === "p") void controller.control("pause")
     else if (event.ctrl && event.name === "r") void controller.control("resume")
     else if (event.ctrl && event.name === "s") void controller.control("stop")
     else if (event.ctrl && event.name === "g") { controller.manage(); deps.focusAgents() }
     else action = false
     if (action) { event.preventDefault(); event.stopPropagation() }
     return action
   },
 }
}
