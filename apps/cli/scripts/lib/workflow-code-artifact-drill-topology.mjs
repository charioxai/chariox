import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises'
import path from 'node:path'

import { applyWorkflowCodeArtifactRequest, applyWorkflowCodeRequest, createWorkflowCodeArtifactRequest, exportWorkflowCodeArtifactRequest, exportWorkflowCodeSourceRequest, focusAgentRequest, getProviderRunRequest, getSessionStateRequest, importWorkflowCodeArtifactRequest, invokeWorkflowEndpointRequest, launchProviderRunRequest, validateWorkflowCodeRequest } from '@arroba/kernel-client'
import { buildWorkflowOutline } from '../../dist/workflow-outline/build.js'
import { renderWorkflowOutlineToText } from '../../dist/workflow-outline/text.js'
import { assert, defaultToyExpectation, expectationFromDefinition, providerFamily, providerSetForDefinition, realProviderRebindingsForDefinition, rebindingByNode, rebindingsForDefinition, repoRoot, sha256Hex, shouldPrelaunchRealProvider, sleep, stage, topologyRuntimeExpectation, topologyRuntimeRebindingsForDefinition, unwrap, writeSourceDirectoryExport } from './workflow-code-artifact-drill-runtime.mjs'
import { waitForCompletedWorkflowRun, waitForProviderRunReady } from './workflow-code-artifact-drill-waits.mjs'

export const EXAMPLE_TOPOLOGY_EXPECTATIONS = {
  'adversarial-verification.js': {
    alias: 'pattern-adversarial-verification',
    schemas: ['proposal', 'critique', 'final_output'],
    nodes: ['proposer', 'critic', 'judge'],
    edges: ['proposal_to_critic', 'critic_to_judge'],
    endpoints: ['entry'],
    completers: ['judge'],
    providers: ['codex', 'claude', 'opencode'],
  },
  'evaluator-optimizer.js': {
    alias: 'pattern-evaluator-optimizer',
    schemas: ['candidate', 'evaluation', 'final_output'],
    nodes: ['coordinator', 'optimizer', 'evaluator'],
    edges: ['coordinator_to_optimizer', 'optimizer_to_evaluator', 'revision_loop_optimizer'],
    endpoints: ['entry'],
    completers: ['evaluator'],
    providers: ['codex', 'claude'],
    hasLoop: true,
  },
  'fan-out-synthesize.js': {
    alias: 'pattern-fan-out-synthesize',
    schemas: ['assignment', 'finding', 'final_output'],
    nodes: ['planner', 'worker_a', 'worker_b', 'synthesizer'],
    edges: ['planner_to_worker_a', 'planner_to_worker_b', 'worker_a_to_synthesizer', 'worker_b_to_synthesizer'],
    endpoints: ['entry'],
    completers: ['synthesizer'],
    providers: ['codex', 'claude', 'opencode'],
    waitForAll: ['synthesizer'],
  },
  'generate-filter.js': {
    alias: 'pattern-generate-filter',
    schemas: ['candidates', 'filtered', 'final_output'],
    nodes: ['coordinator', 'generator', 'filter', 'finisher'],
    edges: ['coordinator_to_generator', 'generator_candidates', 'filtered_candidates'],
    endpoints: ['entry'],
    completers: ['finisher'],
    providers: ['codex', 'claude', 'opencode'],
  },
  'loop-until-done.js': {
    alias: 'pattern-loop-until-done',
    schemas: ['work_product', 'feedback', 'final_output'],
    nodes: ['worker', 'checker'],
    edges: ['work_to_checker', 'revise_loop'],
    endpoints: ['entry'],
    completers: ['checker'],
    providers: ['codex', 'claude'],
    hasLoop: true,
  },
  'orchestrator-workers.js': {
    alias: 'pattern-orchestrator-workers',
    schemas: ['assignment', 'result', 'final_output'],
    nodes: ['orchestrator', 'worker', 'synthesizer'],
    edges: ['orchestrator_to_worker', 'worker_to_synthesizer'],
    endpoints: ['entry'],
    completers: ['synthesizer'],
    providers: ['codex', 'opencode', 'claude'],
  },
  'parallelization.js': {
    alias: 'pattern-parallelization',
    schemas: ['review_task', 'review_result', 'final_output'],
    nodes: ['dispatcher', 'reviewer_01', 'reviewer_02', 'aggregator'],
    edges: ['dispatcher_to_reviewer_01', 'dispatcher_to_reviewer_02', 'reviewer_01_to_aggregator', 'reviewer_02_to_aggregator'],
    endpoints: ['entry'],
    completers: ['aggregator'],
    providers: ['codex', 'claude', 'opencode'],
    waitForAll: ['aggregator'],
  },
  'planner-worker-reviewer.js': {
    alias: 'pattern-planner-worker-reviewer',
    schemas: ['implementation_assignment', 'implementation_result', 'revision_request', 'accepted_step_report', 'final_output'],
    nodes: ['planner', 'worker', 'reviewer'],
    edges: ['planner_to_worker', 'worker_to_reviewer', 'reviewer_to_worker', 'reviewer_to_planner'],
    endpoints: ['entry'],
    completers: ['planner'],
    providers: ['codex', 'claude'],
    hasLoop: true,
  },
  'prompt-chaining.js': {
    alias: 'pattern-prompt-chaining',
    schemas: ['handoff', 'final_output'],
    nodes: ['drafter', 'refiner'],
    edges: ['drafter_to_refiner'],
    endpoints: ['entry'],
    completers: ['refiner'],
    providers: ['codex', 'claude'],
  },
  'routing.js': {
    alias: 'pattern-routing',
    schemas: ['route_task', 'final_output'],
    nodes: ['classifier', 'code_specialist', 'research_specialist'],
    edges: ['to_code_specialist', 'to_research_specialist'],
    endpoints: ['entry'],
    completers: ['code_specialist', 'research_specialist'],
    providers: ['codex', 'opencode', 'claude'],
    multiEdgeRouter: 'classifier',
  },
  'tournament.js': {
    alias: 'pattern-tournament',
    schemas: ['contest_prompt', 'entry', 'final_output'],
    nodes: ['seeder', 'contestant_01', 'contestant_02', 'final_judge'],
    edges: ['seed_01', 'seed_02', 'contestant_01_to_final_judge', 'contestant_02_to_final_judge'],
    endpoints: ['entry'],
    completers: ['final_judge'],
    providers: ['codex', 'claude', 'opencode'],
    waitForAll: ['final_judge'],
  },
}

