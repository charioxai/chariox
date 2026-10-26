import type { CliRenderer } from "@opentui/core"
import { createEffect, onCleanup } from "solid-js"
import { routeRawPastes } from "./raw-paste-routing.js"
import { createAppActionsController } from "./app-actions-controller.js"
import { createUserAppViewRenderer } from "./user-app-view-renderer.js"
import { captureCliDialogFocus, restoreCliDialogFocus, type CliDialogFocusTarget } from "./cli-dialog-focus-controller.js"

export function createCliAppActionsComposition(deps: {
  client(): {send(request: any): Promise<any>}; session(): string | undefined; renderer: CliRenderer
  dimensions(): {width: number; height: number}; themeRevision(): unknown
  currentFocus(): CliDialogFocusTarget | null; promptFocus(): CliDialogFocusTarget | null
  closeOtherDialog(): void; notify(message: string): void
}) {
  const surface = createUserAppViewRenderer(deps.renderer, { id: "app-actions", footer: "R: refresh · PgUp/PgDn: scroll · A / Esc: close agent view" })
  let text = "", visible = false, saved: CliDialogFocusTarget | null = null
  const render = () => surface.render({open:visible,text},deps.dimensions())
  const controller = createAppActionsController({ ...deps, scroll:surface.scroll,
    show(value) {
      if (!visible) { deps.closeOtherDialog(); saved = captureCliDialogFocus(deps.currentFocus(),deps.promptFocus()) }
      text = value; visible = true; render()
    },
    hide() { if (visible) { visible = false; render(); restoreCliDialogFocus(saved); saved = null } },
  })
  createEffect(() => { deps.themeRevision(); deps.session(); controller.current(); render() })
  onCleanup(routeRawPastes(deps.renderer.keyInput, () => controller.ownsInput()))
  onCleanup(() => { controller.hide(); surface.dispose() })
  return controller
}
