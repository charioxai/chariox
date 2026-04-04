import { buildWorkflowGraphDrillCases } from "../dist/workflow-graph-drills.js"
import { buildWorkflowOutline } from "../dist/workflow-outline/build.js"
import { renderWorkflowOutlineToText } from "../dist/workflow-outline/text.js"

function assertIncludesAllGraphRefs(drillCase, rendered) {
  for (const node of drillCase.workflow.nodes ?? []) {
    if (!rendered.includes(`node ${node.id}`)) {
      throw new Error(`${drillCase.id}: missing node ref ${node.id}`)
    }
  }
  for (const edge of drillCase.workflow.edges ?? []) {
    if (!rendered.includes(edge.id)) {
      throw new Error(`${drillCase.id}: missing edge ref ${edge.id}`)
    }
    if (!rendered.includes(edge.from_node_id) || !rendered.includes(edge.to_node_id)) {
      throw new Error(`${drillCase.id}: missing adjacent node ref for edge ${edge.id}`)
    }
  }
}

for (const drillCase of buildWorkflowGraphDrillCases()) {
  const outline = buildWorkflowOutline({
    workflow: drillCase.workflow,
    agents: drillCase.agents,
    workflowRuns: [],
    selectedNodeId: drillCase.workflow.nodes?.[0]?.id ?? null,
  })
  const rendered = renderWorkflowOutlineToText(outline)
  assertIncludesAllGraphRefs(drillCase, rendered)
  console.log(`=== ${drillCase.title} (${drillCase.id}) ===`)
  console.log(rendered)
  console.log("")
}

console.log(`outline drills passed (${buildWorkflowGraphDrillCases().length} cases)`)