export function assertSameSet(actual, expected, label) {
  const sortedActual = [...actual].sort()
  const sortedExpected = [...expected].sort()
  assert(
    JSON.stringify(sortedActual) === JSON.stringify(sortedExpected),
    `${label} mismatch`,
    { actual: sortedActual, expected: sortedExpected },
  )
}

export function validationDiagnostics(validation) {
  return validation?.diagnostics ?? []
}

export function validateExampleTopologyDefinition(exampleName, definition, validation) {
  const expectation = EXAMPLE_TOPOLOGY_EXPECTATIONS[exampleName]
  assert(expectation, `missing workflow-code topology expectation for ${exampleName}`)

  assert(definition?.workflow?.alias === expectation.alias, `${exampleName} alias mismatch`, definition?.workflow)
  assert(definition.workflow?.max_concurrent === 32, `${exampleName} should set maxConcurrent to 32`, definition.workflow)
  assert(definition.workflow?.run_output_schema === 'final_output', `${exampleName} should set workflow-level final output schema`, definition.workflow)
  assertSameSet((definition.schemas ?? []).map((schema) => schema.handle), expectation.schemas, `${exampleName} schemas`)
  assertSameSet((definition.nodes ?? []).map((node) => node.handle), expectation.nodes, `${exampleName} nodes`)
  assertSameSet((definition.edges ?? []).map((edge) => edge.handle), expectation.edges, `${exampleName} edges`)
  assertSameSet((definition.endpoints ?? []).map((endpoint) => endpoint.handle), expectation.endpoints, `${exampleName} endpoints`)
  assertSameSet(
    (definition.nodes ?? [])
      .filter((node) => node.can_complete_workflow_run === true)
      .map((node) => node.handle),
    expectation.completers,
    `${exampleName} completion nodes`,
  )

  for (const schema of definition.schemas ?? []) {
    assert(schema.schema?.type === 'object', `${exampleName} schema ${schema.handle} should be an object schema`, schema)
  }
  for (const node of definition.nodes ?? []) {
    assert(node.instructions, `${exampleName} node ${node.handle} should include node-level instructions`, node)
    assert(node.canvas && Number.isFinite(node.canvas.x) && Number.isFinite(node.canvas.y), `${exampleName} node ${node.handle} should include canvas coordinates`, node)
    assert(node.agent?.kind === 'create', `${exampleName} node ${node.handle} should create an agent for portability`, node)
  }
  for (const endpoint of definition.endpoints ?? []) {
    assert(endpoint.canvas && Number.isFinite(endpoint.canvas.x) && Number.isFinite(endpoint.canvas.y), `${exampleName} endpoint ${endpoint.handle} should include canvas coordinates`, endpoint)
    assert(expectation.nodes.includes(endpoint.entry_node), `${exampleName} endpoint ${endpoint.handle} should target a known node`, endpoint)
  }
  for (const edge of definition.edges ?? []) {
    assert(expectation.nodes.includes(edge.from_node), `${exampleName} edge ${edge.handle} should have a known source node`, edge)
    assert(expectation.nodes.includes(edge.to_node), `${exampleName} edge ${edge.handle} should have a known target node`, edge)
    if (edge.handoff_schema != null) {
      assert(expectation.schemas.includes(edge.handoff_schema), `${exampleName} edge ${edge.handle} should use a known schema`, edge)
    }
  }

  for (const waitNode of expectation.waitForAll ?? []) {
    const node = (definition.nodes ?? []).find((entry) => entry.handle === waitNode)
    assert(node?.wait_for_all_inputs === true, `${exampleName} node ${waitNode} should wait for all inputs`, node)
  }
  if (expectation.hasLoop) {
    assert(hasDirectedCycle(definition), `${exampleName} should include a directed cycle`, definition.edges)
  }
  if (expectation.multiEdgeRouter) {
    const outgoing = (definition.edges ?? []).filter((edge) => edge.from_node === expectation.multiEdgeRouter)
    assert(outgoing.length >= 2, `${exampleName} should model conditional routing as multi-edge agent handoff`, outgoing)
  }

  const providers = new Set((definition.nodes ?? []).map((node) => node.agent?.provider).filter(Boolean))
  assertSameSet(providers, expectation.providers, `${exampleName} provider mix`)

  const diagnostics = validationDiagnostics(validation)
  assert(!diagnostics.some((diagnostic) => diagnostic.code === 'canvas_overlap'), `${exampleName} should not have canvas overlap diagnostics`, diagnostics)
  return {
    alias: expectation.alias,
    providers: [...providers].sort(),
    completers: expectation.completers,
    waitForAll: expectation.waitForAll ?? [],
    hasLoop: Boolean(expectation.hasLoop),
    multiEdgeRouter: expectation.multiEdgeRouter ?? null,
  }
}

export function hasDirectedCycle(definition) {
  const adjacency = new Map()
  for (const node of definition.nodes ?? []) {
    adjacency.set(node.handle, [])
  }
  for (const edge of definition.edges ?? []) {
    const outgoing = adjacency.get(edge.from_node)
    if (outgoing) outgoing.push(edge.to_node)
  }
  const visiting = new Set()
  const visited = new Set()
  const visit = (node) => {
    if (visiting.has(node)) return true
    if (visited.has(node)) return false
    visiting.add(node)
    for (const next of adjacency.get(node) ?? []) {
      if (visit(next)) return true
    }
    visiting.delete(node)
    visited.add(node)
    return false
  }
  return [...adjacency.keys()].some(visit)
}

