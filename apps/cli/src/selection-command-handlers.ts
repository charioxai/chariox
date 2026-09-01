import type {
  RuntimeAttachment,
  RuntimeSession,
  SessionConfigState,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { MultiAgentResponseLayout, UiPreferences } from "./preferences.js"

type FooterTone = "info" | "error"

export type SelectionCommandHandlerDeps = {
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  attachmentState: () => RuntimeAttachment | null
  focusedAgentId: () => string | null
  multiAgentResponseLayout: () => MultiAgentResponseLayout
  flashFooter: (message: string, tone: FooterTone) => void
  applyModelSelection: (value: string) => Promise<void>
  applyAccountSelection?: (value: string) => Promise<void>
  applyVariantSelection: (value: string) => Promise<void>
  applyModeSelection?: (value: string) => Promise<void>
  applyPermissionSelection?: (value: string) => Promise<void>
  logViewCommand?: (fields: Record<string, unknown>) => void
  setMultiAgentResponseLayout: (layout: MultiAgentResponseLayout) => void
  applyResponseLayout: () => void
  updateSessionResponseLayout: (
    sessionId: string,
    attachmentId: string,
    layout: MultiAgentResponseLayout,
  ) => Promise<{ session: RuntimeSession; config: SessionConfigState }>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  saveUiPreferences: (prefs: UiPreferences) => Promise<void>
  rebuildTranscript: () => void
  requestRender: () => void
  afterViewRender?: (layout: MultiAgentResponseLayout) => void
}

export async function handleModelSlashCommand(
  deps: SelectionCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "model" }>,
): Promise<void> {
  const { value } = command
  if (!value) {
    deps.flashFooter("usage: /model <provider/model>", "error")
    return
  }
  await deps.applyModelSelection(value)
}

export async function handleAccountSlashCommand(
  deps: SelectionCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "account" }>,
): Promise<void> {
  if (!command.value) {
    deps.flashFooter("usage: /account <account>", "error")
    return
  }
  if (!deps.applyAccountSelection) {
    deps.flashFooter("account selection is unavailable in this build", "error")
    return
  }
  await deps.applyAccountSelection(command.value)
}

export async function handleVariantSlashCommand(
  deps: SelectionCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "variant" }>,
): Promise<void> {
  const { value } = command
  if (!value) {
    deps.flashFooter("usage: /variant <name>", "error")
    return
  }
  await deps.applyVariantSelection(value)
}

export async function handleModeSlashCommand(
  deps: SelectionCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "mode" }>,
): Promise<void> {
  if (!command.value) {
    deps.flashFooter("usage: /mode <build|plan>", "error")
    return
  }
  if (!deps.applyModeSelection) {
    deps.flashFooter("mode selection is unavailable in this build", "error")
    return
  }
  await deps.applyModeSelection(command.value)
}

export async function handlePermissionsSlashCommand(
  deps: SelectionCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "permissions" }>,
): Promise<void> {
  if (!command.value) {
    deps.flashFooter("usage: /permissions <required|yolo>", "error")
    return
  }
  if (!deps.applyPermissionSelection) {
    deps.flashFooter("permission selection is unavailable in this build", "error")
    return
  }
  await deps.applyPermissionSelection(command.value)
}

export async function handleViewSlashCommand(
  deps: SelectionCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "view" }>,
): Promise<void> {
  const selection = parseRequestedViewLayout(command.value, deps.multiAgentResponseLayout())
  if (selection.kind === "summary") {
    deps.flashFooter(
      `view: ${deps.multiAgentResponseLayout()} • agents: ${deps.sessionState().agents.length}`,
      "info",
    )
    return
  }
  if (selection.kind === "invalid") {
    deps.flashFooter("usage: /view <split|individual>", "error")
    return
  }

  const nextLayout = selection.layout
  deps.logViewCommand?.({
    requested_layout: nextLayout,
    previous_layout: deps.multiAgentResponseLayout(),
    attached: deps.isAttached(),
    agent_count: deps.sessionState().agents.length,
    focused_agent_id: deps.focusedAgentId(),
  })
  deps.setMultiAgentResponseLayout(nextLayout)
  deps.applyResponseLayout()
  if (deps.isAttached() && deps.attachmentState()) {
    const updated = await deps.updateSessionResponseLayout(
      deps.sessionState().id,
      deps.attachmentState()!.id,
      nextLayout,
    )
    deps.applySessionState(updated.session)
    await deps.refreshAgentPanes(updated.session)
  }
  await deps.saveUiPreferences({ multiAgentResponseLayout: nextLayout })
  deps.rebuildTranscript()
  deps.requestRender()
  deps.afterViewRender?.(nextLayout)
  deps.flashFooter(`view set to ${nextLayout} • ${deps.sessionState().agents.length} agents`, "info")
}

export function parseRequestedViewLayout(
  value: string,
  currentLayout: MultiAgentResponseLayout,
):
  | { kind: "summary" }
  | { kind: "invalid" }
  | { kind: "set"; layout: MultiAgentResponseLayout } {
  const normalized = value.trim().toLowerCase()
  if (!normalized) {
    return { kind: "summary" }
  }
  if (normalized !== "split" && normalized !== "individual") {
    return { kind: "invalid" }
  }
  if (normalized === currentLayout) {
    return { kind: "set", layout: currentLayout }
  }
  return { kind: "set", layout: normalized }
}
