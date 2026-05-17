import {
  BoxRenderable,
  TextAttributes,
  TextRenderable,
} from "@opentui/core"

import type {
  AgentInstance,
  RuntimeProviderRun,
} from "./cli-types.js"
import type { PromptMetaTone } from "./prompt-meta.js"
import {
  agentPaneStatusBadge,
  formatSplitPaneFooterParts,
  type StatusBadgeTone,
} from "./split-pane-footer.js"
import { renderStatusBadgeLabel } from "./status-badge-renderer.js"
import { theme } from "./theme.js"

export type SplitPaneFooterTextGroup = {
  agentText?: TextRenderable
  agentDividerText?: TextRenderable
  providerText?: TextRenderable
  providerDividerText?: TextRenderable
  modelText?: TextRenderable
  modelDividerText?: TextRenderable
  variantText?: TextRenderable
  variantDividerText?: TextRenderable
  modeText?: TextRenderable
  modeDividerText?: TextRenderable
  permissionText?: TextRenderable
}

export type SplitPaneFooterRenderState = {
  primary: SplitPaneFooterSlotState
  auxiliaries: SplitPaneFooterSlotState[]
}

type SplitPaneFooterSlotState = {
  parts: SplitPaneFooterTextGroup
  badgeTexts: TextRenderable[]
}

type ProviderSelection = {
  model: string
  effort: string
}

export type SplitPaneFooterRenderOptions = {
  renderer: ConstructorParameters<typeof BoxRenderable>[0]
  state: SplitPaneFooterRenderState
  primaryBox: BoxRenderable | undefined
  auxiliaryBoxes: Array<BoxRenderable | undefined>
  showAgentFooters: boolean
  maxAgentsPerScreen: number
  visibleAgents: Array<AgentInstance | null | undefined>
  focusedAgentId: string | null
  providerRun: RuntimeProviderRun | null
  currentProviderSelection: ProviderSelection
  agentActivityLabels: Record<string, string | null>
  hasPromptWorkByAgent: Record<string, boolean>
  streamingAgentId: string | null
  agentBusyLatch: (agentId: string) => boolean
  sessionConfigValues: Record<string, string> | undefined
  agentLocationLabel: (agent: AgentInstance | null | undefined) => string | null
  badgeWidth: number
  animationFrame: number
}

export function createSplitPaneFooterRenderState(): SplitPaneFooterRenderState {
  return {
    primary: { parts: {}, badgeTexts: [] },
    auxiliaries: [],
  }
}

export function renderSplitPaneFooters(options: SplitPaneFooterRenderOptions): void {
  ensureFooterRenderables(
    options.renderer,
    options.primaryBox,
    options.state.primary,
    options.badgeWidth,
  )
  for (let slotIndex = 0; slotIndex < options.maxAgentsPerScreen - 1; slotIndex += 1) {
    let slotState = options.state.auxiliaries[slotIndex]
    if (!slotState) {
      slotState = { parts: {}, badgeTexts: [] }
      options.state.auxiliaries[slotIndex] = slotState
    }
    ensureFooterRenderables(
      options.renderer,
      options.auxiliaryBoxes[slotIndex],
      slotState,
      options.badgeWidth,
    )
  }

  if (!options.showAgentFooters) {
    clearFooter(options.state.primary, options.primaryBox, options)
    for (let slotIndex = 0; slotIndex < options.maxAgentsPerScreen - 1; slotIndex += 1) {
      clearFooter(options.state.auxiliaries[slotIndex], options.auxiliaryBoxes[slotIndex], options)
    }
    return
  }

  renderFooter(
    options,
    options.visibleAgents[0] ?? null,
    options.primaryBox,
    options.state.primary,
  )
  for (let slotIndex = 0; slotIndex < options.maxAgentsPerScreen - 1; slotIndex += 1) {
    renderFooter(
      options,
      options.visibleAgents[slotIndex + 1] ?? null,
      options.auxiliaryBoxes[slotIndex],
      options.state.auxiliaries[slotIndex],
    )
  }
  options.primaryBox?.requestRender()
}

