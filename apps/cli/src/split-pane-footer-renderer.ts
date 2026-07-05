import {
  BoxRenderable,
  TextAttributes,
  TextRenderable,
} from "@opentui/core"

import type {
  AgentInstance,
  MetaagentTask,
  RuntimeProviderRun,
} from "./cli-types.js"
import type { PromptMetaTone } from "@arroba/kernel-client/prompt-meta"
import {
  formatSplitPaneFooterParts,
  type StatusBadgeTone,
} from "./split-pane-footer.js"
import { sessionAgentPaneStatusBadge } from "@arroba/kernel-client/session-runtime-status"
import { renderStatusBadgeLabel } from "./status-badge-renderer.js"
import { theme } from "./theme.js"

export type SplitPaneFooterTextGroup = {
  texts: TextRenderable[]
  dividers: TextRenderable[]
}

export type SplitPaneFooterRenderState = {
  primary: SplitPaneFooterSlotState
  auxiliaries: SplitPaneFooterSlotState[]
}

type SplitPaneFooterSlotState = {
  parts: Partial<SplitPaneFooterTextGroup>
  taskParts: SplitPaneFooterTaskGroup
  badgeTexts: TextRenderable[]
}

type SplitPaneFooterTaskGroup = {
  taskBox?: BoxRenderable
  statusText?: TextRenderable
  dividerText?: TextRenderable
  summaryText?: TextRenderable
}

type ProviderSelection = {
  provider?: string
  model: string
  effort: string
}

const MAX_FOOTER_PARTS = 10

export type SplitPaneFooterRenderOptions = {
  renderer: ConstructorParameters<typeof BoxRenderable>[0]
  state: SplitPaneFooterRenderState
  primaryBox: BoxRenderable | undefined
  auxiliaryBoxes: Array<BoxRenderable | undefined>
  showAgentFooters: boolean
  maxAgentsPerScreen: number
  visibleAgents: Array<AgentInstance | null | undefined>
  metaagentTasks: readonly MetaagentTask[]
  focusedAgentId: string | null
  providerRun: RuntimeProviderRun | null
  currentProviderSelection: ProviderSelection
  agentActivityLabels: Record<string, string | null>
  hasPromptWorkByAgent: Record<string, boolean>
  streamingAgentId: string | null
  agentBusyLatch: (agentId: string) => boolean
  hasProjectedRuntimeState: boolean
  sessionConfigValues: Record<string, string> | undefined
  agentLocationLabel: (agent: AgentInstance | null | undefined) => string | null
  badgeWidth: number
  animationFrame: number
}

export function createSplitPaneFooterRenderState(): SplitPaneFooterRenderState {
  return {
    primary: { parts: {}, taskParts: {}, badgeTexts: [] },
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
      slotState = { parts: {}, taskParts: {}, badgeTexts: [] }
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
  if (!footerBox || hasFooterTextGroup(state.parts)) {
    return
  }
  footerBox.flexDirection = "column"
  footerBox.gap = 1
  const taskBox = new BoxRenderable(renderer, {
    flexDirection: "row",
    flexShrink: 0,
  })
  const footerRow = new BoxRenderable(renderer, {
    flexDirection: "row",
    flexShrink: 0,
    gap: 1,
  })
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
    texts: [],
    dividers: [],
  }
  for (let index = 0; index < MAX_FOOTER_PARTS; index += 1) {
    const text = new TextRenderable(renderer, { wrapMode: "none" })
    const divider = new TextRenderable(renderer, { wrapMode: "none" })
    nextParts.texts.push(text)
    nextParts.dividers.push(divider)
    infoBox.add(text)
    infoBox.add(divider)
  }
  const nextTaskParts: SplitPaneFooterTaskGroup = {
    taskBox,
    statusText: new TextRenderable(renderer, { wrapMode: "none" }),
    dividerText: new TextRenderable(renderer, { wrapMode: "none" }),
    summaryText: new TextRenderable(renderer, { wrapMode: "none" }),
  }
  taskBox.add(nextTaskParts.statusText!)
  taskBox.add(nextTaskParts.dividerText!)
  taskBox.add(nextTaskParts.summaryText!)
  footerRow.add(badgeBox)
  footerRow.add(infoBox)
  footerBox.add(taskBox)
  footerBox.add(footerRow)
  state.parts = nextParts
  state.taskParts = nextTaskParts
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
  renderTaskParts(state.taskParts, null)
  clearFooterParts(state.parts)
  footerBox?.requestRender()
}

