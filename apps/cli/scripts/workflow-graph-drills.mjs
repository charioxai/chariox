import {
  buildWorkflowGraphLayout,
} from "../dist/workflow-graph/index.js"
import {
  buildWorkflowGraphDrillCases,
  renderWorkflowGraphAscii,
  validateWorkflowGraphLayout,
} from "../dist/workflow-graph-drills.js"

const sections = []
let failed = false

for (const drillCase of buildWorkflowGraphDrillCases()) {
  const layout = buildWorkflowGraphLayout({
    workflow: drillCase.workflow,
    agents: drillCase.agents,
    workflowRuns: [],
    selectedNodeId: null,
  })
  const validation = validateWorkflowGraphLayout(layout)
  const issues = [
    ...validation.nodeOverlaps.map(({ leftNodeId, rightNodeId }) => `node overlap: ${leftNodeId} vs ${rightNodeId}`),
    ...validation.diagonalSegments.map(({ edgeId, fromNodeId, toNodeId }) => `diagonal segment: ${edgeId} (${fromNodeId} -> ${toNodeId})`),
    ...validation.edgeNodeCollisions.map(({ edgeId, nodeId }) => `edge-node collision: ${edgeId} through ${nodeId}`),
    ...validation.reciprocalOverlaps.map(({ forwardEdgeId, reverseEdgeId }) => `reciprocal overlap: ${forwardEdgeId} and ${reverseEdgeId}`),
  ]
  if (issues.length > 0) {
    failed = true
  }
  sections.push([
    `=== ${drillCase.title} (${drillCase.id}) ===`,
    `nodes=${layout.nodes.length} edges=${layout.edges.length} width=${layout.width} height=${layout.height}`,
    issues.length === 0 ? "issues=none" : `issues=${issues.join("; ")}`,
    "",
    renderWorkflowGraphAscii(layout),
  ].join("\n"))
}

console.log(sections.join("\n\n"))

if (failed) {
  process.exitCode = 1
}
