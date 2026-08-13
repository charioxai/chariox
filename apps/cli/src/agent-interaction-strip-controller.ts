import type { RuntimeInteraction } from "./cli-types.js"
import type { QueuedPromptStripItem } from "@chariox/kernel-client/queued-prompt-strip-state"

type AgentInteractionStripRenderOptions<TRenderer, TBox, TAgent extends { id: string }> = {
  renderer: TRenderer
  primaryBox: TBox | undefined
  auxiliaryBoxes: Array<TBox | undefined>
  visibleAgents: Array<TAgent | null | undefined>
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

export type AgentInteractionStripControllerDeps<
  TRenderer,
  TBox,
  TAgent extends { id: string },
> = {
  renderer: TRenderer
  primaryBox: () => TBox | undefined
  auxiliaryBoxes: () => Array<TBox | undefined>
  visibleAgents: () => Array<TAgent | null | undefined>
  maxAgentsPerScreen: () => number
  focusedAgentId: () => string | null
  activeInteractionForAgent: (agentId: string | null | undefined) => RuntimeInteraction | null
  selectedChoiceIndex: (interactionId: string) => number
  setSelectedChoiceIndex: (interactionId: string, index: number) => void
  customReply: (interactionId: string) => string
  customEditing: (interactionId: string) => boolean
  queuedPromptStripItemsForAgent: (agentId: string | null | undefined) => readonly QueuedPromptStripItem[]
  selectedQueuedPromptIndexForAgent: (agentId: string | null | undefined) => number
  onQueuedPromptAction: (item: QueuedPromptStripItem, action: "steer" | "cancel") => void
  renderStrips: (options: AgentInteractionStripRenderOptions<TRenderer, TBox, TAgent>) => void
}

export function createAgentInteractionStripController<
  TRenderer,
  TBox,
  TAgent extends { id: string },
>(
  deps: AgentInteractionStripControllerDeps<TRenderer, TBox, TAgent>,
) {
  return {
    render() {
      deps.renderStrips({
        renderer: deps.renderer,
        primaryBox: deps.primaryBox(),
        auxiliaryBoxes: deps.auxiliaryBoxes(),
        visibleAgents: deps.visibleAgents(),
        maxAgentsPerScreen: deps.maxAgentsPerScreen(),
        focusedAgentId: deps.focusedAgentId(),
        activeInteractionForAgent: deps.activeInteractionForAgent,
        selectedChoiceIndex: deps.selectedChoiceIndex,
        setSelectedChoiceIndex: deps.setSelectedChoiceIndex,
        customReply: deps.customReply,
        customEditing: deps.customEditing,
        queuedPromptStripItemsForAgent: deps.queuedPromptStripItemsForAgent,
        selectedQueuedPromptIndexForAgent: deps.selectedQueuedPromptIndexForAgent,
        onQueuedPromptAction: deps.onQueuedPromptAction,
      })
    },
  }
}
