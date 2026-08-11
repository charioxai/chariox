import path from 'node:path'
import { writeFile } from 'node:fs/promises'
const { addWorkflowEdgeRequest, addWorkflowNodeRequest, attachToSessionRequest, createSessionRequest, createWorkflowEndpointRequest, createWorkflowPublicationRequest, createWorkflowRequest, createWorkflowScheduleRequest, getProviderRunRequest, launchProviderRunRequest, setWorkflowNodeCanCompleteRunRequest, setWorkflowNodeCanEmitIntermediateOutputRequest, setWorkflowRunOutputSchemaRequest, spawnAgentRequest, updateWorkflowNodeInstructionsRequest } = await import('../../../../packages/kernel-client/dist/ipc-requests.js')
import { variant } from './live-workflow-publication-drill-runtime.mjs'
import { waitForProviderRunReady } from './live-workflow-publication-drill-waiters.mjs'

export const HUMAN_HTTP_COMPOSER_METHODS = Object.freeze(['GET', 'POST'])

export function publicationRequestTransportOptions(options) {
  const transportKind = options.transportKind
  const supportsHttpMethods = transportKind !== 'websocket_json'
  const supportsParser = transportKind !== 'websocket_json' && transportKind !== 'mcp'
  return {
    route: options.route,
    ...(supportsHttpMethods && options.methods ? { methods: options.methods } : {}),
    transport: { kind: transportKind },
    ...(supportsParser ? { parser: { kind: 'json' } } : {}),
    mode: transportKind === 'mcp' ? 'sync' : 'async',
  }
}

export async function createSchedulePublicationArtifacts(client, {
  sessionId,
  workflowId,
  endpointId,
}) {
  const schedule = variant(
    await client.send(createWorkflowScheduleRequest(
      sessionId,
      workflowId,
      endpointId,
      { kind: 'interval', every_seconds: 60 },
      'schedule-publication',
      'queue',
      1,
    )),
    'WorkflowScheduleCreated',
  ).schedule
  const publication = variant(
    await client.send(createWorkflowPublicationRequest(sessionId, workflowId, endpointId, {
      alias: 'public_schedule',
      route: '/schedule',
      methods: ['POST'],
      transport: { kind: 'api_sse_json' },
      parser: { kind: 'json' },
      mode: 'async',
    })),
    'WorkflowPublicationCreated',
  ).publication
  return { schedule, publication }
}

export async function createDeterministicPublicationSession(client, sessionIds, options) {
  const model = options.model ?? 'workflow-intermediate-node'
  const session = variant(
    await client.send(createSessionRequest(options.workspace, options.workspace, options.sessionAlias)),
    'SessionCreated',
  ).session
  sessionIds.push(session.id)
  await client.send(attachToSessionRequest(session.id, `${options.attachAlias}-${process.pid}`))
  const agent = variant(
    await client.send(spawnAgentRequest(session.id, 'dev-stub', options.agentAlias, model, options.workspace, 'low')),
    'AgentSpawned',
  ).agent
  const providerRun = variant(
    await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', model, 'low', agent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, providerRun.id)
  const workflow = variant(await client.send(createWorkflowRequest(session.id, options.workflowAlias)), 'WorkflowCreated').workflow
  const node = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, agent.id)), 'WorkflowNodeAdded').node
  await client.send(updateWorkflowNodeInstructionsRequest(
    session.id,
    workflow.id,
    node.id,
    'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
  ))
  await client.send(setWorkflowNodeCanCompleteRunRequest(session.id, workflow.id, node.id, true))
  if (model === 'workflow-intermediate-node') {
    await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(session.id, workflow.id, node.id, true))
  }
  const endpoint = variant(
    await client.send(createWorkflowEndpointRequest(session.id, workflow.id, node.id, options.endpointAlias)),
    'WorkflowEndpointCreated',
  ).endpoint
  const publication = variant(
    await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: options.publicationAlias,
      ...publicationRequestTransportOptions(options),
      traceExposure: options.traceExposure === 'all'
        ? { nodes: { [node.id]: ['output_summary', 'assistant_messages', 'thinking', 'tool_use'] } }
        : options.traceExposure ?? null,
    })),
    'WorkflowPublicationCreated',
  ).publication
  return { session, workflow, node, agent, providerRun, endpoint, publication }
}

