import type { CliRenderer } from "@opentui/core"
import { createEffect, onCleanup } from "solid-js"
import { captureCliDialogFocus, restoreCliDialogFocus, type CliDialogFocusTarget } from "./cli-dialog-focus-controller.js"
import { createUserAppViewController, type UserAppViewClient } from "./user-app-view-controller.js"
import { createUserAppViewRenderer } from "./user-app-view-renderer.js"
import { userAppViewsPrototypeEnabled } from "./user-app-views-flag.js"
import { routeRawPastes } from "./raw-paste-routing.js"

export function createCliUserAppViewsComposition(deps: {
  client(): UserAppViewClient; renderer: CliRenderer
  dimensions(): {width: number; height: number}; themeRevision(): unknown
  currentFocus(): CliDialogFocusTarget | null; promptFocus(): CliDialogFocusTarget | null
  notify(message: string): void; approvalOwnsInput(): boolean
}) {
  if (!userAppViewsPrototypeEnabled()) return undefined
  const surface = createUserAppViewRenderer(deps.renderer)
  let saved: CliDialogFocusTarget | null = null
  const controller = createUserAppViewController({ ...deps,
    onOpen() { saved = captureCliDialogFocus(deps.currentFocus(), deps.promptFocus()) },
    onClose() { restoreCliDialogFocus(saved); saved = null },
    onView(view) { surface.render(view, deps.dimensions()) }, scroll: surface.scroll,
  })
  createEffect(() => { deps.themeRevision(); surface.render(controller.view(), deps.dimensions()) })
  onCleanup(routeRawPastes(deps.renderer.keyInput, event => !deps.approvalOwnsInput() && controller.handlePaste(event)))
  onCleanup(() => { void controller.dispose(); surface.dispose() })
  return controller
}
