const { addWorkflowNodeRequest, attachToSessionRequest, createSessionRequest, createWorkflowEndpointRequest, createWorkflowPublicationRequest, createWorkflowRequest, getProviderRunRequest, launchProviderRunRequest, setWorkflowNodeCanCompleteRunRequest, setWorkflowNodeCanEmitIntermediateOutputRequest, spawnAgentRequest, updateWorkflowNodeInstructionsRequest } = await import('../../../../packages/kernel-client/dist/ipc-requests.js')
import { logStep, REAL_DASHBOARD_PROMPT, variant } from './live-workflow-publication-drill-runtime.mjs'
import { HUMAN_HTTP_COMPOSER_METHODS, createDashboardPublicationSession, createDeterministicPublicationSession, createRealProviderDashboardPublicationSession, createSchedulePublicationArtifacts, publicationRequestTransportOptions } from './live-workflow-publication-drill-session-builders.mjs'
import { waitForProviderRunReady } from './live-workflow-publication-drill-waiters.mjs'

export async function createPublicationDrillSessionSuite(client, sessionIds, {
  workspace,
  apiSseWorkspace,
  apiSseTunnelWorkspace,
  websocketWorkspace,
  websocketTunnelWorkspace,
  browserWorkspace,
  browserRootWorkspace,
  browserHtmlWorkspace,
  browserRealHtmlWorkspace,
  mcpWorkspace,
  scheduleWorkspace,
  realDashboard,
}) {
  logStep('create_session')
  const session = variant(await client.send(createSessionRequest(workspace, workspace, 'publication-drill')), 'SessionCreated').session
  sessionIds.push(session.id)
  await client.send(attachToSessionRequest(session.id, `publication-drill-${process.pid}`))

  logStep('spawn_agent')
  const agent = variant(
    await client.send(spawnAgentRequest(session.id, 'dev-stub', 'publisher', 'publication-drill-model', workspace, 'low')),
    'AgentSpawned',
  ).agent
  const providerRun = variant(
    await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'publication-drill-model', 'low', agent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, providerRun.id)

  logStep('create_workflow')
  const workflow = variant(await client.send(createWorkflowRequest(session.id, 'published')), 'WorkflowCreated').workflow
  const node = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, agent.id)), 'WorkflowNodeAdded').node
  await client.send(updateWorkflowNodeInstructionsRequest(
    session.id,
    workflow.id,
    node.id,
    'For the publication drill, acknowledge the request and complete the workflow.',
  ))
  const endpoint = variant(
    await client.send(createWorkflowEndpointRequest(session.id, workflow.id, node.id, 'http')),
    'WorkflowEndpointCreated',
  ).endpoint

  logStep('create_publication')
  const publication = variant(
    await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: 'public_http',
      route: '/qa/*',
      methods: HUMAN_HTTP_COMPOSER_METHODS,
      parser: { kind: 'path_template', template: '/qa/:task' },
      mode: 'async',
    })),
    'WorkflowPublicationCreated',
  ).publication

  logStep('create_api_sse_publication')
  const apiSsePublication = variant(
    await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: 'public_api_sse',
      route: '/invoke',
      methods: ['POST'],
      transport: { kind: 'api_sse_json' },
      parser: { kind: 'json' },
      traceExposure: { nodes: { [node.id]: ['output_summary', 'assistant_messages', 'thinking', 'tool_use'] } },
      mode: 'async',
    })),
    'WorkflowPublicationCreated',
  ).publication

  logStep('create_api_sse_final_session')
  const apiSseSession = variant(
    await client.send(createSessionRequest(apiSseWorkspace, apiSseWorkspace, 'publication-drill-api-sse-final')),
    'SessionCreated',
  ).session
  sessionIds.push(apiSseSession.id)
  await client.send(attachToSessionRequest(apiSseSession.id, `publication-drill-api-sse-final-${process.pid}`))
  const apiSseAgent = variant(
    await client.send(spawnAgentRequest(apiSseSession.id, 'dev-stub', 'api-sse-final', 'workflow-intermediate-node', apiSseWorkspace, 'low')),
    'AgentSpawned',
  ).agent
  const apiSseProviderRun = variant(
    await client.send(launchProviderRunRequest(apiSseSession.id, 'dev-stub', 'default', 'workflow-intermediate-node', 'low', apiSseAgent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, apiSseProviderRun.id)
  const apiSseWorkflow = variant(await client.send(createWorkflowRequest(apiSseSession.id, 'published-api-sse-final')), 'WorkflowCreated').workflow
  const apiSseNode = variant(await client.send(addWorkflowNodeRequest(apiSseSession.id, apiSseWorkflow.id, apiSseAgent.id)), 'WorkflowNodeAdded').node
  await client.send(updateWorkflowNodeInstructionsRequest(
    apiSseSession.id,
    apiSseWorkflow.id,
    apiSseNode.id,
    'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
  ))
  await client.send(setWorkflowNodeCanCompleteRunRequest(apiSseSession.id, apiSseWorkflow.id, apiSseNode.id, true))
  await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(apiSseSession.id, apiSseWorkflow.id, apiSseNode.id, true))
  const apiSseFinalEndpoint = variant(
    await client.send(createWorkflowEndpointRequest(apiSseSession.id, apiSseWorkflow.id, apiSseNode.id, 'api')),
    'WorkflowEndpointCreated',
  ).endpoint
  const apiSseFinalPublication = variant(
    await client.send(createWorkflowPublicationRequest(apiSseSession.id, apiSseWorkflow.id, apiSseFinalEndpoint.id, {
      alias: 'public_api_sse_final',
      route: '/invoke',
      methods: ['POST'],
      transport: { kind: 'api_sse_json' },
      parser: { kind: 'json' },
      traceExposure: { nodes: { [apiSseNode.id]: ['output_summary', 'assistant_messages', 'thinking', 'tool_use'] } },
      mode: 'async',
    })),
    'WorkflowPublicationCreated',
  ).publication
  logStep('create_api_sse_tunnel_session')
  const apiSseTunnel = await createDeterministicPublicationSession(client, sessionIds, {
    workspace: apiSseTunnelWorkspace,
    sessionAlias: 'publication-drill-api-sse-tunnel-final',
    attachAlias: 'publication-drill-api-sse-tunnel-final',
    agentAlias: 'api-sse-tunnel-final',
    workflowAlias: 'published-api-sse-tunnel-final',
    endpointAlias: 'api-tunnel',
    publicationAlias: 'public_api_sse_tunnel_final',
    route: '/invoke',
    methods: ['POST'],
    transportKind: 'api_sse_json',
    traceExposure: 'all',
  })
  logStep('create_websocket_session')
  const websocketSession = variant(
    await client.send(createSessionRequest(websocketWorkspace, websocketWorkspace, 'publication-drill-websocket-final')),
    'SessionCreated',
  ).session
  sessionIds.push(websocketSession.id)
  await client.send(attachToSessionRequest(websocketSession.id, `publication-drill-websocket-final-${process.pid}`))
  const websocketAgent = variant(
    await client.send(spawnAgentRequest(websocketSession.id, 'dev-stub', 'websocket-final', 'workflow-intermediate-node', websocketWorkspace, 'low')),
    'AgentSpawned',
  ).agent
  const websocketProviderRun = variant(
    await client.send(launchProviderRunRequest(websocketSession.id, 'dev-stub', 'default', 'workflow-intermediate-node', 'low', websocketAgent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, websocketProviderRun.id)
  const websocketWorkflow = variant(await client.send(createWorkflowRequest(websocketSession.id, 'published-websocket-final')), 'WorkflowCreated').workflow
  const websocketNode = variant(await client.send(addWorkflowNodeRequest(websocketSession.id, websocketWorkflow.id, websocketAgent.id)), 'WorkflowNodeAdded').node
  await client.send(updateWorkflowNodeInstructionsRequest(
    websocketSession.id,
    websocketWorkflow.id,
    websocketNode.id,
    'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
  ))
  await client.send(setWorkflowNodeCanCompleteRunRequest(websocketSession.id, websocketWorkflow.id, websocketNode.id, true))
  await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(websocketSession.id, websocketWorkflow.id, websocketNode.id, true))
  const websocketEndpoint = variant(
    await client.send(createWorkflowEndpointRequest(websocketSession.id, websocketWorkflow.id, websocketNode.id, 'websocket')),
    'WorkflowEndpointCreated',
  ).endpoint
  const websocketFinalPublication = variant(
    await client.send(createWorkflowPublicationRequest(websocketSession.id, websocketWorkflow.id, websocketEndpoint.id, {
      alias: 'public_websocket_final',
      ...publicationRequestTransportOptions({
        route: '/.well-known/arroba/publication/ws',
        transportKind: 'websocket_json',
      }),
      traceExposure: { nodes: { [websocketNode.id]: ['output_summary', 'assistant_messages', 'thinking', 'tool_use'] } },
    })),
    'WorkflowPublicationCreated',
  ).publication
  logStep('create_websocket_tunnel_session')
  const websocketTunnel = await createDeterministicPublicationSession(client, sessionIds, {
    workspace: websocketTunnelWorkspace,
    sessionAlias: 'publication-drill-websocket-tunnel-final',
    attachAlias: 'publication-drill-websocket-tunnel-final',
    agentAlias: 'websocket-tunnel-final',
    workflowAlias: 'published-websocket-tunnel-final',
    endpointAlias: 'websocket-tunnel',
    publicationAlias: 'public_websocket_tunnel_final',
    route: '/.well-known/arroba/publication/ws',
    transportKind: 'websocket_json',
    traceExposure: 'all',
  })
  logStep('create_browser_session')
  const browserSession = variant(
    await client.send(createSessionRequest(browserWorkspace, browserWorkspace, 'publication-drill-human-http-final')),
    'SessionCreated',
  ).session
  sessionIds.push(browserSession.id)
  await client.send(attachToSessionRequest(browserSession.id, `publication-drill-human-http-final-${process.pid}`))
  const browserAgent = variant(
    await client.send(spawnAgentRequest(browserSession.id, 'dev-stub', 'human-http-final', 'workflow-intermediate-node', browserWorkspace, 'low')),
    'AgentSpawned',
  ).agent
  const browserProviderRun = variant(
    await client.send(launchProviderRunRequest(browserSession.id, 'dev-stub', 'default', 'workflow-intermediate-node', 'low', browserAgent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, browserProviderRun.id)
  const browserWorkflow = variant(await client.send(createWorkflowRequest(browserSession.id, 'published-human-http-final')), 'WorkflowCreated').workflow
  const browserNode = variant(await client.send(addWorkflowNodeRequest(browserSession.id, browserWorkflow.id, browserAgent.id)), 'WorkflowNodeAdded').node
  await client.send(updateWorkflowNodeInstructionsRequest(
    browserSession.id,
    browserWorkflow.id,
    browserNode.id,
    'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
  ))
  await client.send(setWorkflowNodeCanCompleteRunRequest(browserSession.id, browserWorkflow.id, browserNode.id, true))
  await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(browserSession.id, browserWorkflow.id, browserNode.id, true))
  const browserEndpoint = variant(
    await client.send(createWorkflowEndpointRequest(browserSession.id, browserWorkflow.id, browserNode.id, 'browser')),
    'WorkflowEndpointCreated',
  ).endpoint
  const humanHttpFinalPublication = variant(
    await client.send(createWorkflowPublicationRequest(browserSession.id, browserWorkflow.id, browserEndpoint.id, {
      alias: 'public_human_http_final',
      route: '/final/*',
      methods: HUMAN_HTTP_COMPOSER_METHODS,
      transport: { kind: 'human_http' },
      parser: { kind: 'path_template', template: '/final/:task' },
      traceExposure: { nodes: { [browserNode.id]: ['output_summary', 'assistant_messages', 'thinking', 'tool_use'] } },
      mode: 'async',
    })),
    'WorkflowPublicationCreated',
  ).publication
  logStep('create_browser_root_form_session')
  const browserRoot = await createDeterministicPublicationSession(client, sessionIds, {
    workspace: browserRootWorkspace,
    sessionAlias: 'publication-drill-human-http-root-form',
    attachAlias: 'publication-drill-human-http-root-form',
    agentAlias: 'human-http-root-form',
    workflowAlias: 'published-human-http-root-form',
    endpointAlias: 'browser-root',
    publicationAlias: 'public_human_http_root_form',
    route: '/final/*',
    methods: HUMAN_HTTP_COMPOSER_METHODS,
    transportKind: 'human_http',
    traceExposure: 'all',
  })
  logStep('create_browser_html_final_session')
  const browserHtml = await createDashboardPublicationSession(client, sessionIds, {
    workspace: browserHtmlWorkspace,
    sessionAlias: 'publication-drill-human-http-html-final',
    attachAlias: 'publication-drill-human-http-html-final',
    agentAlias: 'human-http-html-final',
    workflowAlias: 'published-human-http-html-final',
    endpointAlias: 'browser-html',
    publicationAlias: 'public_human_http_html_final',
    route: '/dashboard/*',
    methods: ['GET'],
    transportKind: 'human_http',
  })
  let browserRealHtml = null
  if (realDashboard) {
    logStep('create_browser_real_html_final_session', realDashboard)
    browserRealHtml = await createRealProviderDashboardPublicationSession(client, sessionIds, {
      workspace: browserRealHtmlWorkspace,
      sessionAlias: `publication-drill-human-http-real-html-final-${realDashboard.provider}`,
      attachAlias: `publication-drill-human-http-real-html-final-${realDashboard.provider}`,
      agentAlias: `${realDashboard.provider}-real-dashboard-renderer`,
      workflowAlias: `published-human-http-real-html-final-${realDashboard.provider}`,
      endpointAlias: 'browser-real-html',
      publicationAlias: 'public_human_http_real_html_final',
      route: '/real-dashboard/*',
      methods: ['GET'],
      transportKind: 'human_http',
      provider: realDashboard.provider,
      accountProfile: realDashboard.accountProfile,
      model: realDashboard.model,
      effort: realDashboard.effort,
      expectThinking: realDashboard.expectThinking,
    })
  }
  logStep('create_mcp_session')
  const mcpSession = variant(
    await client.send(createSessionRequest(mcpWorkspace, mcpWorkspace, 'publication-drill-mcp')),
    'SessionCreated',
  ).session
  sessionIds.push(mcpSession.id)
  await client.send(attachToSessionRequest(mcpSession.id, `publication-drill-mcp-${process.pid}`))
  const mcpAgent = variant(
    await client.send(spawnAgentRequest(mcpSession.id, 'dev-stub', 'mcp-final', 'workflow-single-turn-node', mcpWorkspace, 'low')),
    'AgentSpawned',
  ).agent
  const mcpProviderRun = variant(
    await client.send(launchProviderRunRequest(mcpSession.id, 'dev-stub', 'default', 'workflow-single-turn-node', 'low', mcpAgent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, mcpProviderRun.id)
  const mcpWorkflow = variant(await client.send(createWorkflowRequest(mcpSession.id, 'published-mcp')), 'WorkflowCreated').workflow
  const mcpNode = variant(await client.send(addWorkflowNodeRequest(mcpSession.id, mcpWorkflow.id, mcpAgent.id)), 'WorkflowNodeAdded').node
  await client.send(updateWorkflowNodeInstructionsRequest(
    mcpSession.id,
    mcpWorkflow.id,
    mcpNode.id,
    'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
  ))
  await client.send(setWorkflowNodeCanCompleteRunRequest(mcpSession.id, mcpWorkflow.id, mcpNode.id, true))
  const mcpEndpoint = variant(
    await client.send(createWorkflowEndpointRequest(mcpSession.id, mcpWorkflow.id, mcpNode.id, 'mcp')),
    'WorkflowEndpointCreated',
  ).endpoint
  const mcpPublication = variant(
    await client.send(createWorkflowPublicationRequest(mcpSession.id, mcpWorkflow.id, mcpEndpoint.id, {
      alias: 'public_mcp',
      ...publicationRequestTransportOptions({
        route: '/mcp',
        methods: ['POST'],
        transportKind: 'mcp',
      }),
      traceExposure: { nodes: { [mcpNode.id]: ['output_summary', 'assistant_messages', 'thinking'] } },
    })),
    'WorkflowPublicationCreated',
  ).publication

  logStep('create_schedule_session')
  const scheduleSession = variant(
    await client.send(createSessionRequest(scheduleWorkspace, scheduleWorkspace, 'publication-drill-schedule')),
    'SessionCreated',
  ).session
  sessionIds.push(scheduleSession.id)
  await client.send(attachToSessionRequest(scheduleSession.id, `publication-drill-schedule-${process.pid}`))
  const scheduleAgent = variant(
    await client.send(spawnAgentRequest(scheduleSession.id, 'dev-stub', 'schedule-final', 'workflow-intermediate-node', scheduleWorkspace, 'low')),
    'AgentSpawned',
  ).agent
  const scheduleProviderRun = variant(
    await client.send(launchProviderRunRequest(scheduleSession.id, 'dev-stub', 'default', 'workflow-intermediate-node', 'low', scheduleAgent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, scheduleProviderRun.id)
  const scheduleWorkflow = variant(await client.send(createWorkflowRequest(scheduleSession.id, 'published-schedule')), 'WorkflowCreated').workflow
  const scheduleNode = variant(await client.send(addWorkflowNodeRequest(scheduleSession.id, scheduleWorkflow.id, scheduleAgent.id)), 'WorkflowNodeAdded').node
  await client.send(updateWorkflowNodeInstructionsRequest(
    scheduleSession.id,
    scheduleWorkflow.id,
    scheduleNode.id,
    'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
  ))
  await client.send(setWorkflowNodeCanCompleteRunRequest(scheduleSession.id, scheduleWorkflow.id, scheduleNode.id, true))
  await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(scheduleSession.id, scheduleWorkflow.id, scheduleNode.id, true))
  const scheduleEndpoint = variant(
    await client.send(createWorkflowEndpointRequest(scheduleSession.id, scheduleWorkflow.id, scheduleNode.id, 'schedule')),
    'WorkflowEndpointCreated',
  ).endpoint
  const { schedule, publication: schedulePublication } = await createSchedulePublicationArtifacts(client, {
    sessionId: scheduleSession.id,
    workflowId: scheduleWorkflow.id,
    endpointId: scheduleEndpoint.id,
  })
  return {
    session,
    agent,
    providerRun,
    workflow,
    node,
    endpoint,
    publication,
    apiSsePublication,
    apiSseSession,
    apiSseAgent,
    apiSseProviderRun,
    apiSseWorkflow,
    apiSseNode,
    apiSseFinalEndpoint,
    apiSseFinalPublication,
    apiSseTunnel,
    websocketSession,
    websocketAgent,
    websocketProviderRun,
    websocketWorkflow,
    websocketNode,
    websocketEndpoint,
    websocketFinalPublication,
    websocketTunnel,
    browserSession,
    browserAgent,
    browserProviderRun,
    browserWorkflow,
    browserNode,
    browserEndpoint,
    humanHttpFinalPublication,
    browserRoot,
    browserHtml,
    browserRealHtml,
    mcpSession,
    mcpAgent,
    mcpProviderRun,
    mcpWorkflow,
    mcpNode,
    mcpEndpoint,
    mcpPublication,
    scheduleSession,
    scheduleAgent,
    scheduleProviderRun,
    scheduleWorkflow,
    scheduleNode,
    scheduleEndpoint,
    schedule,
    schedulePublication
  }
}
