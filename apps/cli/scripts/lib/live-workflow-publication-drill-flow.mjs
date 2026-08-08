import path from 'node:path'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
const { LocalIpcClient } = await import('../../../../packages/kernel-client/dist/ipc.js')
const { createDefaultShellContext, parseShellCommand } = await import('../../../../packages/kernel-client/dist/shell-core.js')
const { executeShellCommand } = await import('../../../../packages/kernel-client/dist/shell-executor.js')
const { createSessionRequest, attachToSessionRequest, spawnAgentRequest, launchProviderRunRequest, createWorkflowRequest, addWorkflowNodeRequest, updateWorkflowNodeInstructionsRequest, createWorkflowEndpointRequest, createWorkflowPublicationRequest, setWorkflowNodeCanCompleteRunRequest, setWorkflowNodeCanEmitIntermediateOutputRequest, getSessionStateRequest, endSessionRequest, createWorkflowScheduleRequest } = await import('../../../../packages/kernel-client/dist/ipc-requests.js')
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './drill-artifacts.mjs'
import { REAL_DASHBOARD_PROMPT, buildRustBinary, cliRoot, createContainerPortablePackage, createSelfSignedCertificate, envFlag, freePort, hasAcceptedRunMetadata, logStep, nowStamp, readSseUntilEvent, realDashboardOptionsFromEnv, removeDockerContainer, removeDockerImage, repoRoot, run, secureGatewayPublicationEnvs, sseEventNames, startProcess, startServeWithProviderPrompt, stopProcess, tail, variant, websocketUrlFromHttp, withPublicationDrillProviderInventory } from './live-workflow-publication-drill-runtime.mjs'
import { invokePublicationWebSocket } from './live-workflow-publication-drill-runtime.mjs'
import { runHumanHttpBrowserDrill, runHumanHttpHtmlFinalBrowserDrill, runHumanHttpRootFormBrowserDrill, runSharedViewerBrowserDrill } from './live-workflow-publication-drill-browser.mjs'
import { assertGatewayDoesNotListen, assertPackageDoesNotContain, assertPublicationRuntimeSessionHidden, collectPublicationInvocationDiagnostic, createUnavailableProviderPackage, waitForGateway, waitForKernel, waitForPublicationStatusLatestOutput, waitForRegisteredPublicationEndpoint, waitForRelayTarget, waitForScheduledWorkflowRun, waitForTcpPort } from './live-workflow-publication-drill-waiters.mjs'
import { createPublicationDrillSessionSuite } from './live-workflow-publication-drill-session-suite.mjs'
import { createRealProviderDashboardPublicationSession } from './live-workflow-publication-drill-session-builders.mjs'
import { runLivePublicationManifestMode } from './live-workflow-publication-drill-manifest.mjs'
import { runPublicationPackageValidation } from './live-workflow-publication-drill-package-validation.mjs'

