import type { TriggerDelivery } from "@chariox/kernel-client/kernel-types"
import type { RuntimeInteraction } from "./cli-types.js"
import type { ProviderLoginStripState } from "./provider-login-interaction-controller.js"
import type { AgentTriggerStripItem } from "@chariox/kernel-client/agent-trigger-state"
import type { QueuedPromptStripItem } from "@chariox/kernel-client/queued-prompt-strip-state"
import type { QueuedJob } from "@chariox/kernel-client/queued-jobs"
import type { SubAgentStripeItem } from "./interaction-strip-renderer.js"

type AgentInteractionStripRenderOptions<TRenderer, TBox, TAgent extends { id: string }> = {
  renderer: TRenderer
  primaryBox: TBox | undefined
  auxiliaryBoxes: Array<TBox | undefined>
  visibleAgents: Array<TAgent | null | undefined>
  maxAgentsPerScreen: number
  focusedAgentId: string | null
  activeInteractionForAgent: (agentId: string | null | undefined) => RuntimeInteraction | null
  askingAgentLabel?: (interaction: RuntimeInteraction) => string | null
  selectedChoiceIndex: (interactionId: string) => number
  setSelectedChoiceIndex: (interactionId: string, index: number) => void
  customReply: (interactionId: string) => string
  customEditing: (interactionId: string) => boolean
  queuedPromptStripItemsForAgent: (agentId: string | null | undefined) => readonly QueuedPromptStripItem[]
  subAgentStripeForAgent?: (agentId: string | null | undefined) => SubAgentStripeItem | null
  queuedJobsForAgent?: (agentId: string | null | undefined) => readonly QueuedJob[]
  onQueuedJobAction?: (job: QueuedJob, action: "remote" | "stop") => void
  remoteJobLinesForAgent?: (agentId: string | null | undefined) => readonly string[]
  selectedQueuedPromptIndexForAgent: (agentId: string | null | undefined) => number
  onQueuedPromptAction: (item: QueuedPromptStripItem, action: "steer" | "cancel") => void
  agentTriggerStripItemsForAgent: (agentId: string | null | undefined) => readonly AgentTriggerStripItem[]
  unconfirmedTriggerDeliveriesForAgent?: (agentId: string | null | undefined) => readonly TriggerDelivery[]
  providerLoginState?: (interaction: RuntimeInteraction) => ProviderLoginStripState | null
  focusCustomChoiceOnce?: (interactionId: string, index: number, requestedAtMs: number) => void
  onProviderLoginLinkClick?: (interaction: RuntimeInteraction) => void
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
  askingAgentLabel?: (interaction: RuntimeInteraction) => string | null
  selectedChoiceIndex: (interactionId: string) => number
  setSelectedChoiceIndex: (interactionId: string, index: number) => void
  customReply: (interactionId: string) => string
  customEditing: (interactionId: string) => boolean
  queuedPromptStripItemsForAgent: (agentId: string | null | undefined) => readonly QueuedPromptStripItem[]
  subAgentStripeForAgent?: (agentId: string | null | undefined) => SubAgentStripeItem | null
  queuedJobsForAgent?: (agentId: string | null | undefined) => readonly QueuedJob[]
  onQueuedJobAction?: (job: QueuedJob, action: "remote" | "stop") => void
  remoteJobLinesForAgent?: (agentId: string | null | undefined) => readonly string[]
  selectedQueuedPromptIndexForAgent: (agentId: string | null | undefined) => number
  onQueuedPromptAction: (item: QueuedPromptStripItem, action: "steer" | "cancel") => void
  agentTriggerStripItemsForAgent: (agentId: string | null | undefined) => readonly AgentTriggerStripItem[]
  unconfirmedTriggerDeliveriesForAgent?: (agentId: string | null | undefined) => readonly TriggerDelivery[]
  providerLoginState?: (interaction: RuntimeInteraction) => ProviderLoginStripState | null
  focusCustomChoiceOnce?: (interactionId: string, index: number, requestedAtMs: number) => void
  onProviderLoginLinkClick?: (interaction: RuntimeInteraction) => void
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
        ...(deps.askingAgentLabel ? { askingAgentLabel: deps.askingAgentLabel } : {}),
        selectedChoiceIndex: deps.selectedChoiceIndex,
        setSelectedChoiceIndex: deps.setSelectedChoiceIndex,
        customReply: deps.customReply,
        customEditing: deps.customEditing,
        queuedPromptStripItemsForAgent: deps.queuedPromptStripItemsForAgent,
        ...(deps.subAgentStripeForAgent ? { subAgentStripeForAgent: deps.subAgentStripeForAgent } : {}),
        ...(deps.queuedJobsForAgent ? { queuedJobsForAgent: deps.queuedJobsForAgent } : {}),
        ...(deps.onQueuedJobAction ? { onQueuedJobAction: deps.onQueuedJobAction } : {}),
        ...(deps.remoteJobLinesForAgent ? { remoteJobLinesForAgent: deps.remoteJobLinesForAgent } : {}),
        selectedQueuedPromptIndexForAgent: deps.selectedQueuedPromptIndexForAgent,
        onQueuedPromptAction: deps.onQueuedPromptAction,
        agentTriggerStripItemsForAgent: deps.agentTriggerStripItemsForAgent,
        ...(deps.unconfirmedTriggerDeliveriesForAgent ? { unconfirmedTriggerDeliveriesForAgent: deps.unconfirmedTriggerDeliveriesForAgent } : {}),
        ...(deps.providerLoginState ? { providerLoginState: deps.providerLoginState } : {}),
        ...(deps.focusCustomChoiceOnce ? { focusCustomChoiceOnce: deps.focusCustomChoiceOnce } : {}),
        ...(deps.onProviderLoginLinkClick ? { onProviderLoginLinkClick: deps.onProviderLoginLinkClick } : {}),
      })
    },
  }
}
