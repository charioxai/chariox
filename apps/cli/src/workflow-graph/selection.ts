import type { WorkflowDefinition } from "../cli-types.js"

export function resolveSelectedWorkflow(
  workflows: WorkflowDefinition[],
  selectedWorkflowId: string | null,
): WorkflowDefinition | null {
  if (workflows.length === 0) {
    return null
  }
  if (selectedWorkflowId) {
    const exact = workflows.find((workflow) => workflow.id === selectedWorkflowId)
    if (exact) {
      return exact
    }
  }
  return workflows[0] ?? null
}

export function resolveSelectedWorkflowNodeId(
  workflow: WorkflowDefinition | null,
  selectedNodeId: string | null,
): string | null {
  const nodes = workflow?.nodes ?? []
  if (nodes.length === 0) {
    return null
  }
  if (selectedNodeId && nodes.some((node) => node.id === selectedNodeId)) {
    return selectedNodeId
  }
  return nodes[0]?.id ?? null
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