export function validateLiveExportedTopologyDefinition(exampleName, definition, validation) {
  const expectation = EXAMPLE_TOPOLOGY_EXPECTATIONS[exampleName]
  assert(expectation, `missing workflow-code topology expectation for ${exampleName}`)

  assert(definition?.workflow?.alias?.startsWith(expectation.alias), `${exampleName} live export alias should derive from original alias`, definition?.workflow)
  assert(definition.workflow?.max_concurrent === 32, `${exampleName} live export should preserve maxConcurrent`, definition.workflow)
  const schemaHandles = new Set((definition.schemas ?? []).map((schema) => schema.handle))
  const nodeHandles = new Set((definition.nodes ?? []).map((node) => node.handle))
  assert((definition.schemas ?? []).length === expectation.schemas.length, `${exampleName} live export schema count mismatch`, definition.schemas)
  assert((definition.nodes ?? []).length === expectation.nodes.length, `${exampleName} live export node count mismatch`, definition.nodes)
  assert((definition.edges ?? []).length === expectation.edges.length, `${exampleName} live export edge count mismatch`, definition.edges)
  assert((definition.endpoints ?? []).length === expectation.endpoints.length, `${exampleName} live export endpoint count mismatch`, definition.endpoints)
  assert(schemaHandles.has(definition.workflow?.run_output_schema), `${exampleName} live export run output schema should resolve to an exported schema`, definition.workflow)

  for (const schema of definition.schemas ?? []) {
    assert(schema.schema?.type === 'object', `${exampleName} live export schema ${schema.handle} should be an object schema`, schema)
  }
  for (const node of definition.nodes ?? []) {
    assert(node.instructions, `${exampleName} live export node ${node.handle} should preserve instructions`, node)
    assert(node.canvas && Number.isFinite(node.canvas.x) && Number.isFinite(node.canvas.y), `${exampleName} live export node ${node.handle} should include canvas coordinates`, node)
    assert(node.agent?.kind === 'create', `${exampleName} live export node ${node.handle} should create an agent for portability`, node)
    assert(node.agent?.provider === 'dev-stub', `${exampleName} live export node ${node.handle} should preserve provider rebinding`, node.agent)
    assert(node.agent?.model === 'workflow-code-topology-node', `${exampleName} live export node ${node.handle} should preserve model rebinding`, node.agent)
    if (node.intermediate_output_schema != null) {
      assert(schemaHandles.has(node.intermediate_output_schema), `${exampleName} live export node ${node.handle} intermediate schema should resolve`, node)
    }
  }
  for (const endpoint of definition.endpoints ?? []) {
    assert(endpoint.canvas && Number.isFinite(endpoint.canvas.x) && Number.isFinite(endpoint.canvas.y), `${exampleName} live export endpoint ${endpoint.handle} should include canvas coordinates`, endpoint)
    assert(nodeHandles.has(endpoint.entry_node), `${exampleName} live export endpoint ${endpoint.handle} should target an exported node`, endpoint)
  }
  for (const edge of definition.edges ?? []) {
    assert(nodeHandles.has(edge.from_node), `${exampleName} live export edge ${edge.handle} should have an exported source node`, edge)
    assert(nodeHandles.has(edge.to_node), `${exampleName} live export edge ${edge.handle} should have an exported target node`, edge)
    if (edge.handoff_schema != null) {
      assert(schemaHandles.has(edge.handoff_schema), `${exampleName} live export edge ${edge.handle} handoff schema should resolve`, edge)
    }
  }

  const completers = (definition.nodes ?? []).filter((node) => node.can_complete_workflow_run === true)
  assert(completers.length === expectation.completers.length, `${exampleName} live export completion node count mismatch`, completers)
  const waiters = (definition.nodes ?? []).filter((node) => node.wait_for_all_inputs === true)
  assert(waiters.length === (expectation.waitForAll ?? []).length, `${exampleName} live export wait-for-all node count mismatch`, waiters)
  if (expectation.hasLoop) {
    assert(hasDirectedCycle(definition), `${exampleName} live export should preserve a loop`, definition.edges)
  }
  if (expectation.multiEdgeRouter) {
    const outgoingCounts = new Map()
    for (const edge of definition.edges ?? []) {
      outgoingCounts.set(edge.from_node, (outgoingCounts.get(edge.from_node) ?? 0) + 1)
    }
    assert([...outgoingCounts.values()].some((count) => count >= 2), `${exampleName} live export should preserve a multi-edge router`, definition.edges)
  }

  const diagnostics = validationDiagnostics(validation)
  assert(!diagnostics.some((diagnostic) => diagnostic.code === 'canvas_overlap'), `${exampleName} live export should not have canvas overlap diagnostics`, diagnostics)
}

export function parseWorkflowOutputMessage(output) {
  const message = output?.message
  if (message == null) return null
  if (typeof message !== 'string') return message
  try {
    return JSON.parse(message)
  } catch {
    return message
  }
}

export function compactWorkflowRunSummary(run) {
  if (!run) return null
  return {
    id: run.id,
    status: run.status,
    active_node_run_id: run.active_node_run_id ?? null,
    messages: run.messages?.length ?? 0,
    intermediate_outputs: run.intermediate_outputs?.length ?? 0,
    final_output_valid: run.final_output_valid ?? null,
    node_runs: (run.node_runs ?? []).map((nodeRun) => ({
      id: nodeRun.id,
      node_id: nodeRun.node_id,
      agent_id: nodeRun.agent_id,
      status: nodeRun.status,
      iteration_index: nodeRun.iteration_index,
      failures: nodeRun.failures?.length ?? 0,
      completed: Boolean(nodeRun.completed_at_ms),
      summary: nodeRun.summary ?? nodeRun.completion?.summary ?? null,
      turn_envelope: nodeRun.turn_envelope ? {
        state: nodeRun.turn_envelope.state,
        dispatched: Boolean(nodeRun.turn_envelope.dispatched_at_ms),
        acknowledged: Boolean(nodeRun.turn_envelope.acknowledged_at_ms),
        validated_completed: Boolean(nodeRun.turn_envelope.validated_completed_at_ms),
        runtime_tool_calls: (nodeRun.turn_envelope.runtime_tool_calls ?? []).map((call) => ({
          tool_name: call.tool_name,
          ok: call.ok,
        })),
      } : null,
      recent_failure_events: (nodeRun.failure_events ?? []).slice(-3).map((event) => ({
        kind: event.kind,
        message: event.message,
      })),
    })),
  }
}