function clearFooterParts(parts: Partial<SplitPaneFooterTextGroup>): void {
  if (!hasFooterTextGroup(parts)) {
    return
  }
  for (const text of parts.texts) {
    setTextRenderable(text, "", theme.textMuted)
  }
  for (const divider of parts.dividers) {
    setTextRenderable(divider, "", theme.textMuted)
  }
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
  if (!footerBox || !hasFooterTextGroup(state.parts)) {
    return
  }
  const selectionOverride = agent?.id === options.focusedAgentId
    ? options.currentProviderSelection
    : null
  const badge = sessionAgentPaneStatusBadge({
    agent: agent ?? null,
    activeLabel: agent ? options.agentActivityLabels[agent.id] ?? null : null,
    hasPromptWork: agent ? options.hasPromptWorkByAgent[agent.id] ?? false : false,
    isStreaming: agent?.id === options.streamingAgentId,
    busyLatch: agent ? options.agentBusyLatch(agent.id) : false,
    useLegacyAgentProcessingState: !options.hasProjectedRuntimeState,
  })
  const focused = agent?.id === options.focusedAgentId
  const task = agent?.meta_mode
    ? options.metaagentTasks.find((entry) => entry.metaagent_id === agent.id) ?? null
    : null
  renderTaskParts(state.taskParts, task)
  renderStatusBadgeTexts(state.badgeTexts, badge.label, badge.tone, options)
  const activeRun = options.providerRun && options.providerRun.agent_instance_id === agent?.id
    ? {
        agentInstanceId: options.providerRun.agent_instance_id,
        provider: options.providerRun.provider,
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
      ? { provider: selectionOverride.provider, model: selectionOverride.model, variant: selectionOverride.effort }
      : undefined,
  )
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
  for (let index = 0; index < MAX_FOOTER_PARTS; index += 1) {
    const part = nextParts[index]
    setPart(state.parts.texts[index], part?.text ?? "", part?.tone, focused && index === 0)
    setTextRenderable(state.parts.dividers[index], nextParts[index + 1] ? " • " : "", theme.textMuted)
  }
  footerBox?.requestRender()
}

function hasFooterTextGroup(parts: Partial<SplitPaneFooterTextGroup>): parts is SplitPaneFooterTextGroup {
  return Array.isArray(parts.texts) && Array.isArray(parts.dividers)
}

function renderTaskParts(
  parts: SplitPaneFooterTaskGroup,
  task: MetaagentTask | null,
): void {
  if (!parts.taskBox) {
    return
  }
  parts.taskBox.visible = Boolean(task)
  if (!task) {
    setTextRenderable(parts.statusText, "", theme.textMuted)
    setTextRenderable(parts.dividerText, "", theme.textMuted)
    setTextRenderable(parts.summaryText, "", theme.textMuted)
    return
  }
  setTextRenderable(parts.statusText, taskStatusLabel(task.status), taskStatusColor(task.status), TextAttributes.BOLD)
  setTextRenderable(parts.dividerText, " • ", theme.textMuted)
  setTextRenderable(parts.summaryText, taskSummary(task), theme.textMuted)
}

function taskStatusLabel(status: MetaagentTask["status"]): string {
  switch (status) {
    case "active":
      return "TASK"
    case "paused":
      return "PAUSED"
    case "blocked":
      return "BLOCKED"
    case "completed":
      return "DONE"
    case "aborted":
      return "ABORTED"
  }
}

function taskStatusColor(status: MetaagentTask["status"]): (typeof theme)[keyof typeof theme] {
  switch (status) {
    case "active":
      return theme.info
    case "paused":
      return theme.textMuted
    case "blocked":
      return theme.warning
    case "completed":
      return theme.success
    case "aborted":
      return theme.error
  }
}

function taskSummary(task: MetaagentTask): string {
  const firstLine = task.task_markdown
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find(Boolean)
  return truncateText(firstLine ?? "(empty task)", 96)
}

function truncateText(value: string, maxLength: number): string {
  if (value.length <= maxLength) {
    return value
  }
  return `${value.slice(0, Math.max(0, maxLength - 3))}...`
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
