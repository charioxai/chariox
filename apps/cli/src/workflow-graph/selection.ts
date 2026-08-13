import type { WorkflowDefinition } from "../cli-types.js"
import {
  resolveSelectedWorkflow as sharedResolveSelectedWorkflow,
  resolveSelectedWorkflowNodeId as sharedResolveSelectedWorkflowNodeId,
} from "@chariox/kernel-client/workflow-prompt-state"

export function resolveSelectedWorkflow(
  workflows: WorkflowDefinition[],
  selectedWorkflowId: string | null,
): WorkflowDefinition | null {
  return sharedResolveSelectedWorkflow(workflows, selectedWorkflowId)
}

export function resolveSelectedWorkflowNodeId(
  workflow: WorkflowDefinition | null,
  selectedNodeId: string | null,
): string | null {
  return sharedResolveSelectedWorkflowNodeId(workflow, selectedNodeId)
}

export function cycleWorkflowNodeId(
  workflow: WorkflowDefinition | null,
  selectedNodeId: string | null,
  step = 1,
): string | null {
  const nodes = workflow?.nodes ?? []
  if (nodes.length === 0) {
    return null
  }
  if (!selectedNodeId) {
    return nodes[0]?.id ?? null
  }
  const currentIndex = nodes.findIndex((node) => node.id === selectedNodeId)
  if (currentIndex < 0) {
    return nodes[0]?.id ?? null
  }
  const nextIndex = modulo(currentIndex + step, nodes.length)
  return nodes[nextIndex]?.id ?? null
}

function modulo(value: number, divisor: number) {
  return ((value % divisor) + divisor) % divisor
}