export const EXAMPLE_RUNTIME_EXPECTATIONS = {
  'adversarial-verification.js': {
    minMessages: 3,
    fields: { decision: 'accept' },
    note: 'proposal, critique, and judge completion',
  },
  'evaluator-optimizer.js': {
    minMessages: 3,
    fields: { accepted: true },
    note: 'optimizer revision loop',
  },
  'fan-out-synthesize.js': {
    minMessages: 4,
    fields: { source_count: 2 },
    note: 'two workers and synthesizer join',
  },
  'generate-filter.js': {
    minMessages: 2,
    fields: { selected_count: 1 },
    note: 'candidate generation and filter',
  },
  'loop-until-done.js': {
    minMessages: 3,
    fields: { iterations: 2 },
    note: 'one revise loop before completion',
  },
  'orchestrator-workers.js': {
    minMessages: 2,
    fields: { delegated: true },
    note: 'orchestrator assignment and synthesis',
  },
  'parallelization.js': {
    minMessages: 4,
    fields: { reviewer_count: 2 },
    note: 'two independent reviewers and aggregation',
  },
  'planner-worker-reviewer.js': {
    minMessages: 6,
    fields: { completed: true },
    note: 'planner assignment, one worker-reviewer revision, and planner completion',
  },
  'prompt-chaining.js': {
    minMessages: 1,
    fields: { answer: 'refined draft accepted' },
    note: 'draft handoff to refiner',
  },
  'routing.js': {
    minMessages: 2,
    maxMessages: 2,
    fields: { specialist: 1 },
    note: 'router chooses exactly one specialist edge',
  },
  'tournament.js': {
    minMessages: 4,
    fields: { winner: 'a' },
    note: 'two contestants and judge',
  },
}

export function validateExampleRuntimeResult(exampleName, run) {
  assert(run?.status === 'Completed', `example ${exampleName} runtime should complete`, run)
  assert(run.final_output_valid === true, `example ${exampleName} final output should be schema-valid`, run)
  const finalOutput = parseWorkflowOutputMessage(run.final_output)
  assert(finalOutput && typeof finalOutput === 'object', `example ${exampleName} should produce structured final output`, run.final_output)
  const completionHandoffCount = (run.node_runs ?? []).reduce((count, nodeRun) => {
    const output = parseWorkflowOutputMessage(nodeRun.completion?.output)
    return count + (Array.isArray(output?.workflow_handoffs) ? output.workflow_handoffs.length : 0)
  }, 0)
  const messageCount = (run.messages?.length ?? 0) || completionHandoffCount
  const nodeRunCount = run.node_runs?.length ?? 0
  const expectations = EXAMPLE_RUNTIME_EXPECTATIONS[exampleName]
  assert(expectations, `missing runtime expectation for ${exampleName}`)
  assert(messageCount >= expectations.minMessages, `example ${exampleName} runtime should emit enough handoffs for ${expectations.note}`, { messageCount, run })
  if (expectations.maxMessages != null) {
    assert(messageCount <= expectations.maxMessages, `example ${exampleName} runtime should not emit extra routed handoffs`, { messageCount, run })
  }
  for (const [field, value] of Object.entries(expectations.fields)) {
    assert(finalOutput[field] === value, `example ${exampleName} final output field ${field} mismatch`, { finalOutput, expected: value })
  }
  return {
    status: run.status,
    messages: messageCount,
    nodeRuns: nodeRunCount,
    finalOutput,
  }
}

export async function validateTopologyTuiOutlineProjection(client, sessionId, exampleName, apply, completedRun) {
  const stateResponse = await client.send(getSessionStateRequest(sessionId))
  const session = unwrap(stateResponse, 'SessionStateLoaded')?.session
    ?? unwrap(stateResponse, 'SessionState')?.session
  const workflow = (session?.workflows ?? []).find((entry) => entry.id === apply.workflow_id)
  assert(workflow, `example ${exampleName} TUI outline should find workflow in session`, { workflowId: apply.workflow_id })
  const entryNodeId = completedRun.entry_node_id ?? workflow.endpoints?.[0]?.entry_node_id ?? workflow.nodes?.[0]?.id
  const outline = buildWorkflowOutline({
    workflow,
    agents: session?.agents ?? [],
    workflowRuns: session?.workflow_runs ?? [],
    selectedNodeId: entryNodeId ?? null,
  })
  const rendered = renderWorkflowOutlineToText(outline)
  assert(rendered.includes(`workflow: ${workflow.id}`), `example ${exampleName} TUI outline should include workflow id`, rendered)
  assert(rendered.includes(`status ${String(completedRun.status).toLowerCase()}`), `example ${exampleName} TUI outline should include run status`, rendered)
  const finalMessage = completedRun.final_output?.message
  const finalOutputVisible = Boolean(finalMessage && renderedIncludesWorkflowFinalOutput(rendered, finalMessage))
  assert(finalOutputVisible, `example ${exampleName} TUI outline should include final output`, {
    rendered,
    finalMessage,
  })
  for (const node of workflow.nodes ?? []) {
    assert(rendered.includes(`node ${node.id}`), `example ${exampleName} TUI outline should include node ${node.id}`, rendered)
  }
  for (const edge of workflow.edges ?? []) {
    assert(rendered.includes(edge.id), `example ${exampleName} TUI outline should include edge ${edge.id}`, rendered)
    assert(rendered.includes(edge.from_node_id), `example ${exampleName} TUI outline should include edge source ${edge.from_node_id}`, rendered)
    assert(rendered.includes(edge.to_node_id), `example ${exampleName} TUI outline should include edge target ${edge.to_node_id}`, rendered)
  }
  for (const endpoint of workflow.endpoints ?? []) {
    assert(rendered.includes(endpoint.id), `example ${exampleName} TUI outline should include endpoint ${endpoint.id}`, rendered)
  }
  return {
    workflowId: workflow.id,
    workflowRunId: completedRun.id,
    statusVisible: rendered.includes(`status ${String(completedRun.status).toLowerCase()}`),
    finalOutputVisible,
    nodes: workflow.nodes?.length ?? 0,
    edges: workflow.edges?.length ?? 0,
    endpoints: workflow.endpoints?.length ?? 0,
  }
}

export function renderedIncludesWorkflowFinalOutput(rendered, message) {
  const singleLine = String(message).replace(/\s+/g, ' ').trim()
  if (singleLine.length <= 180) {
    return rendered.includes(`final output: ${singleLine}`)
  }
  return rendered.includes(`final output: ${singleLine.slice(0, 177)}...`)
}

