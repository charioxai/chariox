import { userAppViewsPrototypeEnabled } from "./user-app-views-flag.js"
import { answerUserDomainInteraction } from "./user-domain-interaction-api.js"
import type { BoxRenderable, CliRenderer } from "@opentui/core"
import { createEffect, onCleanup } from "solid-js"
import type { RuntimeSession } from "./cli-types.js"
import { captureCliDialogFocus, restoreCliDialogFocus, type CliDialogFocusTarget } from "./cli-dialog-focus-controller.js"
import { createKernelApprovalController, type KernelApprovalKey } from "./kernel-approval-controller.js"
import { createKernelApprovalRenderer } from "./kernel-approval-renderer.js"
import type { LocalIpcClient } from "./ipc.js"
import { createPasskeyPopupController, passkeyPromptsFromEvent } from "./passkey-popup-controller.js"
import { createPasskeyPopupRenderer } from "./passkey-popup-renderer.js"
import { extendKernelSudo, revokeKernelAccessGrant } from "./kernel-api.js"
import { respondToInteraction, respondToKernelAccessDecision } from "./prompt-runtime-api.js"
import { createSudoWindowBand } from "./sudo-window-band.js"
import { routeRawPastes, type RawPasteEvent } from "./raw-paste-routing.js"

/** The kernel's decisions on this terminal: the session's approval panel and,
 * on top of it, the passkey popup (protocol 403), which shows the owner's
 * passkey prompts whether or not a session is attached. */