export async function createDashboardPublicationSession(client, sessionIds, options) {
  const session = variant(
    await client.send(createSessionRequest(options.workspace, options.workspace, options.sessionAlias)),
    'SessionCreated',
  ).session
  sessionIds.push(session.id)
  await client.send(attachToSessionRequest(session.id, `${options.attachAlias}-${process.pid}`))

  const producerAgent = variant(
    await client.send(spawnAgentRequest(session.id, 'dev-stub', `${options.agentAlias}-producer`, 'workflow-intermediate-node', options.workspace, 'low')),
    'AgentSpawned',
  ).agent
  const producerRun = variant(
    await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'workflow-intermediate-node', 'low', producerAgent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, producerRun.id)

  const dashboardAgent = variant(
    await client.send(spawnAgentRequest(session.id, 'dev-stub', options.agentAlias, 'workflow-html-final-node', options.workspace, 'low')),
    'AgentSpawned',
  ).agent
  const dashboardRun = variant(
    await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'workflow-html-final-node', 'low', dashboardAgent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, dashboardRun.id)

  const workflow = variant(await client.send(createWorkflowRequest(session.id, options.workflowAlias)), 'WorkflowCreated').workflow
  const producerNode = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, producerAgent.id)), 'WorkflowNodeAdded').node
  const dashboardNode = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, dashboardAgent.id)), 'WorkflowNodeAdded').node
  await client.send(updateWorkflowNodeInstructionsRequest(
    session.id,
    workflow.id,
    producerNode.id,
    'Emit the deterministic intermediate output and then hand off to the dashboard renderer.',
  ))
  await client.send(updateWorkflowNodeInstructionsRequest(
    session.id,
    workflow.id,
    dashboardNode.id,
    'Render the deterministic vibrant dashboard HTML final output.',
  ))
  await client.send(addWorkflowEdgeRequest(session.id, workflow.id, producerNode.id, dashboardNode.id))
  await client.send(setWorkflowNodeCanCompleteRunRequest(session.id, workflow.id, producerNode.id, false))
  await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(session.id, workflow.id, producerNode.id, true))
  await client.send(setWorkflowNodeCanCompleteRunRequest(session.id, workflow.id, dashboardNode.id, true))

  const endpoint = variant(
    await client.send(createWorkflowEndpointRequest(session.id, workflow.id, producerNode.id, options.endpointAlias)),
    'WorkflowEndpointCreated',
  ).endpoint
  const publication = variant(
    await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: options.publicationAlias,
      route: options.route,
      methods: options.methods,
      transport: { kind: options.transportKind },
      parser: { kind: 'json' },
      traceExposure: {
        nodes: {
          [producerNode.id]: ['output_summary', 'assistant_messages', 'thinking', 'tool_use'],
          [dashboardNode.id]: ['output_summary', 'assistant_messages', 'thinking', 'tool_use'],
        },
      },
      mode: 'async',
    })),
    'WorkflowPublicationCreated',
  ).publication
  return {
    session,
    workflow,
    producerNode,
    dashboardNode,
    producerAgent,
    dashboardAgent,
    producerRun,
    dashboardRun,
    endpoint,
    publication,
  }
}