export function validateApplyResult(result, label, expected = defaultToyExpectation()) {
  assert(result?.compile?.validation?.ok, `${label} compile validation failed`, result?.compile?.validation)
  const apply = result.apply
  assert(apply?.workflow_id, `${label} did not return workflow id`, result)
  assert(Object.keys(apply.node_ids ?? {}).length === expected.nodes, `${label} should create ${expected.nodes} nodes`, apply)
  assert(Object.keys(apply.agent_ids ?? {}).length === expected.agents, `${label} should resolve ${expected.agents} node agents`, apply)
  assert(Object.keys(apply.edge_ids ?? {}).length === expected.edges, `${label} should create ${expected.edges} edges`, apply)
  assert(Object.keys(apply.endpoint_ids ?? {}).length === expected.endpoints, `${label} should create ${expected.endpoints} endpoints`, apply)
  assert(Object.keys(apply.queue_ids ?? {}).length === (expected.queues ?? 1), `${label} should create ${expected.queues ?? 1} queues`, apply)
  assert(Object.keys(apply.schedule_ids ?? apply.watchdog_ids ?? {}).length === (expected.schedules ?? 0), `${label} should create ${expected.schedules ?? 0} schedules`, apply)
  for (const schemaHandle of expected.requiredSchemas) {
    assert(apply.schema_refs?.[schemaHandle], `${label} should report schema ${schemaHandle}`, apply)
  }
  assert(apply.canvas_layout_applied === true, `${label} should create a canvas layout`, apply)
  return apply
}

export function validateSessionProjection(session, apply, label, expected = defaultToyExpectation()) {
  const workflow = (session.workflows ?? []).find((entry) => entry.id === apply.workflow_id)
  assert(workflow, `${label} workflow should appear in session projection`, { workflowId: apply.workflow_id })
  assert(workflow.canvas_layout, `${label} workflow should include canvas layout`, workflow)
  if (expected.requiredSchemas.includes('final_output')) {
    assert(workflow.run_output_schema_ref === apply.schema_refs.final_output, `${label} final schema ref should be assigned`, {
      workflow,
      schemaRefs: apply.schema_refs,
    })
  }
  for (const [handle, queueId] of Object.entries(apply.queue_ids ?? {})) {
    const queue = (session.workflow_prompt_queues ?? []).find((entry) => entry.id === queueId)
    assert(queue, `${label} prompt queue ${handle} should appear in session`, { queueId })
    assert(queue.workflow_id === apply.workflow_id, `${label} prompt queue ${handle} should belong to workflow`, { queue, apply })
    if (handle === 'urgent') {
      assert(queue.alias === 'urgent', `${label} urgent queue alias should be projected`, queue)
      assert(queue.priority === 10, `${label} urgent queue priority should be projected`, queue)
      assert(queue.enabled === false, `${label} urgent queue enabled state should be projected`, queue)
    }
  }
  for (const [handle, scheduleId] of Object.entries(apply.schedule_ids ?? apply.watchdog_ids ?? {})) {
    const schedule = (session.workflow_schedules ?? session.workflow_watchdogs ?? []).find((entry) => entry.id === scheduleId)
    assert(schedule, `${label} schedule ${handle} should appear in session`, { scheduleId })
    assert(schedule.workflow_id === apply.workflow_id, `${label} schedule ${handle} should belong to workflow`, { schedule, apply })
    if (handle === 'entry_schedule') {
      assert(schedule.queue_id === apply.queue_ids?.urgent, `${label} schedule should target the scripted urgent queue`, { schedule, apply })
      assert(schedule.trigger?.kind === 'interval' && schedule.trigger.every_seconds === 300, `${label} schedule interval should be projected`, schedule)
      assert(schedule.invocation_prompt === 'Wake the workflow-code artifact drill.', `${label} schedule prompt should be projected`, schedule)
    }
  }
  for (const [handle, agentId] of Object.entries(apply.agent_ids ?? {})) {
    const agent = (session.agents ?? []).find((entry) => entry.id === agentId)
    assert(agent, `${label} node agent ${handle} should appear in session`, { agentId })
    const expectedProvider = expected.providerByNode?.[handle] ?? 'dev-stub'
    const expectedModel = expected.modelByNode?.[handle] ?? expected.agentModel ?? 'default'
    assert(agent.provider === expectedProvider, `${label} node agent ${handle} should use ${expectedProvider}`, agent)
    assert(agent.model === expectedModel, `${label} node agent ${handle} should use ${expectedModel} model`, agent)
    for (const extension of expected.nodeExtensions?.[handle] ?? []) {
      assert(
        (agent.extension_grants ?? []).some((grant) => (
          grant.kind === extension.kind
          && grant.name === extension.name
          && (extension.environment === undefined || grant.environment === extension.environment)
          && (extension.credential === undefined || grant.credential === extension.credential)
          && (extension.max_safety === undefined || grant.max_safety === extension.max_safety)
        )),
        `${label} node agent ${handle} should receive extension ${extension.kind}:${extension.name}`,
        { agent, expectedExtension: extension },
      )
    }
  }
}

export async function writeWorkflowCodeArtifactSkillSource(skillSourceRoot, skillName) {
  const skillDir = path.join(skillSourceRoot, skillName)
  await mkdir(skillDir, { recursive: true })
  await writeFile(
    path.join(skillDir, 'SKILL.md'),
    [
      '---',
      `name: ${skillName}`,
      'description: Skill used by the workflow-code artifact drill.',
      '---',
      'Use this skill only for workflow-code artifact drill validation.',
      '',
    ].join('\n'),
    'utf8',
  )
  return skillDir
}

export async function workflowCodeExamples() {
  const examplesDir = path.join(repoRoot, 'examples', 'workflow-code')
  const names = (await readdir(examplesDir))
    .filter((name) => name.endsWith('.js'))
    .sort()
  return await Promise.all(names.map(async (name) => ({
    name,
    source: await readFile(path.join(examplesDir, name), 'utf8'),
  })))
}