export async function runLiveWorkflowPublicationDrill() {
  const drillTmpRoot = process.env.ARROBA_PUBLICATION_DRILL_TMPDIR
    ?? path.join(repoRoot, '.artifacts', 'live-workflow-publication-drill')
  await mkdir(drillTmpRoot, { recursive: true })
  const root = path.join(drillTmpRoot, `arroba-publication-drill-${nowStamp()}`)
  await prepareDrillArtifacts(root)
  const workspace = path.join(root, 'workspace')
  const apiSseWorkspace = path.join(root, 'api-sse-workspace')
  const apiSseTunnelWorkspace = path.join(root, 'api-sse-tunnel-workspace')
  const websocketWorkspace = path.join(root, 'websocket-workspace')
  const websocketTunnelWorkspace = path.join(root, 'websocket-tunnel-workspace')
  const browserWorkspace = path.join(root, 'browser-workspace')
  const browserRootWorkspace = path.join(root, 'browser-root-workspace')
  const browserHtmlWorkspace = path.join(root, 'browser-html-workspace')
  const browserRealHtmlWorkspace = path.join(root, 'browser-real-html-workspace')
  const mcpWorkspace = path.join(root, 'mcp-workspace')
  const scheduleWorkspace = path.join(root, 'schedule-workspace')
  const realDashboard = realDashboardOptionsFromEnv()
  const hostHome = process.env.HOME
  const home = path.join(root, 'home')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const relayPort = await freePort()
  const kernelPort = await freePort()
  const mcpPort = await freePort()
  const opencodePort = await freePort()
  const codexPort = await freePort()
  const gatewayPort = await freePort()
  const gatewayHttpsPort = await freePort()
  const relayUrl = `ws://127.0.0.1:${relayPort}`
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const gatewayUrl = `http://127.0.0.1:${gatewayPort}`
  const gatewayHttpsUrl = `https://127.0.0.1:${gatewayHttpsPort}`
  const relayToken = `publication-drill-relay-${process.pid}`
  const daemonAlias = `publication-drill-${process.pid}`
  const env = withPublicationDrillProviderInventory({
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_DAEMON_ID: daemonAlias,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(root, 'history'),
    ARROBA_RELAY_URL: relayUrl,
    ARROBA_RELAY_TOKEN: relayToken,
  })
  if (realDashboard?.useHostProviderHome && hostHome) {
    if (!env.CODEX_HOME) env.CODEX_HOME = path.join(hostHome, '.codex')
    if (!env.ARROBA_CLAUDE_CONFIG) env.ARROBA_CLAUDE_CONFIG = path.join(hostHome, '.claude.json')
  }

  let relay = null
  let kernel = null
  let gateway = null
  let client = null
  const sessionIds = []
  const dockerContainers = []
  const dockerImages = []
  let succeeded = false
  let failure = null
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(apiSseWorkspace, { recursive: true })
    await mkdir(apiSseTunnelWorkspace, { recursive: true })
    await mkdir(websocketWorkspace, { recursive: true })
    await mkdir(websocketTunnelWorkspace, { recursive: true })
    await mkdir(browserWorkspace, { recursive: true })
    await mkdir(browserRootWorkspace, { recursive: true })
    await mkdir(browserHtmlWorkspace, { recursive: true })
    await mkdir(browserRealHtmlWorkspace, { recursive: true })
    await mkdir(mcpWorkspace, { recursive: true })
    await mkdir(scheduleWorkspace, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')
    const tls = await createSelfSignedCertificate(root)

    const kernelBinary = await buildRustBinary('arroba-kernel')
    const cliBinary = await buildRustBinary('arroba-cli')
    const relayBinary = await buildRustBinary('arroba-relay')
    relay = startProcess(relayBinary, [], {
      ...env,
      ARROBA_RELAY_HOST: '127.0.0.1',
      ARROBA_RELAY_PORT: String(relayPort),
      ARROBA_RELAY_TOKEN: relayToken,
    }, 'relay')
    await waitForTcpPort('127.0.0.1', relayPort)
    kernel = startProcess(kernelBinary, [], env, 'kernel')
    await waitForKernel(kernelUrl)
    await waitForRelayTarget(relayUrl, relayToken, daemonAlias)
    client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })

    if (realDashboard && envFlag('ARROBA_PUBLICATION_REAL_DASHBOARD_ONLY')) {
      const maxAttempts = Number(process.env.ARROBA_PUBLICATION_REAL_DASHBOARD_ATTEMPTS || '3')
      for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
        logStep('create_browser_real_html_final_session', { ...realDashboard, attempt, maxAttempts })
        const attemptWorkspace = path.join(browserRealHtmlWorkspace, `attempt-${attempt}`)
        await mkdir(attemptWorkspace, { recursive: true })
        const browserRealHtml = await createRealProviderDashboardPublicationSession(client, sessionIds, {
          workspace: attemptWorkspace,
          sessionAlias: `publication-drill-human-http-real-html-final-${realDashboard.provider}-${attempt}`,
          attachAlias: `publication-drill-human-http-real-html-final-${realDashboard.provider}-${attempt}`,
          agentAlias: `${realDashboard.provider}-real-dashboard-renderer`,
          workflowAlias: `published-human-http-real-html-final-${realDashboard.provider}-${attempt}`,
          endpointAlias: 'browser-real-html',
          publicationAlias: `public_human_http_real_html_final_${attempt}`,
          route: `/real-dashboard-${attempt}/*`,
          methods: ['GET'],
          transportKind: 'human_http',
          provider: realDashboard.provider,
          accountProfile: realDashboard.accountProfile,
          model: realDashboard.model,
          effort: realDashboard.effort,
          expectThinking: realDashboard.expectThinking,
        })

        logStep('invoke_human_http_real_html_final_browser', { attempt, maxAttempts })
        gateway = startProcess(
          process.execPath,
          [path.join(repoRoot, 'apps/server/dist/index.js')],
          {
            ...env,
            HOST: '127.0.0.1',
            PORT: String(gatewayPort),
            ARROBA_KERNEL_URL: kernelUrl,
            ARROBA_PUBLICATION_SESSION_ID: browserRealHtml.session.id,
            ARROBA_PUBLICATION_ID: browserRealHtml.publication.id,
          },
          'gateway-human-http-real-html-final',
        )
        await waitForGateway(gatewayUrl)
        try {
          await runHumanHttpHtmlFinalBrowserDrill({
            url: `${gatewayUrl}/real-dashboard-${attempt}/${encodeURIComponent(REAL_DASHBOARD_PROMPT)}`,
            root,
            timeoutMs: 360_000,
            expectedHtmlText: 'Real Provider Workflow Dashboard',
            requiredHtmlSnippets: ['data-arroba-real-provider-dashboard="true"'],
            requiredTraceLevels: browserRealHtml.requiredTraceLevels,
            requiredTraceAlias: browserRealHtml.dashboardAgent.alias ?? browserRealHtml.dashboardAgent.id,
            visualArtifactPrefix: 'local-human-http-real-dashboard',
            requirePartial: false,
          })
          await stopProcess(gateway)
          gateway = null
          succeeded = true
          logStep('real_dashboard_only_ok', { attempt })
          return
        } catch (error) {
          await stopProcess(gateway).catch(() => {})
          gateway = null
          if (attempt < maxAttempts && String(error?.message || error).includes('completed without required trace levels')) {
            logStep('real_dashboard_retry_missing_required_trace_levels', { attempt, error: error.message })
            continue
          }
          throw error
        }
      }
      succeeded = true
      return
    }

    const {
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
    } = await createPublicationDrillSessionSuite(client, sessionIds, {
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
    })

    if (process.env.ARROBA_PUBLICATION_LIVE_MANIFEST) {
      await runLivePublicationManifestMode({
        manifestPath: process.env.ARROBA_PUBLICATION_LIVE_MANIFEST,
        client,
        env,
        kernelUrl,
        relayPort,
        publications: [{
          key: 'human_http',
          transport: 'human_http',
          sessionId: browserSession.id,
          publication: humanHttpFinalPublication,
        }, {
          key: 'human_http_dashboard',
          transport: 'human_http',
          sessionId: browserHtml.session.id,
          publication: browserHtml.publication,
        }, ...(browserRealHtml ? [{
          key: 'human_http_dashboard_real',
          transport: 'human_http',
          sessionId: browserRealHtml.session.id,
          publication: browserRealHtml.publication,
          expectedHtmlText: 'Real Provider Workflow Dashboard',
          requiredHtmlSnippets: ['data-arroba-real-provider-dashboard="true"'],
          promptText: REAL_DASHBOARD_PROMPT,
          requiredTraceLevels: browserRealHtml.requiredTraceLevels,
          requiredTraceAlias: browserRealHtml.dashboardAgent.alias ?? browserRealHtml.dashboardAgent.id,
        }] : []), {
          key: 'api_sse_json',
          transport: 'api_sse_json',
          sessionId: apiSseSession.id,
          publication: apiSseFinalPublication,
        }, {
          key: 'websocket_json',
          transport: 'websocket_json',
          sessionId: websocketSession.id,
          publication: websocketFinalPublication,
        }, {
          key: 'mcp',
          transport: 'mcp',
          sessionId: mcpSession.id,
          publication: mcpPublication,
        }, {
          key: 'schedule',
          transport: 'schedule',
          sessionId: scheduleSession.id,
          publication: schedulePublication,
        }],
      })
      succeeded = true
      return
    }

    logStep('start_gateway', { publicationId: publication.id })
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: publication.id,
      },
      'gateway',
    )
    await waitForGateway(gatewayUrl)
    const registeredPublication = await waitForRegisteredPublicationEndpoint(
      client,
      session.id,
      publication.id,
      `${gatewayUrl}/`,
      `http://127.0.0.1:${relayPort}/display/publication-`,
    )
    logStep('publication_endpoint_registered', {
      publicationId: registeredPublication.id,
      status: registeredPublication.status,
      openUrl: registeredPublication.open_url,
      deployment: registeredPublication.deployment?.kind ?? null,
    })
    if (registeredPublication.deployment?.kind !== 'tunnel') {
      throw new Error(`expected registered publication deployment tunnel, got ${JSON.stringify(registeredPublication.deployment)}`)
    }

    logStep('invoke_browser_html')
    const rootHtmlResponse = await fetch(`${gatewayUrl}/`, { headers: { accept: 'text/html' } })
    const rootHtml = await rootHtmlResponse.text()
    if (rootHtmlResponse.status !== 200 || !rootHtml.includes('id="invoke-form"')) {
      throw new Error(`expected browser root form HTML, got ${rootHtmlResponse.status}: ${rootHtml.slice(0, 200)}`)
    }
    if (!rootHtml.includes('type="file" name="artifact" multiple')) {
      throw new Error(`expected browser root form artifact upload input, got: ${rootHtml.slice(0, 300)}`)
    }
    const browserResponse = await fetch(`${gatewayUrl}/qa/browser-publication`, { headers: { accept: 'text/html' } })
    const browserInvocationHtml = await browserResponse.text()
    if (browserResponse.status !== 200 || !browserInvocationHtml.includes('EventSource') || !browserInvocationHtml.includes("addEventListener('queued'")) {
      throw new Error(`expected browser invocation HTML with SSE subscription, got ${browserResponse.status}: ${browserInvocationHtml.slice(0, 200)}`)
    }

    logStep('invoke_browser_html_tunnel')
    const tunnelRootResponse = await fetch(registeredPublication.open_url, { headers: { accept: 'text/html' } })
    const tunnelRootHtml = await tunnelRootResponse.text()
    if (tunnelRootResponse.status !== 200 || !tunnelRootHtml.includes('invoke-form')) {
      throw new Error(`expected tunnel root form HTML, got ${tunnelRootResponse.status}: ${tunnelRootHtml.slice(0, 200)}`)
    }
    const tunnelBrowserUrl = new URL('qa/browser-publication-tunnel', registeredPublication.open_url).toString()
    const tunnelBrowserResponse = await fetch(tunnelBrowserUrl, { headers: { accept: 'text/html' } })
    const tunnelBrowserHtml = await tunnelBrowserResponse.text()
    if (tunnelBrowserResponse.status !== 200 || !tunnelBrowserHtml.includes('EventSource') || !tunnelBrowserHtml.includes("addEventListener('queued'")) {
      throw new Error(`expected tunnel browser invocation HTML with SSE subscription, got ${tunnelBrowserResponse.status}: ${tunnelBrowserHtml.slice(0, 200)}`)
    }

    logStep('invoke_browser_upload')
    const uploadResponse = await fetch(`${gatewayUrl}/.well-known/arroba/publication/human-http/invoke`, {
      method: 'POST',
      headers: { accept: 'text/html', 'content-type': 'application/json' },
      body: JSON.stringify({
        prompt: 'browser-upload-publication',
        artifacts: [{
          name: 'upload.txt',
          type: 'text/plain',
          size_bytes: 18,
          data_url: 'data:text/plain;base64,cHVibGljYXRpb24tdXBsb2Fk',
        }],
      }),
    })
    const uploadHtml = await uploadResponse.text()
    if (uploadResponse.status !== 200 || !uploadHtml.includes('EventSource') || !uploadHtml.includes("addEventListener('queued'")) {
      throw new Error(`expected browser upload invocation HTML with SSE subscription, got ${uploadResponse.status}: ${uploadHtml.slice(0, 200)}`)
    }

    logStep('invoke_http')
    const response = await fetch(`${gatewayUrl}/qa/ship-publication`)
    const body = await response.json()
    if (response.status !== 202) {
      throw new Error(`expected HTTP 202 from async publication, got ${response.status}: ${JSON.stringify(body)}`)
    }
    if (!body.accepted || !hasAcceptedRunMetadata(body)) {
      throw new Error(`gateway did not return accepted workflow run metadata: ${JSON.stringify(body)}`)
    }
    logStep('anonymous_ok', { publicationId: publication.id, workflowRunId: body.workflow_run?.id ?? null, queued: body.queued === true })

    logStep('parser_failure_400')
    const badParserResponse = await fetch(`${gatewayUrl}/qa/a/b`)
    if (badParserResponse.status !== 400) {
      throw new Error(`expected parser failure HTTP 400, got ${badParserResponse.status}: ${await badParserResponse.text()}`)
    }

    await stopProcess(gateway)
    gateway = null

    logStep('invoke_api_sse')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: apiSsePublication.id,
      },
      'gateway-api-sse',
    )
    await waitForGateway(gatewayUrl)
    const apiSseResponse = await fetch(`${gatewayUrl}/invoke`, {
      method: 'POST',
      headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
      body: JSON.stringify({
        prompt: 'api-sse-publication',
        artifacts: [{
          name: 'input.txt',
          type: 'text/plain',
          base64: 'YXBpLXN0cmVhbQ==',
        }, {
          name: 'input-url.txt',
          type: 'text/plain',
          url: 'https://example.invalid/arroba-publication-input.txt',
        }],
      }),
    })
    if (apiSseResponse.status !== 200) {
      throw new Error(`expected API SSE queued event, got ${apiSseResponse.status}: ${(await apiSseResponse.text()).slice(0, 400)}`)
    }
    const apiSseBody = await readSseUntilEvent(apiSseResponse, 'queued')
    logStep('api_sse_queued_ok')
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_api_sse_final')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: apiSseSession.id,
        ARROBA_PUBLICATION_ID: apiSseFinalPublication.id,
      },
      'gateway-api-sse-final',
    )
    await waitForGateway(gatewayUrl)
    const apiSseFinalResponse = await fetch(`${gatewayUrl}/invoke`, {
      method: 'POST',
      headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
      body: JSON.stringify({ prompt: 'api-sse-final-publication' }),
    })
    const apiSseFinalBody = await apiSseFinalResponse.text()
    const apiSseFinalEvents = sseEventNames(apiSseFinalBody)
    if (
      apiSseFinalResponse.status !== 200
      || !apiSseFinalEvents.includes('queued')
      || !apiSseFinalEvents.includes('started')
      || !apiSseFinalEvents.includes('partial')
      || !apiSseFinalEvents.includes('trace')
      || !apiSseFinalEvents.includes('final')
      || (!apiSseFinalBody.includes('"value":1841') && !apiSseFinalBody.includes('\\"value\\":1841'))
      || (!apiSseFinalBody.includes('"value":1842') && !apiSseFinalBody.includes('\\"value\\":1842'))
    ) {
      const errorIndex = apiSseFinalBody.lastIndexOf('event: error')
      const diagnostic = errorIndex >= 0 ? apiSseFinalBody.slice(errorIndex, errorIndex + 800) : apiSseFinalBody.slice(0, 2_000)
      throw new Error(`expected API SSE queued/started/partial/final with deterministic output, got ${apiSseFinalResponse.status} ${JSON.stringify(apiSseFinalEvents)}: ${diagnostic}`)
    }
    if (!apiSseFinalBody.includes('"agent_alias":"api-sse-final"')) {
      throw new Error(`expected API SSE trace event tagged with agent alias, got ${apiSseFinalBody.slice(0, 2_000)}`)
    }
    logStep('api_sse_final_ok', { events: apiSseFinalEvents })
    logStep('invoke_api_sse_final_repeat')
    const apiSseFinalRepeatResponse = await fetch(`${gatewayUrl}/invoke`, {
      method: 'POST',
      headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
      body: JSON.stringify({ prompt: 'api-sse-final-publication-repeat' }),
    })
    const apiSseFinalRepeatBody = await apiSseFinalRepeatResponse.text()
    const apiSseFinalRepeatEvents = sseEventNames(apiSseFinalRepeatBody)
    if (
      apiSseFinalRepeatResponse.status !== 200
      || !apiSseFinalRepeatEvents.includes('queued')
      || !apiSseFinalRepeatEvents.includes('started')
      || !apiSseFinalRepeatEvents.includes('partial')
      || !apiSseFinalRepeatEvents.includes('trace')
      || !apiSseFinalRepeatEvents.includes('final')
      || (!apiSseFinalRepeatBody.includes('"value":1841') && !apiSseFinalRepeatBody.includes('\\"value\\":1841'))
      || (!apiSseFinalRepeatBody.includes('"value":1842') && !apiSseFinalRepeatBody.includes('\\"value\\":1842'))
    ) {
      const errorIndex = apiSseFinalRepeatBody.lastIndexOf('event: error')
      const diagnostic = errorIndex >= 0 ? apiSseFinalRepeatBody.slice(errorIndex, errorIndex + 800) : apiSseFinalRepeatBody.slice(0, 2_000)
      const runtimeDiagnostic = await collectPublicationInvocationDiagnostic(client, {
        sessionId: apiSseSession.id,
        workflowId: apiSseWorkflow.id,
        agentId: apiSseAgent.id,
        providerRunId: apiSseProviderRun.id,
      })
      throw new Error(`expected repeated API SSE queued/started/partial/final with deterministic output, got ${apiSseFinalRepeatResponse.status} ${JSON.stringify(apiSseFinalRepeatEvents)}: ${diagnostic}; runtime=${JSON.stringify(runtimeDiagnostic)}`)
    }
    logStep('api_sse_final_repeat_ok', { events: apiSseFinalRepeatEvents })
    await runSharedViewerBrowserDrill({
      baseUrl: `${gatewayUrl}/`,
      root,
      label: 'api_sse_shared_viewer',
      prompt: 'api-sse-browser-viewer-publication',
      timeoutMs: 90_000,
    })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_api_sse_final_tunnel')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: apiSseTunnel.session.id,
        ARROBA_PUBLICATION_ID: apiSseTunnel.publication.id,
      },
      'gateway-api-sse-final-tunnel',
    )
    await waitForGateway(gatewayUrl)
    const registeredApiSseFinalPublication = await waitForRegisteredPublicationEndpoint(
      client,
      apiSseTunnel.session.id,
      apiSseTunnel.publication.id,
      `${gatewayUrl}/`,
      `http://127.0.0.1:${relayPort}/display/publication-`,
    )
    const apiSseFinalTunnelUrl = new URL('invoke', registeredApiSseFinalPublication.open_url).toString()
    const apiSseFinalTunnelResponse = await fetch(apiSseFinalTunnelUrl, {
      method: 'POST',
      headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
      body: JSON.stringify({ prompt: 'api-sse-final-publication-tunnel' }),
    })
    const apiSseFinalTunnelBody = await apiSseFinalTunnelResponse.text()
    const apiSseFinalTunnelEvents = sseEventNames(apiSseFinalTunnelBody)
    if (
      apiSseFinalTunnelResponse.status !== 200
      || !apiSseFinalTunnelEvents.includes('queued')
      || !apiSseFinalTunnelEvents.includes('started')
      || !apiSseFinalTunnelEvents.includes('partial')
      || !apiSseFinalTunnelEvents.includes('trace')
      || !apiSseFinalTunnelEvents.includes('final')
      || (!apiSseFinalTunnelBody.includes('"value":1841') && !apiSseFinalTunnelBody.includes('\\"value\\":1841'))
      || (!apiSseFinalTunnelBody.includes('"value":1842') && !apiSseFinalTunnelBody.includes('\\"value\\":1842'))
    ) {
      const errorIndex = apiSseFinalTunnelBody.lastIndexOf('event: error')
      const diagnostic = errorIndex >= 0 ? apiSseFinalTunnelBody.slice(errorIndex, errorIndex + 800) : apiSseFinalTunnelBody.slice(0, 2_000)
      throw new Error(`expected tunnel API SSE queued/started/partial/final with deterministic output, got ${apiSseFinalTunnelResponse.status} ${JSON.stringify(apiSseFinalTunnelEvents)}: ${diagnostic}`)
    }
    logStep('api_sse_final_tunnel_ok', { events: apiSseFinalTunnelEvents, openUrl: registeredApiSseFinalPublication.open_url })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_websocket_final')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: websocketSession.id,
        ARROBA_PUBLICATION_ID: websocketFinalPublication.id,
      },
      'gateway-websocket-final',
    )
    await waitForGateway(gatewayUrl)
    const webSocketFinal = await invokePublicationWebSocket(
      `ws://127.0.0.1:${gatewayPort}/.well-known/arroba/publication/ws`,
      { prompt: 'websocket-final-publication' },
      { waitForFinal: true },
    )
    const webSocketFinalTypes = webSocketFinal.messages.map((message) => message.type)
    const webSocketFinalBody = JSON.stringify(webSocketFinal.messages)
    if (
      !webSocketFinalTypes.includes('accepted')
      || !webSocketFinalTypes.includes('queued')
      || !webSocketFinalTypes.includes('started')
      || !webSocketFinalTypes.includes('partial')
      || !webSocketFinalTypes.includes('trace')
      || !webSocketFinalTypes.includes('final')
      || (!webSocketFinalBody.includes('"value":1841') && !webSocketFinalBody.includes('\\"value\\":1841'))
      || (!webSocketFinalBody.includes('"value":1842') && !webSocketFinalBody.includes('\\"value\\":1842'))
    ) {
      throw new Error(`expected websocket accepted/queued/started/partial/final with deterministic output, got ${JSON.stringify(webSocketFinal.messages)}`)
    }
    if (!webSocketFinalBody.includes('"agent_alias":"websocket-final"')) {
      throw new Error(`expected websocket trace event tagged with agent alias, got ${webSocketFinalBody.slice(0, 2_000)}`)
    }
    logStep('websocket_final_ok', { events: webSocketFinalTypes })
    await runSharedViewerBrowserDrill({
      baseUrl: `${gatewayUrl}/`,
      root,
      label: 'websocket_shared_viewer',
      prompt: 'websocket-browser-viewer-publication',
      timeoutMs: 90_000,
    })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_websocket_final_tunnel')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: websocketTunnel.session.id,
        ARROBA_PUBLICATION_ID: websocketTunnel.publication.id,
      },
      'gateway-websocket-final-tunnel',
    )
    await waitForGateway(gatewayUrl)
    const registeredWebSocketPublication = await waitForRegisteredPublicationEndpoint(
      client,
      websocketTunnel.session.id,
      websocketTunnel.publication.id,
      `${gatewayUrl}/`,
      `http://127.0.0.1:${relayPort}/display/publication-`,
    )
    const webSocketTunnelUrl = websocketUrlFromHttp(
      new URL('.well-known/arroba/publication/ws', registeredWebSocketPublication.open_url).toString(),
    )
    const webSocketTunnelFinal = await invokePublicationWebSocket(
      webSocketTunnelUrl,
      { prompt: 'websocket-final-publication-tunnel' },
      { waitForFinal: true },
    )
    const webSocketTunnelTypes = webSocketTunnelFinal.messages.map((message) => message.type)
    const webSocketTunnelBody = JSON.stringify(webSocketTunnelFinal.messages)
    if (
      !webSocketTunnelTypes.includes('accepted')
      || !webSocketTunnelTypes.includes('queued')
      || !webSocketTunnelTypes.includes('started')
      || !webSocketTunnelTypes.includes('partial')
      || !webSocketTunnelTypes.includes('trace')
      || !webSocketTunnelTypes.includes('final')
      || (!webSocketTunnelBody.includes('"value":1841') && !webSocketTunnelBody.includes('\\"value\\":1841'))
      || (!webSocketTunnelBody.includes('"value":1842') && !webSocketTunnelBody.includes('\\"value\\":1842'))
    ) {
      throw new Error(`expected tunnel websocket accepted/queued/started/partial/final with deterministic output, got ${JSON.stringify(webSocketTunnelFinal.messages)}`)
    }
    logStep('websocket_final_tunnel_ok', { events: webSocketTunnelTypes, openUrl: registeredWebSocketPublication.open_url })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_human_http_browser_final')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: browserSession.id,
        ARROBA_PUBLICATION_ID: humanHttpFinalPublication.id,
      },
      'gateway-human-http-final',
    )
    await waitForGateway(gatewayUrl)
    await runHumanHttpBrowserDrill({
      url: `${gatewayUrl}/final/browser-final-publication`,
      root,
    })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_human_http_browser_root_form')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: browserRoot.session.id,
        ARROBA_PUBLICATION_ID: browserRoot.publication.id,
      },
      'gateway-human-http-root-form',
    )
    await waitForGateway(gatewayUrl)
    await runHumanHttpRootFormBrowserDrill({
      baseUrl: `${gatewayUrl}/`,
      root,
      timeoutMs: 90_000,
    })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_human_http_html_final_browser')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: browserHtml.session.id,
        ARROBA_PUBLICATION_ID: browserHtml.publication.id,
      },
      'gateway-human-http-html-final',
    )
    await waitForGateway(gatewayUrl)
    await runHumanHttpHtmlFinalBrowserDrill({
      url: `${gatewayUrl}/dashboard/generate%20a%20dashboard%20with%20vibrant%20colours`,
      root,
      timeoutMs: 90_000,
    })
    await stopProcess(gateway)
    gateway = null

    if (browserRealHtml) {
      logStep('invoke_human_http_real_html_final_browser')
      gateway = startProcess(
        process.execPath,
        [path.join(repoRoot, 'apps/server/dist/index.js')],
        {
          ...env,
          HOST: '127.0.0.1',
          PORT: String(gatewayPort),
          ARROBA_KERNEL_URL: kernelUrl,
          ARROBA_PUBLICATION_SESSION_ID: browserRealHtml.session.id,
          ARROBA_PUBLICATION_ID: browserRealHtml.publication.id,
        },
        'gateway-human-http-real-html-final',
      )
      await waitForGateway(gatewayUrl)
      await runHumanHttpHtmlFinalBrowserDrill({
        url: `${gatewayUrl}/real-dashboard/${encodeURIComponent(REAL_DASHBOARD_PROMPT)}`,
        root,
        timeoutMs: 240_000,
        expectedHtmlText: 'Real Provider Workflow Dashboard',
        requiredHtmlSnippets: ['data-arroba-real-provider-dashboard="true"'],
        requiredTraceLevels: browserRealHtml.requiredTraceLevels,
        requiredTraceAlias: browserRealHtml.dashboardAgent.alias ?? browserRealHtml.dashboardAgent.id,
        visualArtifactPrefix: 'local-human-http-real-dashboard',
        requirePartial: false,
      })
      await stopProcess(gateway)
      gateway = null
    }

    logStep('invoke_mcp')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: mcpSession.id,
        ARROBA_PUBLICATION_ID: mcpPublication.id,
      },
      'gateway-mcp',
    )
    await waitForGateway(gatewayUrl)
    const mcpToolsResponse = await fetch(`${gatewayUrl}/mcp`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
    })
    const mcpToolsBody = await mcpToolsResponse.json()
    const mcpToolName = mcpToolsBody.result?.tools?.[0]?.name
    if (mcpToolsResponse.status !== 200 || typeof mcpToolName !== 'string') {
      throw new Error(`expected MCP tools/list to expose publication tool, got ${mcpToolsResponse.status}: ${JSON.stringify(mcpToolsBody)}`)
    }
    const mcpCallResponse = await fetch(`${gatewayUrl}/mcp`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 2,
        method: 'tools/call',
        params: { name: mcpToolName, arguments: { prompt: 'mcp-publication' } },
      }),
    })
    const mcpCallBody = await mcpCallResponse.json()
    const mcpText = mcpCallBody.result?.content?.[0]?.text ?? ''
    if (
      mcpCallResponse.status !== 200
      || mcpCallBody.result?.isError !== false
      || (!mcpText.includes('"value":1842') && !mcpText.includes('\\"value\\":1842'))
    ) {
      throw new Error(`expected MCP tools/call final output, got ${mcpCallResponse.status}: ${JSON.stringify(mcpCallBody).slice(0, 1_200)}`)
    }
    if (!JSON.stringify(mcpCallBody.result?.structuredContent?.traces ?? []).includes('"agent_alias":"mcp-final"')) {
      throw new Error(`expected MCP structuredContent traces tagged with agent alias, got ${JSON.stringify(mcpCallBody).slice(0, 1_200)}`)
    }
    logStep('mcp_ok', { tool: mcpToolName, workflowRunId: mcpCallBody.result?.structuredContent?.workflow_run_id ?? null })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_https')
    const previousTlsReject = process.env.NODE_TLS_REJECT_UNAUTHORIZED
    process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0'
    const secureGatewayEnvs = secureGatewayPublicationEnvs(env, {
      host: '127.0.0.1',
      port: gatewayHttpsPort,
      kernelUrl,
      tls,
      humanHttp: { sessionId: session.id, publicationId: publication.id },
      websocket: { sessionId: websocketSession.id, publicationId: websocketFinalPublication.id },
    })
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      secureGatewayEnvs.https,
      'gateway-https',
    )
    let httpsBody = null
    let httpsResponse = null
    try {
      await waitForGateway(gatewayHttpsUrl)
      httpsResponse = await fetch(`${gatewayHttpsUrl}/qa/ship-publication-secure`)
      httpsBody = await httpsResponse.json()
      if (httpsResponse.status !== 202 || !hasAcceptedRunMetadata(httpsBody)) {
        throw new Error(`expected HTTPS 202 from async publication, got ${httpsResponse.status}: ${JSON.stringify(httpsBody)}`)
      }
      await stopProcess(gateway)
      gateway = startProcess(
        process.execPath,
        [path.join(repoRoot, 'apps/server/dist/index.js')],
        secureGatewayEnvs.wss,
        'gateway-wss',
      )
      await waitForGateway(gatewayHttpsUrl)
      logStep('invoke_wss')
      const wssAccepted = await invokePublicationWebSocket(
        `wss://127.0.0.1:${gatewayHttpsPort}/.well-known/arroba/publication/ws`,
        { task: 'wss-publication' },
        { rejectUnauthorized: false },
      )
      logStep('wss_ok', { workflowRunId: wssAccepted.accepted.workflow_run?.id ?? null, queued: wssAccepted.accepted.queued === true })
    } finally {
      if (previousTlsReject === undefined) delete process.env.NODE_TLS_REJECT_UNAUTHORIZED
      else process.env.NODE_TLS_REJECT_UNAUTHORIZED = previousTlsReject
    }
    await stopProcess(gateway)
    gateway = null

    await runPublicationPackageValidation({
      root,
      client,
      kernelUrl,
      env,
      sessionIds,
      dockerImages,
      dockerContainers,
      relayToken,
      cliBinary,
      gatewayPort,
      gatewayUrl,
      publication,
      workspace,
      workflow,
      session,
      humanHttpFinalPublication,
      browserWorkspace,
      browserSession,
      browserWorkflow,
      apiSseFinalPublication,
      apiSseWorkspace,
      apiSseSession,
      apiSseWorkflow,
      websocketFinalPublication,
      websocketWorkspace,
      websocketSession,
      websocketWorkflow,
      mcpPublication,
      mcpWorkspace,
      mcpSession,
      mcpWorkflow,
      schedulePublication,
      scheduleWorkspace,
      scheduleSession,
      scheduleWorkflow,
      schedule,
    })

    logStep('ok', {
      anonymousPublicationId: publication.id,
      workflowRunId: body.workflow_run?.id ?? null,
    })
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) {
      for (const id of sessionIds.reverse()) {
        await client.send(endSessionRequest(id)).catch(() => {})
      }
    }
    await client?.close?.().catch(() => {})
    await stopProcess(gateway)
    await stopProcess(kernel)
    await stopProcess(relay)
    for (const name of dockerContainers.reverse()) {
      await removeDockerContainer(name).catch(() => {})
    }
    for (const image of dockerImages.reverse()) {
      await removeDockerImage(image).catch(() => {})
    }
    if (!succeeded) {
      console.error('[publication-drill] relay logs', relay?.logs ?? null)
      console.error('[publication-drill] kernel logs', kernel?.logs ?? null)
      console.error('[publication-drill] gateway logs', gateway?.logs ?? null)
    }
    await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: 'live-workflow-publication',
        relayUrl,
        kernelUrl,
        gatewayUrl,
        gatewayHttpsUrl,
        daemonAlias,
        sessionCount: sessionIds.length,
        remainingDockerContainers: dockerContainers.length,
        remainingDockerImages: dockerImages.length,
        realDashboardEnabled: realDashboard != null,
        containerDrillEnabled: envFlag('ARROBA_PUBLICATION_CONTAINER_DRILL'),
        processLogs: {
          relayStdout: tail(relay?.logs?.stdout),
          relayStderr: tail(relay?.logs?.stderr),
          kernelStdout: tail(kernel?.logs?.stdout),
          kernelStderr: tail(kernel?.logs?.stderr),
          gatewayStdout: tail(gateway?.logs?.stdout),
          gatewayStderr: tail(gateway?.logs?.stderr),
        },
      },
    })
  }
}
