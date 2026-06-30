import {
  BoxRenderable,
  MouseButton,
  TextAttributes,
  TextRenderable,
} from "@opentui/core"
import { setTimeout as startTimeout } from "node:timers"

import type {
  AgentInstance,
  RuntimeInteraction,
} from "./cli-types.js"
import { renderInteractionCustomChoiceValue } from "./interaction-custom-choice-render.js"
import {
  queuedPromptActionLabel,
  queuedPromptMetaLabel,
  queuedPromptTitleLabel,
} from "./queued-prompt-strip-labels.js"
import type { QueuedPromptStripItem } from "./queued-prompt-strip-state.js"
import { theme } from "./theme.js"

type InteractionStripRenderOptions = {
  renderer: ConstructorParameters<typeof BoxRenderable>[0]
  primaryBox: BoxRenderable | undefined
  auxiliaryBoxes: Array<BoxRenderable | undefined>
  visibleAgents: Array<AgentInstance | null | undefined>
  maxAgentsPerScreen: number
  focusedAgentId: string | null
  activeInteractionForAgent: (agentId: string | null | undefined) => RuntimeInteraction | null
  selectedChoiceIndex: (interactionId: string) => number
  setSelectedChoiceIndex: (interactionId: string, index: number) => void
  customReply: (interactionId: string) => string
  customEditing: (interactionId: string) => boolean
  queuedPromptStripItemsForAgent: (agentId: string | null | undefined) => readonly QueuedPromptStripItem[]
  selectedQueuedPromptIndexForAgent: (agentId: string | null | undefined) => number
  onQueuedPromptAction: (item: QueuedPromptStripItem, action: "steer" | "cancel") => void
}

export function renderAgentInteractionStrips(options: InteractionStripRenderOptions): void {
  renderInteractionStrip(options, options.primaryBox, options.visibleAgents[0] ?? null)
  for (let slotIndex = 0; slotIndex < options.maxAgentsPerScreen - 1; slotIndex += 1) {
    renderInteractionStrip(
      options,
      options.auxiliaryBoxes[slotIndex],
      options.visibleAgents[slotIndex + 1] ?? null,
    )
  }
}

function renderInteractionStrip(
  options: InteractionStripRenderOptions,
  box: BoxRenderable | undefined,
  agent: AgentInstance | null | undefined,
): void {
  if (!box) {
    return
  }
  for (const child of [...box.getChildren()]) {
    box.remove(String(child.id))
    child.destroyRecursively?.()
  }
  const interaction = options.activeInteractionForAgent(agent?.id ?? null)
  const queuedPrompts = options.queuedPromptStripItemsForAgent(agent?.id ?? null)
  const visible = Boolean(interaction) || queuedPrompts.length > 0
  box.visible = visible
  box.flexDirection = "column"
  box.gap = 0
  box.paddingLeft = visible ? 1 : 0
  box.paddingRight = visible ? 1 : 0
  box.paddingTop = interaction ? 0 : 0
  box.paddingBottom = interaction ? 0 : 0
  box.backgroundColor = theme.backgroundElement
  if (!visible) {
    box.requestRender?.()
    return
  }

  const focused = agent?.id === options.focusedAgentId
  const selectedQueuedPromptIndex = focused
    ? options.selectedQueuedPromptIndexForAgent(agent?.id ?? null)
    : -1
  if (interaction) {
    const titleLine = new TextRenderable(options.renderer, {
      wrapMode: "char",
      fg: interaction.level === "critical"
        ? theme.error
        : interaction.level === "warning"
          ? theme.warning
          : theme.info,
      attributes: TextAttributes.BOLD,
    })
    const titlePrefix = interaction.level.toUpperCase()
    titleLine.content = interaction.title
      ? `${titlePrefix} • ${interaction.title}`
      : titlePrefix
    const messageLine = new TextRenderable(options.renderer, {
      wrapMode: "word",
      fg: theme.text,
    })
    const timeoutSuffix = interaction.timeout_sec
      ? ` • timeout ${interaction.timeout_sec}s`
      : ""
    messageLine.content = `${interaction.message}${timeoutSuffix}`
    box.add(titleLine)
    box.add(messageLine)
    renderInteractionChoices(options, box, interaction, focused)
  }
  renderQueuedPromptStrip(options, box, queuedPrompts, focused, selectedQueuedPromptIndex)
  box.requestRender?.()
}