export async function completeAppliedTopologyWorkflow(client, sessionId, exampleName, definition, apply, timeoutMs) {
  const endpoint = (definition.endpoints ?? []).find((entry) => entry.handle === 'entry' || entry.alias === 'entry')
    ?? definition.endpoints?.[0]
  const entryNodeHandle = endpoint?.entry_node
  assert(entryNodeHandle, `example ${exampleName} runtime should resolve entry node`, { definition, apply })
  const runtimeAgentEntries = Object.entries(apply.agent_ids ?? {})
    .sort(([left], [right]) => {
      if (left === entryNodeHandle) return 1
      if (right === entryNodeHandle) return -1
      return left.localeCompare(right)
    })
  for (const [handle, agentId] of runtimeAgentEntries) {
    const launchResponse = unwrap(
      await client.send(launchProviderRunRequest(
        sessionId,
        'dev-stub',
        'default',
        'workflow-code-topology-node',
        'low',
        agentId,
      )),
      'ProviderRunLaunchAccepted',
    )
    assert(launchResponse?.provider_run?.id, `example ${exampleName} runtime should launch provider for node ${handle}`, launchResponse)
    await waitForProviderRunReady(client, launchResponse.provider_run.id, timeoutMs)
  }
  const endpointId = apply.endpoint_ids?.[endpoint.handle]
  assert(endpointId, `example ${exampleName} runtime should resolve entry endpoint`, { endpoint, apply })
  const entryAgentId = apply.agent_ids?.[entryNodeHandle]
  assert(entryAgentId, `example ${exampleName} runtime should resolve entry agent`, { entryNodeHandle, apply })
  await client.send(focusAgentRequest(sessionId, entryAgentId))
  const invokeResponse = unwrap(
    await client.send(invokeWorkflowEndpointRequest(
      sessionId,
      apply.workflow_id,
      endpointId,
      `Run ${exampleName} topology runtime validation.`,
    )),
    'WorkflowRunInvoked',
  )
  const runtimeRun = invokeResponse?.workflow_run
  assert(runtimeRun?.id, `example ${exampleName} runtime should start a workflow run`, invokeResponse)
  const completedRun = await waitForCompletedWorkflowRun(
    client,
    sessionId,
    runtimeRun.id,
    timeoutMs,
    `example ${exampleName}`,
  )
  const tuiOutline = await validateTopologyTuiOutlineProjection(
    client,
    sessionId,
    exampleName,
    apply,
    completedRun,
  )
  return {
    workflowRunId: completedRun.id,
    runtime: validateExampleRuntimeResult(exampleName, completedRun),
    tuiOutline,
  }
}

export function validateRealProviderTopologyRuntimeResult(exampleName, run) {
  assert(run?.status === 'Completed', `real-provider example ${exampleName} runtime should complete`, run)
  assert(run.final_output_valid === true, `real-provider example ${exampleName} final output should be schema-valid`, run)
  const finalOutput = parseWorkflowOutputMessage(run.final_output)
  assert(
    finalOutput && typeof finalOutput === 'object' && !Array.isArray(finalOutput),
    `real-provider example ${exampleName} should produce structured final output`,
    run.final_output,
  )
  assert(
    (run.node_runs?.length ?? 0) >= 3,
    `real-provider example ${exampleName} should execute at least three workflow nodes`,
    run,
  )
  return {
    status: run.status,
    nodeRuns: run.node_runs?.length ?? 0,
    finalOutput,
  }
}

export async function completeAppliedTopologyWorkflowWithRealProviders(client, sessionId, exampleName, definition, apply, rebindings, timeoutMs) {
  const endpoint = (definition.endpoints ?? []).find((entry) => entry.handle === 'entry' || entry.alias === 'entry')
    ?? definition.endpoints?.[0]
  const entryNodeHandle = endpoint?.entry_node
  assert(entryNodeHandle, `real-provider example ${exampleName} runtime should resolve entry node`, { definition, apply })
  const rebindingsByNode = rebindingByNode(rebindings)
  const runtimeAgentEntries = Object.entries(apply.agent_ids ?? {})
    .sort(([left], [right]) => {
      if (left === entryNodeHandle) return 1
      if (right === entryNodeHandle) return -1
      return left.localeCompare(right)
    })
  const prelaunchedProviders = new Set()
  const prelaunchedProviderFamilies = new Set()
  const configuredProviderFamilies = new Set((rebindings ?? []).map((entry) => providerFamily(entry.provider)))
  for (const [handle, agentId] of runtimeAgentEntries) {
    const rebinding = rebindingsByNode.get(handle)
    assert(rebinding, `real-provider example ${exampleName} missing rebinding for node ${handle}`, { handle, rebindings })
    assert(rebinding.provider !== 'dev-stub', `real-provider example ${exampleName} must not launch dev-stub for node ${handle}`, rebinding)
    if (!shouldPrelaunchRealProvider(rebinding.provider)) {
      stage(`real-provider ${exampleName}: deferring node ${handle} launch to workflow dispatch`, {
        provider: rebinding.provider,
        model: rebinding.model,
      })
      continue
    }
    stage(`real-provider ${exampleName}: launching node ${handle}`, {
      provider: rebinding.provider,
      model: rebinding.model,
      account_profile: rebinding.account_profile ?? 'default',
      effort: rebinding.effort ?? '',
    })
    const launchResponse = unwrap(
      await client.send(launchProviderRunRequest(
        sessionId,
        rebinding.provider,
        rebinding.account_profile ?? 'default',
        rebinding.model,
        rebinding.effort ?? 'low',
        agentId,
      )),
      'ProviderRunLaunchAccepted',
    )
    assert(launchResponse?.provider_run?.id, `real-provider example ${exampleName} should launch provider for node ${handle}`, launchResponse)
    try {
      await waitForProviderRunReady(client, launchResponse.provider_run.id, timeoutMs)
    } catch (error) {
      throw new Error([
        `real-provider example ${exampleName} provider launch failed for node ${handle}`,
        `provider=${rebinding.provider}`,
        `model=${rebinding.model}`,
        `account_profile=${rebinding.account_profile ?? 'default'}`,
        `effort=${rebinding.effort ?? 'low'}`,
        `provider_run_id=${launchResponse.provider_run.id}`,
        error?.message ?? String(error),
      ].join('\n'))
    }
    stage(`real-provider ${exampleName}: provider ready for node ${handle}`, {
      provider: rebinding.provider,
      provider_run_id: launchResponse.provider_run.id,
    })
    prelaunchedProviders.add(rebinding.provider)
    prelaunchedProviderFamilies.add(providerFamily(rebinding.provider))
  }
  assertSameSet(configuredProviderFamilies, ['claude', 'codex', 'opencode'], `real-provider example ${exampleName} configured provider families`)

  const endpointId = apply.endpoint_ids?.[endpoint.handle]
  assert(endpointId, `real-provider example ${exampleName} runtime should resolve entry endpoint`, { endpoint, apply })
  const entryAgentId = apply.agent_ids?.[entryNodeHandle]
  assert(entryAgentId, `real-provider example ${exampleName} runtime should resolve entry agent`, { entryNodeHandle, apply })
  await client.send(focusAgentRequest(sessionId, entryAgentId))
  stage(`real-provider ${exampleName}: invoking workflow endpoint`, {
    workflow_id: apply.workflow_id,
    endpoint_id: endpointId,
    entry_node: entryNodeHandle,
  })
  const invokeResponse = unwrap(
    await client.send(invokeWorkflowEndpointRequest(
      sessionId,
      apply.workflow_id,
      endpointId,
      [
        `Run ${exampleName} as a short release-validation workflow.`,
        'Use concise handoffs.',
        'When you complete the workflow, submit final output that strictly matches the workflow final-output schema.',
      ].join(' '),
    )),
    'WorkflowRunInvoked',
  )
  const runtimeRun = invokeResponse?.workflow_run
  assert(runtimeRun?.id, `real-provider example ${exampleName} runtime should start a workflow run`, invokeResponse)
  stage(`real-provider ${exampleName}: workflow run started`, { workflow_run_id: runtimeRun.id })
  const completedRun = await waitForCompletedWorkflowRun(
    client,
    sessionId,
    runtimeRun.id,
    timeoutMs,
    `real-provider ${exampleName}`,
  )
  const runtime = validateRealProviderTopologyRuntimeResult(exampleName, completedRun)
  const tuiOutline = await validateTopologyTuiOutlineProjection(
    client,
    sessionId,
    exampleName,
    apply,
    completedRun,
  )
  return {
    workflowRunId: completedRun.id,
    runtime,
    tuiOutline,
    configuredProviderFamilies: [...configuredProviderFamilies].sort(),
    prelaunchedProviders: [...prelaunchedProviders].sort(),
    prelaunchedProviderFamilies: [...prelaunchedProviderFamilies].sort(),
  }
}

