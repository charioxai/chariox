import {
  BoxRenderable,
  TextAttributes,
  TextRenderable,
} from "@opentui/core"

import type {
  AgentInstance,
  MetaagentTask,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import type { PromptMetaTone } from "@chariox/kernel-client/prompt-meta"
import {
  formatSplitPaneFooterParts,
  type StatusBadgeTone,
} from "./split-pane-footer.js"
import { sessionAgentPaneStatusBadgeForSession, sessionAgentTaskStatus } from "@chariox/kernel-client/session-runtime-status"
import { providerRunForPromptSelection } from "@chariox/kernel-client/prompt-provider-selection"
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
  outcomeText?: TextRenderable
  continueText?: TextRenderable
  goalText?: TextRenderable
  badgeBox?: BoxRenderable
}

type SplitPaneFooterTaskGroup = {
  taskBox?: BoxRenderable
  statusText?: TextRenderable
  dividerText?: TextRenderable
  summaryText?: TextRenderable
}

type ProviderSelection = {
  provider?: string
  accountProfile?: string
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
  session: RuntimeSession | null
  visibleAgents: Array<AgentInstance | null | undefined>
  metaagentTasks: readonly MetaagentTask[]
  focusedAgentId: string | null
  providerRun: RuntimeProviderRun | null
  currentProviderSelection: ProviderSelection
  agentActivityLabels: Record<string, string | null>
  streamingAgentId: string | null
  agentBusyLatch: (agentId: string) => boolean
  sessionConfigValues: Record<string, string> | undefined
  agentLocationLabel: (agent: AgentInstance | null | undefined) => string | null
  providerAccountLabel: (provider: string, accountProfile: string) => string
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
  state.goalText = new TextRenderable(renderer, { wrapMode: "word", flexShrink: 0 })
  state.goalText.visible = false
  footerBox.add(state.goalText)
  state.outcomeText = new TextRenderable(renderer, { wrapMode: "none", height: 1, truncate: true, flexShrink: 0 })
  state.outcomeText.visible = false
  footerBox.add(state.outcomeText)
  state.continueText = new TextRenderable(renderer, { wrapMode: "none", flexShrink: 0 })
  state.continueText.visible = false
  footerBox.add(state.continueText)
  footerBox.add(footerRow)
  state.parts = nextParts
  state.taskParts = nextTaskParts
  state.badgeTexts = nextBadgeTexts
  state.badgeBox = badgeBox
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
  if (state.goalText) state.goalText.visible = false
  if (state.outcomeText) state.outcomeText.visible = false
  if (state.continueText) state.continueText.visible = false
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
  const badge = sessionAgentPaneStatusBadgeForSession({
    session: options.session,
    agent: agent ?? null,
    activeLabel: agent ? options.agentActivityLabels[agent.id] ?? null : null,
    isStreaming: agent?.id === options.streamingAgentId,
    busyLatch: agent ? options.agentBusyLatch(agent.id) : false,
  })
  const focused = agent?.id === options.focusedAgentId
  const task = agent?.meta_mode
    ? options.metaagentTasks.find((entry) => entry.metaagent_id === agent.id) ?? null
    : null
  renderTaskParts(state.taskParts, task)
  const goals = options.session?.agent_tasks?.filter((task) => task.goal && task.agent_id === agent?.id) ?? []
  const goalStatus = agent && options.session && goals.length
    ? sessionAgentTaskStatus({ ...options.session, agent_tasks: goals }, agent.id) : null
  if (state.goalText) {
    state.goalText.visible = Boolean(goalStatus)
    state.goalText.content = goalStatus?.label ?? ""
    state.goalText.fg = goalStatus?.tone === "error" ? theme.error : theme.textMuted
  }
  if (state.outcomeText) {
    const activity = agent ? options.session?.agent_activity?.[agent.id] : null
    const outcome = activity?.last_completed_turn?.outcome
    const failed = outcome && outcome.kind !== "completed" && outcome.kind !== "cancelled"
    const settled = activity && !activity.busy && activity.prompt_status === "none"
      && !options.agentBusyLatch(agent?.id ?? "")
    state.outcomeText.visible = Boolean(outcome && settled)
    state.outcomeText.content = outcome && settled
      ? `${outcome.kind}${outcome.reason ? ` · ${outcome.reason.replace(/\s+/g, " ").trim()}` : ""}`
      : ""
    state.outcomeText.fg = failed ? theme.error : theme.textMuted
    if (state.continueText) {
      state.continueText.visible = Boolean(settled && failed && outcome.retryable && focused)
      state.continueText.content = state.continueText.visible ? "Continue: /continue" : ""
      state.continueText.fg = theme.error
    }
  }
  // The goal row above carries the detail; the badge keeps only its phase.
  const badgeLabel = badge.label.startsWith("GOAL ") ? badge.label.split(":")[0]! : badge.label
  while (state.badgeBox && state.badgeTexts.length < badgeLabel.length) {
    const text = new TextRenderable(options.renderer, { wrapMode: "none" })
    state.badgeBox.add(text)
    state.badgeTexts.push(text)
  }
  renderStatusBadgeTexts(state.badgeTexts, badgeLabel, badge.tone, options)
  const run = agent ? providerRunForPromptSelection(options.providerRun, agent) : null
  const activeRun = run
    ? {
        agentInstanceId: run.agent_instance_id,
        provider: run.provider,
        accountProfile: run.account_profile,
        model: run.model,
        variant: run.variant,
      }
    : null
  const provider = activeRun?.provider
    ?? selectionOverride?.provider
    ?? agent?.provider
    ?? "opencode"
  const accountProfile = activeRun?.accountProfile
    ?? selectionOverride?.accountProfile
    ?? agent?.account_profile
    ?? "default"
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
    {
      ...(selectionOverride
        ? { provider: selectionOverride.provider ?? null, model: selectionOverride.model, variant: selectionOverride.effort }
        : {}),
      accountProfile,
      accountLabel: options.providerAccountLabel(provider, accountProfile),
    },
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