export function createCliKernelApprovalComposition(deps: {
  client: LocalIpcClient
  renderer: CliRenderer
  session(): RuntimeSession
  /** Attached to a session and connected: the panel can answer. */
  connected(): boolean
  attached(): boolean
  flashFooter(message: string, tone: "info"): void
  /** Connected to the kernel: the popup can answer. */
  kernelConnected(): boolean
  dimensions(): { width: number; height: number }
  themeRevision(): unknown
  currentFocus(): CliDialogFocusTarget | null
  promptFocus(): CliDialogFocusTarget | null
  closeOtherDialog(): void
  applySession(session: RuntimeSession): void
  notify(message: string): void
  attachmentId(): string | null
}) {
  const actualClient = () => (deps.client as LocalIpcClient & {currentClient?(): LocalIpcClient}).currentClient?.() ?? deps.client
  const userPromptClients = new Map<string, LocalIpcClient>()
  const respondUserPrompt = (id: string, choice: string, proof?: import("./ipc-requests.js").InteractionPasskeyProof) => {
    const source = userPromptClients.get(id)
    if (!source || source !== actualClient()) throw new Error("The App approval belongs to the previous kernel")
    return answerUserDomainInteraction(source, id, choice, proof)
  }
  let savedFocus: CliDialogFocusTarget | null = null
  let dialogs = 0
  const opened = () => {
    if (dialogs++ === 0) {
      deps.closeOtherDialog()
      savedFocus = captureCliDialogFocus(deps.currentFocus(), deps.promptFocus())
    }
  }
  const closed = () => {
    if (--dialogs === 0) { restoreCliDialogFocus(savedFocus); savedFocus = null }
  }
  const popupSurface = createPasskeyPopupRenderer(deps.renderer, {
    show: () => { popup.show() },
    approve: () => { void popup.approve() },
    refuse: () => { void popup.refuse() },
    cycleRemember: () => popup.cycleRemember(),
  })
  const popup = createPasskeyPopupController({
    connected: deps.kernelConnected,
    onView: (view) => popupSurface.render(view, deps.dimensions()),
    onOpen: opened,
    onClose: closed,
    scroll: popupSurface.scroll,
    respond: (prompt, choiceId, proof) =>
      (prompt.session_id === "kernel-access" && (prompt.kind === "access_grant" || prompt.kind === "access_extension")
        ? respondToKernelAccessDecision(deps.client, prompt.interaction_id, choiceId, proof)
        : (prompt.session_id === "" && userAppViewsPrototypeEnabled()
        ? respondUserPrompt(prompt.interaction_id, choiceId, proof)
        : respondToInteraction(deps.client, prompt.session_id, prompt.interaction_id, choiceId, null, proof))),
    notify: deps.notify,
  })
  const surface = createKernelApprovalRenderer(deps.renderer, {
    show: () => controller.show(),
    choose: (interactionId, choiceId) => { void controller.choose(interactionId, choiceId) },
  })
  const controller = createKernelApprovalController({
    getSession: deps.session,
    connected: deps.connected,
    onView: (view) => surface.render(view, deps.dimensions()),
    onOpen: opened,
    onClose: closed,
    scroll: surface.scroll,
    respond: (sessionId, interactionId, choiceId) =>
      respondToInteraction(deps.client, sessionId, interactionId, choiceId, null),
    applySession: deps.applySession,
    showPasskeyPrompt: (sessionId, interactionId) => popup.show(sessionId, interactionId),
  })
  // MP-08/MP-10/MP-11 A04: every attached terminal shows the kernel's sudo
  // windows with Extend (fresh passkey popup) and Revoke.
  const report = (failure: unknown) => deps.notify(failure instanceof Error ? failure.message : String(failure))
  const band = createSudoWindowBand(deps.renderer, {
    extend: (window) => {
      const attachment = deps.attachmentId()
      if (!attachment) return
      deps.notify("Extend sudo: enter your passkey in the popup (F8) and choose 1-8 hours")
      void extendKernelSudo(deps.client, { session_id: window.session_id, attachment_id: attachment, entry_id: window.entry_id, revision: window.revision ?? 0 })
        .then((turn) => deps.notify(`Sudo window ${turn.entry_id} extended`), report)
    },
    revoke: (window) => {
      void revokeKernelAccessGrant(deps.client, window.entry_id)
        .then(() => deps.notify(`Revoked sudo window ${window.entry_id}`), report)
    },
  })
  const renderBand = () => {
    const session = deps.session()
    // MP-08/MP-10/MP-11: resolved kernel status is visible during startup too.
    band.render(session.sudo_windows ?? [], Date.now(),
      (agentId) => session.agents.find((agent) => agent.id === agentId)?.alias ?? agentId)
  }
  const bandTick = setInterval(renderBand, 15_000)
  onCleanup(() => clearInterval(bandTick))
  createEffect(() => {
    deps.themeRevision()
    renderBand()
  })
  createEffect(() => {
    deps.themeRevision()
    deps.dimensions()
    controller.sync()
    popupSurface.render(popup.view(), deps.dimensions())
  })
  onCleanup(deps.client.onKernelEvent((event) => {
    if (event.event === "passkey_prompts_changed") {
      const prompts = passkeyPromptsFromEvent(event.prompts, userAppViewsPrototypeEnabled())
      userPromptClients.clear()
      for (const prompt of prompts) if (prompt.session_id === "") userPromptClients.set(prompt.interaction_id, actualClient())
      popup.apply(prompts)
    }
  }))
  // A paste while the popup or the panel is open never reaches the prompt;
  // in the popup it belongs to the passkey.
  onCleanup(routeRawPastes(deps.renderer.keyInput,
    (event: RawPasteEvent) => popup.handlePaste(event) || controller.handlePaste(event)))
  onCleanup(() => { popup.dispose(); controller.dispose() })
  return {
    ...controller,
    openFromCommand() {
      if (!deps.attached()) deps.flashFooter("start or join a session to view approvals", "info")
      else if (controller.view().count) controller.show()
      else deps.flashFooter("No pending approvals", "info")
    },
    assignBanner(value: BoxRenderable) { surface.assignBanner(value); controller.sync() },
    assignSudoBand(value: BoxRenderable) { band.assign(value); renderBand() },
    /** The popup takes keys first: it sits over the panel. */
    handleKey: (event: KernelApprovalKey) => popup.handleKey(event) || controller.handleKey(event),
    ownsInput: () => popup.ownsInput() || controller.ownsInput(),
    assignBox(value: BoxRenderable) { surface.assign(value); controller.sync() },
    assignPopupBox(value: BoxRenderable) { popupSurface.assign(value); popupSurface.render(popup.view(), deps.dimensions()) },
  }
}