export async function applyRealProviderTopology(client, sessionId, nodePath, exampleName, source, options) {
  const validated = unwrap(
    await client.send(validateWorkflowCodeRequest(sessionId, nodePath, source)),
    'WorkflowCodeValidated',
  ).result
  assert(validated?.validation?.ok, `real-provider example ${exampleName} should validate`, validated?.validation)
  const topology = validateExampleTopologyDefinition(exampleName, validated.definition, validated.validation)
  const providers = providerSetForDefinition(validated.definition)
  assertSameSet(providers, ['claude', 'codex', 'opencode'], `real-provider example ${exampleName} provider mix`)
  assert((validated.definition.nodes?.length ?? 0) >= 3, `real-provider example ${exampleName} must contain at least three nodes`, validated.definition)

  const rebindings = realProviderRebindingsForDefinition(validated.definition, options)
  const providerByNode = Object.fromEntries(rebindings.map((entry) => [entry.node, entry.provider]))
  const modelByNode = Object.fromEntries(rebindings.map((entry) => [entry.node, entry.model]))
  const appliedResponse = unwrap(
    await client.send(applyWorkflowCodeRequest(sessionId, nodePath, source, rebindings)),
    'WorkflowCodeApplied',
  )
  const expected = expectationFromDefinition(validated.definition)
  const apply = validateApplyResult(appliedResponse.result, `real-provider example ${exampleName}`, expected)
  validateSessionProjection(appliedResponse.session, apply, `real-provider example ${exampleName}`, {
    ...expected,
    providerByNode,
    modelByNode,
  })
  const completed = await completeAppliedTopologyWorkflowWithRealProviders(
    client,
    sessionId,
    exampleName,
    validated.definition,
    apply,
    rebindings,
    options.timeoutMs,
  )
  return {
    example: exampleName,
    topology,
    workflowId: apply.workflow_id,
    workflowRunId: completed.workflowRunId,
    rebindings,
    runtime: completed.runtime,
    tuiOutline: completed.tuiOutline,
    configuredProviderFamilies: completed.configuredProviderFamilies,
    prelaunchedProviders: completed.prelaunchedProviders,
    prelaunchedProviderFamilies: completed.prelaunchedProviderFamilies,
  }
}

export async function runRealProviderTopologyDrill(client, sessionId, nodePath, options) {
  const examples = await workflowCodeExamples()
  const requested = options.realProviderTopology
  const exampleName = requested.endsWith('.js') ? requested : `${requested}.js`
  const example = examples.find((entry) => entry.name === exampleName)
  assert(example, `real-provider topology example not found: ${requested}`, {
    requested,
    available: examples.map((entry) => entry.name),
  })
  return await applyRealProviderTopology(client, sessionId, nodePath, example.name, example.source, options)
}

