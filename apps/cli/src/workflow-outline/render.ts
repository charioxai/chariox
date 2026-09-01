import { useRenderer } from "@opentui/solid"
import { BoxRenderable, MouseButton, TextAttributes, TextRenderable } from "@opentui/core"

import type { AgentInstance, WorkflowDefinition, WorkflowEndpointRuntimeInstance, WorkflowRun } from "../cli-types.js"
import { theme } from "../theme.js"
import type { WorkflowComponentSelection } from "../workflow-component-selection.js"
import { resolveSelectedWorkflow, resolveSelectedWorkflowNodeId } from "../workflow-graph/selection.js"
import { buildWorkflowOutline } from "./build.js"
import { buildWorkflowOutlineNodeLines } from "./text.js"

export function buildWorkflowOutlineRenderable(
  renderer: ReturnType<typeof useRenderer>,
    options: {
      workflows: WorkflowDefinition[]
      agents: AgentInstance[]
      workflowRuns: WorkflowRun[]
      workflowRuntimeInstances?: WorkflowEndpointRuntimeInstance[]
      selectedWorkflowId: string | null
    selectedNodeId: string | null
    selectedComponent?: WorkflowComponentSelection | null
    onSelectNode: (nodeId: string | null) => void
    onSelectComponent?: (selection: WorkflowComponentSelection, backingNodeId: string | null) => void
  },
) {
  const selectedWorkflow = resolveSelectedWorkflow(options.workflows, options.selectedWorkflowId)
  if (!selectedWorkflow) {
    return null
  }

  const selectedNodeId = resolveSelectedWorkflowNodeId(selectedWorkflow, options.selectedNodeId)
  const outline = buildWorkflowOutline({
    workflow: selectedWorkflow,
    agents: options.agents,
    workflowRuns: options.workflowRuns,
    ...(options.workflowRuntimeInstances !== undefined ? { workflowRuntimeInstances: options.workflowRuntimeInstances } : {}),
    selectedNodeId,
    ...(options.selectedComponent !== undefined ? { selectedComponent: options.selectedComponent } : {}),
  })

  const wrapper = new BoxRenderable(renderer, {
    flexDirection: "column",
    gap: 1,
    width: "100%",
    paddingTop: 1,
    paddingBottom: 1,
  })

  const workflowLabel = outline.workflowAlias
    ? `${outline.workflowId} (${outline.workflowAlias})`
    : outline.workflowId
  const agentsLabel = outline.agentLabels.length > 0 ? `, agents: ${outline.agentLabels.join(", ")}` : ""
  wrapper.add(
    new TextRenderable(renderer, {
      content: `workflow: ${workflowLabel}${agentsLabel}`,
      fg: theme.primary,
      attributes: TextAttributes.BOLD,
      wrapMode: "word",
    }),
  )
  if (outline.workflowRunId) {
    wrapper.add(
      new TextRenderable(renderer, {
        content: `run: ${outline.workflowRunId} • status ${String(outline.workflowRunStatus).toLowerCase()}`,
        fg: theme.secondary,
        wrapMode: "word",
      }),
    )
  }
  wrapper.add(
    new TextRenderable(renderer, {
      content: `Tab cycles nodes • nodes ${outline.nodeCount} • endpoints ${outline.endpointCount} • edges ${outline.edgeCount}`,
      fg: theme.textMuted,
      wrapMode: "word",
    }),
  )

  for (const node of outline.nodes) {
    const nodeBox = new BoxRenderable(renderer, {
      width: "100%",
      flexDirection: "column",
      border: ["left", "top", "right", "bottom"],
      borderColor: node.missing ? theme.error : node.selectedComponent ? theme.primary : node.selected ? theme.secondary : theme.borderSubtle,
      backgroundColor: node.selectedComponent || node.selected ? theme.backgroundPanel : theme.backgroundElement,
      paddingLeft: 1,
      paddingRight: 1,
      paddingTop: 0,
      paddingBottom: 0,
    })
    nodeBox.onMouseUp = (event) => {
      if (event.button !== MouseButton.LEFT) {
        return
      }
      event.stopPropagation()
      if (options.onSelectComponent) {
        options.onSelectComponent({ kind: "node", id: node.id }, node.id)
      } else {
        options.onSelectNode(node.id)
      }
    }
    for (const line of buildNodeLines(node)) {
      nodeBox.add(
        new TextRenderable(renderer, {
          content: line.content,
          fg: line.fg,
          attributes: line.attributes ?? TextAttributes.NONE,
          wrapMode: "word",
        }),
      )
    }
    if (options.onSelectComponent) {
      for (const endpoint of node.entryEndpoints) {
        nodeBox.add(buildComponentSelector(renderer, {
          label: `${endpoint.id}${endpoint.alias ? ` (${endpoint.alias})` : ""} • pool ${endpoint.busyCount}/${endpoint.maxInstances} busy`,
          selected: options.selectedComponent?.kind === "endpoint" && options.selectedComponent.id === endpoint.id,
          onSelect: () => options.onSelectComponent?.({ kind: "endpoint", id: endpoint.id }, endpoint.entryNodeId),
        }))
      }
      for (const edge of node.outgoingEdges) {
        nodeBox.add(buildComponentSelector(renderer, {
          label: `edge ${edge.id} -> ${edge.nodeId}`,
          selected: options.selectedComponent?.kind === "edge" && options.selectedComponent.id === edge.id,
          onSelect: () => options.onSelectComponent?.({ kind: "edge", id: edge.id }, edge.fromNodeId),
        }))
      }
    }
    wrapper.add(nodeBox)
  }

  if (options.workflows.length > 1) {
    wrapper.add(
      new TextRenderable(renderer, {
        content: `workflows: ${options.workflows.map((workflow) => workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id).join(", ")}`,
        fg: theme.secondary,
        wrapMode: "word",
      }),
    )
  }
  if (outline.nodes.length === 0) {
    wrapper.add(
      new TextRenderable(renderer, {
        content: "Add nodes with /workflow node add <workflow-ref> <agent-id>.",
        fg: theme.warning,
        wrapMode: "word",
      }),
    )
  }

  return wrapper
}

function buildComponentSelector(
  renderer: ReturnType<typeof useRenderer>,
  options: { label: string; selected: boolean; onSelect: () => void },
) {
  const row = new BoxRenderable(renderer, {
    width: "100%",
    backgroundColor: options.selected ? theme.backgroundPanel : theme.backgroundElement,
    paddingLeft: 1,
  })
  row.onMouseUp = (event) => {
    if (event.button !== MouseButton.LEFT) {
      return
    }
    event.stopPropagation()
    options.onSelect()
  }
  row.add(new TextRenderable(renderer, {
    content: `> ${options.label}`,
    fg: options.selected ? theme.primary : theme.textMuted,
    attributes: options.selected ? TextAttributes.BOLD : TextAttributes.NONE,
    wrapMode: "word",
  }))
  return row
}

function buildNodeLines(node: Parameters<typeof buildWorkflowOutlineNodeLines>[0]) {
  return buildWorkflowOutlineNodeLines(node).map((line) => ({
    content: line.content,
    fg: resolveLineColor(line.tone),
    attributes: line.emphasis === "bold" ? TextAttributes.BOLD : TextAttributes.NONE,
  }))
}

function resolveLineColor(tone: "title" | "section" | "detail" | "muted") {
  switch (tone) {
    case "title":
      return theme.text
    case "section":
      return theme.secondary
    case "detail":
      return theme.textMuted
    case "muted":
    default:
      return theme.textMuted
  }
}
