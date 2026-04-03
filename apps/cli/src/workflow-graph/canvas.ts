import { useRenderer } from "@opentui/solid"
import { BoxRenderable, MouseButton, TextAttributes, TextRenderable } from "@opentui/core"

import type { AgentInstance, WorkflowDefinition, WorkflowRun } from "../cli-types.js"
import { theme } from "../theme.js"
import { buildWorkflowGraphLayout } from "./layout.js"
import { buildWorkflowEdgeCells } from "./render.js"
import { resolveSelectedWorkflow, resolveSelectedWorkflowNodeId } from "./selection.js"

export function buildWorkflowCanvasRenderable(
  renderer: ReturnType<typeof useRenderer>,
  options: {
    workflows: WorkflowDefinition[]
    agents: AgentInstance[]
    workflowRuns: WorkflowRun[]
    selectedWorkflowId: string | null
    selectedNodeId: string | null
    onSelectNode: (nodeId: string | null) => void
  },
) {
  const selectedWorkflow = resolveSelectedWorkflow(options.workflows, options.selectedWorkflowId)
  if (!selectedWorkflow) {
    return null
  }

  const selectedNodeId = resolveSelectedWorkflowNodeId(selectedWorkflow, options.selectedNodeId)
  const layout = buildWorkflowGraphLayout({
    workflow: selectedWorkflow,
    agents: options.agents,
    workflowRuns: options.workflowRuns,
    selectedNodeId,
  })
  const wrapper = new BoxRenderable(renderer, {
    flexDirection: "column",
    gap: 1,
    width: "100%",
    minWidth: layout.width,
    minHeight: layout.height + 6,
    paddingTop: 1,
    paddingBottom: 1,
  })

  const agentLabels = options.agents.map((agent) => agent.agent_ref ?? agent.id)
  const workflowLabel = selectedWorkflow.alias
    ? `${layout.workflowId} (${selectedWorkflow.alias})`
    : layout.workflowId
  const agentsLabel = agentLabels.length > 0 ? `, agents: ${agentLabels.join(", ")}` : ""

  wrapper.add(
    new TextRenderable(renderer, {
      content: `workflow: ${workflowLabel}${agentsLabel}`,
      fg: theme.primary,
      attributes: TextAttributes.BOLD,
      wrapMode: "word",
    }),
  )
  if (layout.workflowRunId) {
    wrapper.add(
      new TextRenderable(renderer, {
        content: `run: ${layout.workflowRunId} • status ${String(layout.workflowRunStatus).toLowerCase()}`,
        fg: theme.secondary,
        wrapMode: "word",
      }),
    )
  }
  wrapper.add(
    new TextRenderable(renderer, {
      content: `Tab cycles nodes • endpoints ${layout.endpoints.length} • edges ${layout.edges.length}`,
      fg: theme.textMuted,
      wrapMode: "word",
    }),
  )

  const graphArea = new BoxRenderable(renderer, {
    width: layout.width,
    minWidth: layout.width,
    height: layout.height,
    minHeight: layout.height,
    position: "relative",
  })

  for (const node of layout.nodes) {
    const nodeBox = new BoxRenderable(renderer, {
      position: "absolute",
      left: node.x,
      top: node.y,
      width: node.width,
      minWidth: node.width,
      height: node.height,
      minHeight: node.height,
      flexDirection: "column",
      border: ["left", "top", "right", "bottom"],
      borderColor: node.missing ? theme.error : node.selected ? theme.primary : theme.borderSubtle,
      backgroundColor: node.selected ? theme.backgroundPanel : theme.backgroundElement,
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
      options.onSelectNode(node.id)
    }
    for (const [lineIndex, line] of node.lines.entries()) {
      nodeBox.add(
        new TextRenderable(renderer, {
          content: line,
          fg: lineIndex === 0 ? theme.text : theme.textMuted,
          attributes: lineIndex === 0 ? TextAttributes.BOLD : TextAttributes.NONE,
          wrapMode: "none",
        }),
      )
    }
    graphArea.add(nodeBox)
  }

  for (const endpoint of layout.endpoints) {
    graphArea.add(
      new TextRenderable(renderer, {
        position: "absolute",
        left: endpoint.labelX,
        top: endpoint.labelY,
        content: endpoint.label,
        fg: theme.secondary,
        wrapMode: "none",
      }),
    )
    graphArea.add(
      new TextRenderable(renderer, {
        position: "absolute",
        left: endpoint.markerX,
        top: endpoint.markerY,
        content: "o",
        fg: theme.warning,
        wrapMode: "none",
      }),
    )
    if (endpoint.markerY + 1 < layout.height) {
      graphArea.add(
        new TextRenderable(renderer, {
          position: "absolute",
          left: endpoint.markerX,
          top: endpoint.markerY + 1,
          content: "|",
          fg: theme.warning,
          wrapMode: "none",
        }),
      )
    }
  }

  for (const edge of layout.edges) {
    for (const cell of buildWorkflowEdgeCells(edge.points)) {
      graphArea.add(
        new TextRenderable(renderer, {
          position: "absolute",
          left: cell.x,
          top: cell.y,
          content: cell.char,
          fg: theme.secondary,
          wrapMode: "none",
        }),
      )
    }
  }

  const footer = new BoxRenderable(renderer, {
    flexDirection: "column",
    gap: 0,
    marginTop: 1,
  })
  if (options.workflows.length > 1) {
    footer.add(
      new TextRenderable(renderer, {
        content: `workflows: ${options.workflows.map((workflow) => workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id).join(", ")}`,
        fg: theme.secondary,
        wrapMode: "word",
      }),
    )
  }
  if (layout.nodes.length === 0) {
    footer.add(
      new TextRenderable(renderer, {
        content: "Add nodes with /workflow node add <workflow-ref> <agent-id>.",
        fg: theme.warning,
        wrapMode: "word",
      }),
    )
  }

  wrapper.add(graphArea)
  if (footer.getChildrenCount() > 0) {
    wrapper.add(footer)
  }

  return wrapper
}
