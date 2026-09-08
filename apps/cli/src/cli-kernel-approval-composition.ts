import type { BoxRenderable, CliRenderer } from "@opentui/core"
import { createEffect, onCleanup } from "solid-js"
import type { RuntimeSession } from "./cli-types.js"
import { captureCliDialogFocus, restoreCliDialogFocus, type CliDialogFocusTarget } from "./cli-dialog-focus-controller.js"
import { createKernelApprovalController } from "./kernel-approval-controller.js"
import { createKernelApprovalRenderer } from "./kernel-approval-renderer.js"
import type { LocalIpcClient } from "./ipc.js"
import { respondToInteraction } from "./prompt-runtime-api.js"

export function createCliKernelApprovalComposition(deps: {
  client: LocalIpcClient
  renderer: CliRenderer
  session(): RuntimeSession
  connected(): boolean
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
    respond: (sessionId, interactionId, choiceId) =>
      respondToInteraction(deps.client, sessionId, interactionId, choiceId, null),
    applySession: deps.applySession,
  })
  createEffect(() => { deps.themeRevision(); deps.dimensions(); controller.sync() })
  onCleanup(() => controller.dispose())
  return {
    ...controller,
    assignBox(value: BoxRenderable) { surface.assign(value); controller.sync() },
  }
}
