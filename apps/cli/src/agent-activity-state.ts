import {
  deriveAllAgentsBusyState as kernelDeriveAllAgentsBusyState,
  deriveFocusedActivityLabel as kernelDeriveFocusedActivityLabel,
  deriveFocusedAgentBusy as kernelDeriveFocusedAgentBusy,
  nextAgentActivityLabels as kernelNextAgentActivityLabels,
  nextAgentBusyLatches as kernelNextAgentBusyLatches,
  readAgentBusyLatch as kernelReadAgentBusyLatch,
  resolveActiveToolLabelForAgent as kernelResolveActiveToolLabelForAgent,
  shouldPreserveAgentActivityLabel as kernelShouldPreserveAgentActivityLabel,
  type AgentBusyState,
} from "@arroba/kernel-client/session-runtime-transition"
import type { RuntimeSession } from "./cli-types.js"

export type ToolActivityUpdate = {
  tool?: string | null
  status?: string | null
}

export type { AgentBusyState }

export function readAgentBusyLatch(
  latches: Record<string, boolean>,
  agentId: string | null | undefined,
): boolean {
  return kernelReadAgentBusyLatch(latches, agentId)
}

export function nextAgentBusyLatches(
  current: Record<string, boolean>,
  agentId: string | null | undefined,
  busy: boolean,
): Record<string, boolean> {
  return kernelNextAgentBusyLatches(current, agentId, busy)
}

export function shouldPreserveAgentActivityLabel(options: {
  agentId: string | null | undefined
  session: RuntimeSession
  streamingAgentId: string | null
}): boolean {
  return kernelShouldPreserveAgentActivityLabel({
    agentId: options.agentId,
    session: options.session as Parameters<typeof kernelShouldPreserveAgentActivityLabel>[0]["session"],
    streamingAgentId: options.streamingAgentId,
  })
}

export function nextAgentActivityLabels(
  current: Record<string, string | null>,
  agentId: string | null | undefined,
  nextLabel: string | null,
  preserveCurrent: boolean,
): Record<string, string | null> {
  return kernelNextAgentActivityLabels(current, agentId, nextLabel, preserveCurrent)
}

export function resolveActiveToolLabelForAgent(options: {
  agentId: string | null | undefined
  visibleTranscriptAgentId: string | null
  activeToolLabels: Iterable<string>
  agentPaneToolUpdates: Iterable<ToolActivityUpdate> | null | undefined
}): string | null {
  return kernelResolveActiveToolLabelForAgent(options)
}

export function deriveFocusedActivityLabel(options: {
  focusedAgentId: string | null
  activeToolLabel: string | null
  agentActivityLabel: string | null
}): string | null {
  return kernelDeriveFocusedActivityLabel(options)
}

export function deriveFocusedAgentBusy(options: {
  focusedAgentId: string | null
  submitting: boolean
  submittingAgentId: string | null
  session: RuntimeSession
  streamingAgentId: string | null
  focusedActivityLabel: string | null
  agentBusyLatches: Record<string, boolean>
}): boolean {
  return kernelDeriveFocusedAgentBusy({
    ...options,
    session: options.session as Parameters<typeof kernelDeriveFocusedAgentBusy>[0]["session"],
  })
}

export function deriveAllAgentsBusyState(options: {
  submitting: boolean
  submittingAgentId: string | null
  session: RuntimeSession
  streamingAgentId: string | null
  agentActivityLabels: Record<string, string | null>
  agentBusyLatches: Record<string, boolean>
}): AgentBusyState[] {
  return kernelDeriveAllAgentsBusyState({
    ...options,
    session: options.session as Parameters<typeof kernelDeriveAllAgentsBusyState>[0]["session"],
  })
}