export async function applyExampleSuite(client, sessionId, nodePath, workspace, timeoutMs) {
  const examples = await workflowCodeExamples()
  const results = []
  const liveExports = []
  for (const [index, example] of examples.entries()) {
    const slug = example.name.replace(/\.js$/, '')
    const artifactName = `pattern-${index + 1}-${slug}-${Date.now()}`
    const created = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, artifactName, nodePath, example.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(created?.metadata?.validation?.ok, `example ${example.name} should validate`, created?.metadata?.validation)

    const exported = unwrap(
      await client.send(exportWorkflowCodeArtifactRequest(sessionId, artifactName)),
      'WorkflowCodeArtifactExported',
    ).package
    assert(exported?.source_sha256, `example ${example.name} package export should include source hash`, exported)
    assert(exported?.definition_sha256, `example ${example.name} package export should include definition hash`, exported)
    const topology = validateExampleTopologyDefinition(
      example.name,
      exported.definition,
      created.metadata.validation,
    )

    const importedArtifactName = `${artifactName}-package-imported`
    const imported = unwrap(
      await client.send(importWorkflowCodeArtifactRequest(sessionId, exported, nodePath, {
        name: importedArtifactName,
        overwrite: false,
      })),
      'WorkflowCodeArtifactImported',
    ).artifact
    assert(imported?.metadata?.validation?.ok, `example ${example.name} package import should validate`, imported?.metadata?.validation)

    const expected = expectationFromDefinition(exported.definition)
    const rebindings = rebindingsForDefinition(exported.definition)
    const appliedResponse = unwrap(
      await client.send(applyWorkflowCodeArtifactRequest(sessionId, importedArtifactName, rebindings)),
      'WorkflowCodeApplied',
    )
    const apply = validateApplyResult(appliedResponse.result, `example ${example.name}`, expected)
    validateSessionProjection(appliedResponse.session, apply, `example ${example.name}`, expected)

    const inlineSource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        sessionId,
        { kind: 'artifact', name: artifactName },
        'inline',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(inlineSource?.source, `example ${example.name} inline source export should include source`, inlineSource)
    assert(
      sha256Hex(inlineSource.source) === inlineSource.source_sha256,
      `example ${example.name} inline source hash should match contents`,
      inlineSource,
    )
    const inlineRoundTripName = `${artifactName}-inline-source`
    const inlineRoundTrip = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, inlineRoundTripName, nodePath, inlineSource.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(inlineRoundTrip?.metadata?.validation?.ok, `example ${example.name} inline source should recompile`, inlineRoundTrip?.metadata?.validation)
    validateExampleTopologyDefinition(
      example.name,
      inlineRoundTrip.definition,
      inlineRoundTrip.metadata.validation,
    )

    const directorySource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        sessionId,
        { kind: 'artifact', name: artifactName },
        'directory',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(directorySource?.source_path === 'workflow.js', `example ${example.name} source directory should export workflow.js`, directorySource)
    const directoryManifest = await writeSourceDirectoryExport(workspace, directorySource, `example ${example.name}`)
    assert(directoryManifest?.schema_paths, `example ${example.name} source directory manifest should include schema_paths`, directoryManifest)
    const directoryRoundTripName = `${artifactName}-directory-source`
    const directoryRoundTrip = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, directoryRoundTripName, nodePath, directorySource.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(directoryRoundTrip?.metadata?.validation?.ok, `example ${example.name} source directory should recompile`, directoryRoundTrip?.metadata?.validation)
    validateExampleTopologyDefinition(
      example.name,
      directoryRoundTrip.definition,
      directoryRoundTrip.metadata.validation,
    )

    const runtimeRebindings = topologyRuntimeRebindingsForDefinition(exported.definition)
    const runtimeExpected = topologyRuntimeExpectation(exported.definition)
    const runtimeAppliedResponse = unwrap(
      await client.send(applyWorkflowCodeArtifactRequest(sessionId, importedArtifactName, runtimeRebindings)),
      'WorkflowCodeApplied',
    )
    const runtimeApply = validateApplyResult(runtimeAppliedResponse.result, `example ${example.name} runtime`, runtimeExpected)
    validateSessionProjection(runtimeAppliedResponse.session, runtimeApply, `example ${example.name} runtime`, runtimeExpected)

    const liveInlineSource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        sessionId,
        { kind: 'workflow', workflow_ref: runtimeApply.workflow_id },
        'inline',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(liveInlineSource?.source, `example ${example.name} live inline source export should include source`, liveInlineSource)
    assert(
      sha256Hex(liveInlineSource.source) === liveInlineSource.source_sha256,
      `example ${example.name} live inline source hash should match contents`,
      liveInlineSource,
    )
    const liveInlineRoundTripName = `${artifactName}-live-inline-source`
    const liveInlineRoundTrip = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, liveInlineRoundTripName, nodePath, liveInlineSource.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(liveInlineRoundTrip?.metadata?.validation?.ok, `example ${example.name} live inline source should recompile`, liveInlineRoundTrip?.metadata?.validation)
    validateLiveExportedTopologyDefinition(
      example.name,
      liveInlineRoundTrip.definition,
      liveInlineRoundTrip.metadata.validation,
    )

    const liveDirectorySource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        sessionId,
        { kind: 'workflow', workflow_ref: runtimeApply.workflow_id },
        'directory',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(liveDirectorySource?.source_path === 'workflow.js', `example ${example.name} live source directory should export workflow.js`, liveDirectorySource)
    const liveDirectoryManifest = await writeSourceDirectoryExport(workspace, liveDirectorySource, `example ${example.name} live source directory`)
    assert(liveDirectoryManifest?.schema_paths, `example ${example.name} live source directory manifest should include schema_paths`, liveDirectoryManifest)
    const liveDirectoryRoundTripName = `${artifactName}-live-directory-source`
    const liveDirectoryRoundTrip = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(sessionId, liveDirectoryRoundTripName, nodePath, liveDirectorySource.source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(liveDirectoryRoundTrip?.metadata?.validation?.ok, `example ${example.name} live source directory should recompile`, liveDirectoryRoundTrip?.metadata?.validation)
    validateLiveExportedTopologyDefinition(
      example.name,
      liveDirectoryRoundTrip.definition,
      liveDirectoryRoundTrip.metadata.validation,
    )

    const completed = await completeAppliedTopologyWorkflow(
      client,
      sessionId,
      example.name,
      exported.definition,
      runtimeApply,
      timeoutMs,
    )

    liveExports.push({
      exampleName: example.name,
      sourceWorkflowId: runtimeApply.workflow_id,
      inlineSource: liveInlineSource,
      directorySource: liveDirectorySource,
    })

    results.push({
      example: example.name,
      artifactName,
      topology,
      runtime: completed.runtime,
      tuiOutline: completed.tuiOutline,
      packageImportedArtifactName: importedArtifactName,
      workflowId: apply.workflow_id,
      runtimeWorkflowId: runtimeApply.workflow_id,
      runtimeWorkflowRunId: completed.workflowRunId,
      nodes: expected.nodes,
      edges: expected.edges,
      endpoints: expected.endpoints,
      schemas: Object.keys(apply.schema_refs ?? {}),
      packageSha256: exported.source_sha256,
      inlineSourceSha256: inlineSource.source_sha256,
      directorySourceSha256: directorySource.source_sha256,
      liveInlineSourceSha256: liveInlineSource.source_sha256,
      liveDirectorySourceSha256: liveDirectorySource.source_sha256,
      directoryFiles: (directorySource.files ?? []).map((file) => file.path).sort(),
      liveDirectoryFiles: (liveDirectorySource.files ?? []).map((file) => file.path).sort(),
    })
  }
  return { results, liveExports }
}