function ensureFooterRenderables(
  renderer: ConstructorParameters<typeof BoxRenderable>[0],
  footerBox: BoxRenderable | undefined,
  state: SplitPaneFooterSlotState,
  badgeWidth: number,
): void {
  if (!footerBox || state.parts.agentText) {
    return
  }
  footerBox.flexDirection = "row"
  footerBox.gap = 1
  const badgeBox = new BoxRenderable(renderer, {
    flexDirection: "row",
    flexShrink: 0,
  })
  const infoBox = new BoxRenderable(renderer, {
    flexDirection: "row",
    flexShrink: 0,
  })
  const nextBadgeTexts = Array.from({ length: badgeWidth }, () => new TextRenderable(renderer, { wrapMode: "none" }))
  for (const text of nextBadgeTexts) {
    badgeBox.add(text)
  }
  const nextParts: SplitPaneFooterTextGroup = {
    agentText: new TextRenderable(renderer, { wrapMode: "none" }),
    agentDividerText: new TextRenderable(renderer, { wrapMode: "none" }),
    providerText: new TextRenderable(renderer, { wrapMode: "none" }),
    providerDividerText: new TextRenderable(renderer, { wrapMode: "none" }),
    modelText: new TextRenderable(renderer, { wrapMode: "none" }),
    modelDividerText: new TextRenderable(renderer, { wrapMode: "none" }),
    variantText: new TextRenderable(renderer, { wrapMode: "none" }),
    variantDividerText: new TextRenderable(renderer, { wrapMode: "none" }),
    modeText: new TextRenderable(renderer, { wrapMode: "none" }),
    modeDividerText: new TextRenderable(renderer, { wrapMode: "none" }),
    permissionText: new TextRenderable(renderer, { wrapMode: "none" }),
  }
  infoBox.add(nextParts.agentText!)
  infoBox.add(nextParts.agentDividerText!)
  infoBox.add(nextParts.providerText!)
  infoBox.add(nextParts.providerDividerText!)
  infoBox.add(nextParts.modelText!)
  infoBox.add(nextParts.modelDividerText!)
  infoBox.add(nextParts.variantText!)
  infoBox.add(nextParts.variantDividerText!)
  infoBox.add(nextParts.modeText!)
  infoBox.add(nextParts.modeDividerText!)
  infoBox.add(nextParts.permissionText!)
  footerBox.add(badgeBox)
  footerBox.add(infoBox)
  state.parts = nextParts
  state.badgeTexts = nextBadgeTexts
}

function clearFooter(
  state: SplitPaneFooterSlotState | undefined,
  footerBox: BoxRenderable | undefined,
  options: SplitPaneFooterRenderOptions,
): void {
  if (!state) {
    return
  }
  renderStatusBadgeTexts(state.badgeTexts, "", "idle", options)
  clearFooterParts(state.parts)
  footerBox?.requestRender()
}

function clearFooterParts(parts: SplitPaneFooterTextGroup): void {
  setTextRenderable(parts.agentText, "", theme.textMuted)
  setTextRenderable(parts.agentDividerText, "", theme.textMuted)
  setTextRenderable(parts.providerText, "", theme.textMuted)
  setTextRenderable(parts.providerDividerText, "", theme.textMuted)
  setTextRenderable(parts.modelText, "", theme.textMuted)
  setTextRenderable(parts.modelDividerText, "", theme.textMuted)
  setTextRenderable(parts.variantText, "", theme.textMuted)
  setTextRenderable(parts.variantDividerText, "", theme.textMuted)
  setTextRenderable(parts.modeText, "", theme.textMuted)
  setTextRenderable(parts.modeDividerText, "", theme.textMuted)
  setTextRenderable(parts.permissionText, "", theme.textMuted)
}