function renderQueuedPromptStrip(
  options: InteractionStripRenderOptions,
  container: BoxRenderable,
  items: readonly QueuedPromptStripItem[],
  focused: boolean,
  selectedIndex: number,
): void {
  if (items.length === 0) {
    return
  }
  const title = new TextRenderable(options.renderer, {
    content: queuedPromptTitleLabel(items.length, focused),
    fg: theme.info,
    attributes: TextAttributes.BOLD,
    wrapMode: "none",
  })
  container.add(title)
  items.forEach((item, index) => {
    const selected = focused && index === selectedIndex
    const row = new BoxRenderable(options.renderer, {
      width: "100%",
      flexDirection: "row",
      gap: 1,
      flexShrink: 0,
    })
    const prompt = new TextRenderable(options.renderer, {
      content: selected ? `> ${item.prompt}` : `  ${item.prompt}`,
      fg: selected ? theme.primary : theme.text,
      wrapMode: "word",
    })
    prompt.flexBasis = 0
    prompt.flexGrow = 1
    prompt.flexShrink = 1
    row.add(prompt)
    const meta = new TextRenderable(options.renderer, {
      content: queuedPromptMetaLabel(item),
      fg: theme.textMuted,
      attributes: TextAttributes.BOLD,
      wrapMode: "none",
    })
    meta.flexShrink = 0
    row.add(meta)
    row.add(renderQueuedPromptAction(options, item, "steer", queuedPromptActionLabel("steer", selected)))
    row.add(renderQueuedPromptAction(options, item, "cancel", queuedPromptActionLabel("cancel", selected)))
    container.add(row)
  })
}

function renderQueuedPromptAction(
  options: InteractionStripRenderOptions,
  item: QueuedPromptStripItem,
  action: "steer" | "cancel",
  label: string,
): TextRenderable {
  const disabled = action === "steer" ? item.canSteer !== true : item.canCancel !== true
  const text = new TextRenderable(options.renderer, {
    content: disabled ? label : `[${label}]`,
    fg: disabled ? theme.textMuted : theme.primary,
    attributes: disabled ? TextAttributes.NONE : TextAttributes.BOLD,
    wrapMode: "none",
  })
  text.flexShrink = 0
  text.onMouseUp = (event) => {
    if (disabled || event.button !== MouseButton.LEFT) {
      return
    }
    event.stopPropagation()
    startTimeout(() => {
      options.onQueuedPromptAction(item, action)
    }, 0)
  }
  return text
}

function renderInteractionChoices(
  options: InteractionStripRenderOptions,
  container: BoxRenderable,
  interaction: RuntimeInteraction,
  focused: boolean,
): void {
  const choiceCount = interaction.choices.length + (interaction.custom_choice ? 1 : 0)
  const selectedIndex = Math.min(
    options.selectedChoiceIndex(interaction.id),
    Math.max(0, choiceCount - 1),
  )
  options.setSelectedChoiceIndex(interaction.id, selectedIndex)
  const choicesBox = new BoxRenderable(options.renderer, {
    flexDirection: "row",
    gap: 1,
    flexShrink: 0,
  })
  interaction.choices.forEach((choice, index) => {
    const text = new TextRenderable(options.renderer, { wrapMode: "none" })
    const selected = focused && index === selectedIndex
    const tone = choice.style === "danger"
      ? theme.error
      : choice.style === "secondary"
        ? theme.textMuted
        : theme.primary
    text.content = `${selected ? ">" : " "} ${index + 1}.${choice.label}`
    text.fg = selected ? theme.background : tone
    text.bg = selected ? tone : undefined
    text.attributes = selected ? TextAttributes.BOLD : TextAttributes.NONE
    choicesBox.add(text)
  })
  if (interaction.custom_choice) {
    const index = interaction.choices.length
    const text = new TextRenderable(options.renderer, { wrapMode: "none" })
    const selected = focused && index === selectedIndex
    const editing = options.customEditing(interaction.id)
    const value = options.customReply(interaction.id)
    const placeholder = interaction.custom_choice.placeholder ?? "type another option"
    const renderedValue = renderInteractionCustomChoiceValue(
      value,
      placeholder,
      interaction.custom_choice.input_kind,
    )
    text.content = `${selected ? ">" : " "} ${index + 1}.${interaction.custom_choice.label}: ${renderedValue}${editing ? "_" : ""}`
    text.fg = selected ? theme.background : theme.primary
    text.bg = selected ? theme.primary : undefined
    text.attributes = selected ? TextAttributes.BOLD : TextAttributes.NONE
    choicesBox.add(text)
  }
  container.add(choicesBox)
}