export async function createRealProviderDashboardPublicationSession(client, sessionIds, options) {
  const session = variant(
    await client.send(createSessionRequest(options.workspace, options.workspace, options.sessionAlias)),
    'SessionCreated',
  ).session
  sessionIds.push(session.id)
  await client.send(attachToSessionRequest(session.id, `${options.attachAlias}-${process.pid}`))

  const producerAgent = variant(
    await client.send(spawnAgentRequest(session.id, 'dev-stub', `${options.agentAlias}-producer`, 'workflow-dashboard-producer-node', options.workspace, 'low')),
    'AgentSpawned',
  ).agent
  const producerRun = variant(
    await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'workflow-dashboard-producer-node', 'low', producerAgent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, producerRun.id)

  const dashboardAgent = variant(
    await client.send(spawnAgentRequest(
      session.id,
      options.provider,
      options.agentAlias,
      options.model,
      options.workspace,
      options.effort,
    )),
    'AgentSpawned',
  ).agent
  const dashboardRun = variant(
    await client.send(launchProviderRunRequest(
      session.id,
      options.provider,
      options.accountProfile,
      options.model,
      options.effort,
      dashboardAgent.id,
    )),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, dashboardRun.id)

  const finalizerAgent = variant(
    await client.send(spawnAgentRequest(session.id, 'dev-stub', `${options.agentAlias}-finalizer`, 'workflow-final-passthrough-node', options.workspace, 'low')),
    'AgentSpawned',
  ).agent
  const finalizerRun = variant(
    await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'workflow-final-passthrough-node', 'low', finalizerAgent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, finalizerRun.id)

  const workflow = variant(await client.send(createWorkflowRequest(session.id, options.workflowAlias)), 'WorkflowCreated').workflow
  const producerNode = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, producerAgent.id)), 'WorkflowNodeAdded').node
  const dashboardNode = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, dashboardAgent.id)), 'WorkflowNodeAdded').node
  const finalizerNode = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, finalizerAgent.id)), 'WorkflowNodeAdded').node
  const schemaPath = path.join(options.workspace, 'dashboard-output.schema.json')
  await writeFile(schemaPath, JSON.stringify({
    type: 'object',
    required: ['kind', 'html'],
    properties: {
      kind: { const: 'html' },
      html: {
        type: 'string',
        minLength: 200,
      },
    },
    additionalProperties: false,
  }, null, 2), 'utf8')
  await client.send(setWorkflowRunOutputSchemaRequest(session.id, workflow.id, schemaPath))
  await client.send(updateWorkflowNodeInstructionsRequest(
    session.id,
    workflow.id,
    producerNode.id,
    [
      'Acknowledge the workflow turn, submit the deterministic intermediate output, and hand off the dashboard_task contract to the renderer.',
      'Do not generate the final HTML in this node. The downstream real provider node must generate the final HTML itself.',
    ].join('\n'),
  ))
  await client.send(updateWorkflowNodeInstructionsRequest(
    session.id,
    workflow.id,
    dashboardNode.id,
    [
      'You are the real-provider dashboard renderer for the publication validation drill.',
      'After the required ack_workflow_turn call succeeds, continue this same workflow turn immediately.',
      'Use the upstream dashboard_task handoff message as the authoritative task contract.',
      'Think through a compact implementation plan before writing the file: layout, colors, responsive behavior, KPI data, chart-like visual, and final handoff shape.',
      'Generate a compact, complete, self-contained HTML document for a vibrant dashboard and write it to `dashboard.html` in the current workspace.',
      'Keep `dashboard.html` compact enough for a drill, preferably under 4000 bytes. Use short inline CSS and a few metric blocks.',
      'The HTML must visibly include the title text `Real Provider Workflow Dashboard`.',
      'The HTML must include `data-arroba-real-provider-dashboard="true"` on the main dashboard element.',
      'Include at least four KPI cards, one chart-like visual using HTML/CSS only, and one operational status section.',
      'Make the design responsive for both desktop and narrow mobile widths without scripts.',
      'Use inline CSS only; do not use scripts, external assets, network, or unrelated workspace inspection.',
      'Prefer a shell here-doc to write the file. If you use another file-writing tool, keep the file small.',
      'Use the endpoint prompt only as theme guidance. Do not explain your design.',
      'Do not call validate_and_submit_workflow_run_output. The downstream finalizer node will submit the final workflow output.',
      'After writing `dashboard.html`, emit a small workflow handoff; do not include the full HTML in the chat output.',
      'Emit exactly one fenced json block and then stop.',
      'The fenced json block must be shaped as {"summary":"Real provider dashboard completed.","output":{"message":{"kind":"html_file","path":"dashboard.html"}}}.',
      'Do not put the HTML outside the file.',
    ].join('\n'),
  ))
  await client.send(updateWorkflowNodeInstructionsRequest(
    session.id,
    workflow.id,
    finalizerNode.id,
    [
      'Acknowledge the workflow turn, parse the upstream renderer output.message, and submit that exact HTML object as the final workflow output.',
      'If the upstream renderer provides an html_file path, read the file and submit its contents as {"kind":"html","html":"<file contents>"}.',
      'Do not generate or modify HTML in this node.',
    ].join('\n'),
  ))
  await client.send(addWorkflowEdgeRequest(session.id, workflow.id, producerNode.id, dashboardNode.id))
  await client.send(addWorkflowEdgeRequest(session.id, workflow.id, dashboardNode.id, finalizerNode.id))
  await client.send(setWorkflowNodeCanCompleteRunRequest(session.id, workflow.id, producerNode.id, false))
  await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(session.id, workflow.id, producerNode.id, true))
  await client.send(setWorkflowNodeCanCompleteRunRequest(session.id, workflow.id, dashboardNode.id, false))
  await client.send(setWorkflowNodeCanCompleteRunRequest(session.id, workflow.id, finalizerNode.id, true))

  const endpoint = variant(
    await client.send(createWorkflowEndpointRequest(session.id, workflow.id, producerNode.id, options.endpointAlias)),
    'WorkflowEndpointCreated',
  ).endpoint
  const traceLevels = ['output_summary', 'assistant_messages', 'thinking', 'tool_use']
  const publication = variant(
    await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: options.publicationAlias,
      route: options.route,
      methods: options.methods,
      transport: { kind: options.transportKind },
      parser: { kind: 'json' },
      traceExposure: {
        nodes: {
          [producerNode.id]: traceLevels,
          [dashboardNode.id]: traceLevels,
          [finalizerNode.id]: traceLevels,
        },
      },
      mode: 'async',
      syncTimeoutMs: 360_000,
      pollMs: 500,
    })),
    'WorkflowPublicationCreated',
  ).publication
  return {
    session,
    workflow,
    producerNode,
    dashboardNode,
    finalizerNode,
    producerAgent,
    dashboardAgent,
    finalizerAgent,
    producerRun,
    dashboardRun,
    finalizerRun,
    endpoint,
    publication,
    requiredTraceLevels: options.expectThinking
      ? ['output_summary', 'assistant_messages', 'thinking', 'tool_use']
      : ['output_summary', 'assistant_messages', 'tool_use'],
  }
}