function renderFooter(
  options: SplitPaneFooterRenderOptions,
  agent: AgentInstance | null | undefined,
  footerBox: BoxRenderable | undefined,
  state: SplitPaneFooterSlotState | undefined,
): void {
  if (!state) {
    return
  }
  const selectionOverride = agent?.id === options.focusedAgentId
    ? options.currentProviderSelection
    : null
  const badge = agentPaneStatusBadge(
    agent ?? null,
    agent ? options.agentActivityLabels[agent.id] ?? null : null,
    agent ? options.hasPromptWorkByAgent[agent.id] ?? false : false,
    agent?.id === options.streamingAgentId,
    agent ? options.agentBusyLatch(agent.id) : false,
  )
  const focused = agent?.id === options.focusedAgentId
  renderStatusBadgeTexts(state.badgeTexts, badge.label, badge.tone, options)
  const activeRun = options.providerRun && options.providerRun.agent_instance_id === agent?.id
    ? {
        agentInstanceId: options.providerRun.agent_instance_id,
        model: options.providerRun.model,
        variant: options.providerRun.variant,
      }
    : null
  const nextParts = formatSplitPaneFooterParts(
    agent
      ? {
          ...agent,
          execution_mode: agent.execution_mode_override
            ?? ((options.sessionConfigValues?.["agents.mode"] as "build" | "plan" | undefined) ?? "build"),
          permission_level: agent.permission_level_override
            ?? ((options.sessionConfigValues?.["agents.permissions"] as "required" | "yolo" | undefined) ?? "yolo"),
          location_label: options.agentLocationLabel(agent),
        }
      : null,
    activeRun,
    null,
    selectionOverride
      ? { model: selectionOverride.model, variant: selectionOverride.effort }
      : undefined,
  )
  const partTones = nextParts.map((part) => part.tone)
  const partTexts = nextParts.map((part) => part.text)
  const setPart = (
    text: TextRenderable | undefined,
    content: string,
    tone: PromptMetaTone | undefined,
    bold = false,
  ) => {
    setTextRenderable(
      text,
      content,
      tone ? theme[tone] : theme.textMuted,
      bold ? TextAttributes.BOLD : TextAttributes.NONE,
    )
  }
  setPart(state.parts.agentText, partTexts[0] ?? "", partTones[0], focused)
  setTextRenderable(state.parts.agentDividerText, partTexts[1] ? " • " : "", theme.textMuted)
  setPart(state.parts.providerText, partTexts[1] ?? "", partTones[1], focused)
  setTextRenderable(state.parts.providerDividerText, partTexts[2] ? " • " : "", theme.textMuted)
  setPart(state.parts.modelText, partTexts[2] ?? "", partTones[2], focused)
  setTextRenderable(state.parts.modelDividerText, partTexts[3] ? " • " : "", theme.textMuted)
  setPart(state.parts.variantText, partTexts[3] ?? "", partTones[3], focused)
  setTextRenderable(state.parts.variantDividerText, partTexts[4] ? " • " : "", theme.textMuted)
  setPart(state.parts.modeText, partTexts[4] ?? "", partTones[4], focused)
  setTextRenderable(state.parts.modeDividerText, partTexts[5] ? " • " : "", theme.textMuted)
  setPart(state.parts.permissionText, partTexts[5] ?? "", partTones[5], focused)
  footerBox?.requestRender()
}

function renderStatusBadgeTexts(
  texts: TextRenderable[],
  label: string,
  tone: StatusBadgeTone,
  options: SplitPaneFooterRenderOptions,
): void {
  renderStatusBadgeLabel(texts, label, tone, options.badgeWidth, options.animationFrame)
}

function setTextRenderable(
  text: TextRenderable | undefined,
  content: string,
  fg: (typeof theme)[keyof typeof theme],
  attributes = TextAttributes.NONE,
): void {
  if (!text) {
    return
  }
  text.content = content
  text.fg = fg
  text.attributes = attributes
}
