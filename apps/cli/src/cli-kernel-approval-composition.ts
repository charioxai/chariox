import type { BoxRenderable, CliRenderer } from "@opentui/core"
import { createEffect, onCleanup } from "solid-js"
import type { RuntimeSession } from "./cli-types.js"
import { captureCliDialogFocus, restoreCliDialogFocus, type CliDialogFocusTarget } from "./cli-dialog-focus-controller.js"
import { createKernelApprovalController } from "./kernel-approval-controller.js"
import { createKernelApprovalRenderer } from "./kernel-approval-renderer.js"
import type { LocalIpcClient } from "./ipc.js"
import { respondToInteraction } from "./prompt-runtime-api.js"
import { routeRawPastes } from "./raw-paste-routing.js"

export function createCliKernelApprovalComposition(deps: {
  client: LocalIpcClient
  renderer: CliRenderer
  session(): RuntimeSession
  connected(): boolean
  attached(): boolean
  flashFooter(message: string, tone: "info"): void
  dimensions(): { width: number; height: number }
  themeRevision(): unknown
  currentFocus(): CliDialogFocusTarget | null
  promptFocus(): CliDialogFocusTarget | null
  closeOtherDialog(): void
  applySession(session: RuntimeSession): void
}) {
  let savedFocus: CliDialogFocusTarget | null = null
  const surface = createKernelApprovalRenderer(deps.renderer, {
    show: () => controller.show(),
    choose: (interactionId, choiceId) => { void controller.choose(interactionId, choiceId) },
    cycleRemember: () => controller.cycleRemember(),
    submitPasskey: () => { void controller.submitPasskey() },
  })
  const controller = createKernelApprovalController({
    getSession: deps.session,
    connected: deps.connected,
    onView: (view) => surface.render(view, deps.dimensions()),
    onOpen: () => {
      deps.closeOtherDialog()
      savedFocus = captureCliDialogFocus(deps.currentFocus(), deps.promptFocus())
    },
    onClose: () => { restoreCliDialogFocus(savedFocus); savedFocus = null },
    scroll: surface.scroll,
    respond: (sessionId, interactionId, choiceId, proof) =>
      respondToInteraction(deps.client, sessionId, interactionId, choiceId, null, proof),
    applySession: deps.applySession,
  })
  createEffect(() => { deps.themeRevision(); deps.dimensions(); controller.sync() })
  // A paste while the panel is open belongs to the passkey, never the prompt.
  onCleanup(routeRawPastes(deps.renderer.keyInput, controller.handlePaste))
  onCleanup(() => controller.dispose())
  return {
    ...controller,
    openFromCommand() {
      if (!deps.attached()) deps.flashFooter("start or join a session to view approvals", "info")
      else if (controller.view().count) controller.show()
      else deps.flashFooter("No pending approvals", "info")
    },
    assignBanner(value: BoxRenderable) { surface.assignBanner(value); controller.sync() },
    assignBox(value: BoxRenderable) { surface.assign(value); controller.sync() },
  }
}
