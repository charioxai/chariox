import path from 'node:path'
import { writeFile } from 'node:fs/promises'
const { createDefaultShellContext, parseShellCommand } = await import('../../../../packages/kernel-client/dist/shell-core.js')
const { executeShellCommand } = await import('../../../../packages/kernel-client/dist/shell-executor.js')
import { buildPublicationContainerImage, createContainerPortablePackage, freePort, logStep, publicationStatusWatchdogCount, publicationStatusWatchdogs, removeDockerContainer, sseEventNames, startPublicationContainer, stopProcess, waitForProcessExit } from './live-workflow-publication-drill-runtime.mjs'
import { invokePublicationWebSocket } from './live-workflow-publication-drill-runtime.mjs'
import { runHumanHttpBrowserDrill, runHumanHttpRootFormBrowserDrill } from './live-workflow-publication-drill-browser.mjs'
import { assertGatewayDoesNotListen, waitForContainerGateway, waitForPublicationStatusLatestOutput } from './live-workflow-publication-drill-waiters.mjs'

export async function runContainerPublicationValidation({
  enabled,
  root,
  client,
  kernelUrl,
  env,
  dockerImages,
  dockerContainers,
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
  scheduleExportDir,
  scheduleWorkspace,
  schedule,
  scheduleRun,
}) {
  if (enabled) {
    logStep('container_publication_build')
    const publicationContainerImage = `arroba-publication-drill:${process.pid}`
    dockerImages.push(publicationContainerImage)
    await buildPublicationContainerImage(publicationContainerImage)

    logStep('container_human_http_export')
    const humanHttpContainerExportDir = path.join(root, 'container-human-http-package')
    const humanHttpContainerPackageDir = path.join(root, 'container-human-http-portable')
    const humanHttpContainerExportResult = await executeShellCommand(
      parseShellCommand(`workflow publication export ${humanHttpFinalPublication.id} ${humanHttpContainerExportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({
        workspace: browserWorkspace,
        worktree: browserWorkspace,
        sessionId: browserSession.id,
        workflowId: browserWorkflow.id,
      }),
      { client },
    )
    if (!humanHttpContainerExportResult.ok) {
      throw new Error(`human_http container publication export failed: ${humanHttpContainerExportResult.message}`)
    }
    await createContainerPortablePackage(humanHttpContainerExportDir, humanHttpContainerPackageDir)
    const containerHumanPort = await freePort()
    const containerHumanUrl = `http://127.0.0.1:${containerHumanPort}`
    const containerHumanName = `arroba-publication-human-${process.pid}`
    dockerContainers.push(containerHumanName)
    let containerHumanPromptRuntimeSessionId = null
    let containerProcess = startPublicationContainer({
      image: publicationContainerImage,
      name: containerHumanName,
      packageDir: humanHttpContainerPackageDir,
      workspaceDir: browserWorkspace,
      port: containerHumanPort,
    })
    try {
      await waitForContainerGateway(containerHumanUrl, containerProcess, 60_000)
      const containerStatusResponse = await fetch(`${containerHumanUrl}/.well-known/arroba/publication/status`)
      const containerStatusBody = await containerStatusResponse.json()
      if (containerStatusResponse.status !== 200 || typeof containerStatusBody.runtime_session_id !== 'string') {
        throw new Error(`expected container human_http status with runtime session id, got ${containerStatusResponse.status}: ${JSON.stringify(containerStatusBody)}`)
      }
      containerHumanPromptRuntimeSessionId = containerStatusBody.runtime_session_id
      const containerHumanResponse = await fetch(`${containerHumanUrl}/`, {
        headers: { accept: 'text/html' },
      })
      const containerHumanBody = await containerHumanResponse.text()
      if (
        containerHumanResponse.status !== 200
        || !containerHumanBody.includes('invoke-form')
        || !containerHumanBody.includes('type="file" name="artifact" multiple')
        || !containerHumanBody.includes('/final/')
      ) {
        throw new Error(`expected container human_http root form with prompt and artifact upload, got ${containerHumanResponse.status}: ${containerHumanBody.slice(0, 1_000)}`)
      }
      await runHumanHttpBrowserDrill({
        url: `${containerHumanUrl}/final/container-human-http-browser`,
        root,
        timeoutMs: 90_000,
      })
      logStep('container_human_http_prompt_url_ok', { runtimeSessionId: containerStatusBody.runtime_session_id })
    } finally {
      await stopProcess(containerProcess)
      await removeDockerContainer(containerHumanName).catch(() => {})
      containerProcess = null
    }
    const containerHumanRootPort = await freePort()
    const containerHumanRootUrl = `http://127.0.0.1:${containerHumanRootPort}`
    const containerHumanRootName = `arroba-publication-human-root-${process.pid}`
    dockerContainers.push(containerHumanRootName)
    containerProcess = startPublicationContainer({
      image: publicationContainerImage,
      name: containerHumanRootName,
      packageDir: humanHttpContainerPackageDir,
      workspaceDir: browserWorkspace,
      port: containerHumanRootPort,
    })
    try {
      await waitForContainerGateway(containerHumanRootUrl, containerProcess, 60_000)
      const containerStatusResponse = await fetch(`${containerHumanRootUrl}/.well-known/arroba/publication/status`)
      const containerStatusBody = await containerStatusResponse.json()
      if (containerStatusResponse.status !== 200 || typeof containerStatusBody.runtime_session_id !== 'string') {
        throw new Error(`expected container human_http root status with runtime session id, got ${containerStatusResponse.status}: ${JSON.stringify(containerStatusBody)}`)
      }
      await runHumanHttpRootFormBrowserDrill({
        baseUrl: `${containerHumanRootUrl}/`,
        root,
        timeoutMs: 90_000,
      })
      logStep('container_human_http_ok', {
        promptRuntimeSessionId: containerHumanPromptRuntimeSessionId,
        rootFormRuntimeSessionId: containerStatusBody.runtime_session_id,
      })
    } finally {
      await stopProcess(containerProcess)
      await removeDockerContainer(containerHumanRootName).catch(() => {})
      containerProcess = null
    }

    logStep('container_api_sse_export')
    const apiSseContainerExportDir = path.join(root, 'container-api-sse-package')
    const apiSseContainerPackageDir = path.join(root, 'container-api-sse-portable')
    const apiSseContainerExportResult = await executeShellCommand(
      parseShellCommand(`workflow publication export ${apiSseFinalPublication.id} ${apiSseContainerExportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({
        workspace: apiSseWorkspace,
        worktree: apiSseWorkspace,
        sessionId: apiSseSession.id,
        workflowId: apiSseWorkflow.id,
      }),
      { client },
    )
    if (!apiSseContainerExportResult.ok) {
      throw new Error(`api_sse_json container publication export failed: ${apiSseContainerExportResult.message}`)
    }
    await createContainerPortablePackage(apiSseContainerExportDir, apiSseContainerPackageDir)
    const containerApiPort = await freePort()
    const containerApiUrl = `http://127.0.0.1:${containerApiPort}`
    const containerApiName = `arroba-publication-api-${process.pid}`
    dockerContainers.push(containerApiName)
    containerProcess = startPublicationContainer({
      image: publicationContainerImage,
      name: containerApiName,
      packageDir: apiSseContainerPackageDir,
      workspaceDir: apiSseWorkspace,
      port: containerApiPort,
    })
    try {
      await waitForContainerGateway(containerApiUrl, containerProcess, 60_000)
      const containerApiStatusResponse = await fetch(`${containerApiUrl}/.well-known/arroba/publication/status`)
      const containerApiStatusBody = await containerApiStatusResponse.json()
      if (containerApiStatusResponse.status !== 200 || typeof containerApiStatusBody.runtime_session_id !== 'string') {
        throw new Error(`expected container api_sse_json status with runtime session id, got ${containerApiStatusResponse.status}: ${JSON.stringify(containerApiStatusBody)}`)
      }
      const containerApiResponse = await fetch(`${containerApiUrl}/invoke`, {
        method: 'POST',
        headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
        body: JSON.stringify({ prompt: 'container-api-sse-publication' }),
      })
      const containerApiBody = await containerApiResponse.text()
      const containerApiEvents = sseEventNames(containerApiBody)
      if (
        containerApiResponse.status !== 200
        || !containerApiEvents.includes('queued')
        || !containerApiEvents.includes('started')
        || !containerApiEvents.includes('partial')
        || !containerApiEvents.includes('final')
        || (!containerApiBody.includes('"value":1841') && !containerApiBody.includes('\\"value\\":1841'))
        || (!containerApiBody.includes('"value":1842') && !containerApiBody.includes('\\"value\\":1842'))
      ) {
        throw new Error(`expected container API SSE queued/started/partial/final with deterministic output, got ${containerApiResponse.status} ${JSON.stringify(containerApiEvents)}: ${containerApiBody.slice(0, 2_000)}`)
      }
      logStep('container_api_sse_ok', { runtimeSessionId: containerApiStatusBody.runtime_session_id, events: containerApiEvents })
    } finally {
      await stopProcess(containerProcess)
      await removeDockerContainer(containerApiName).catch(() => {})
      containerProcess = null
    }

    logStep('container_websocket_export')
    const websocketContainerExportDir = path.join(root, 'container-websocket-package')
    const websocketContainerPackageDir = path.join(root, 'container-websocket-portable')
    const websocketContainerExportResult = await executeShellCommand(
      parseShellCommand(`workflow publication export ${websocketFinalPublication.id} ${websocketContainerExportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({
        workspace: websocketWorkspace,
        worktree: websocketWorkspace,
        sessionId: websocketSession.id,
        workflowId: websocketWorkflow.id,
      }),
      { client },
    )
    if (!websocketContainerExportResult.ok) {
      throw new Error(`websocket_json container publication export failed: ${websocketContainerExportResult.message}`)
    }
    await createContainerPortablePackage(websocketContainerExportDir, websocketContainerPackageDir)
    const containerWebSocketPort = await freePort()
    const containerWebSocketUrl = `http://127.0.0.1:${containerWebSocketPort}`
    const containerWebSocketName = `arroba-publication-websocket-${process.pid}`
    dockerContainers.push(containerWebSocketName)
    containerProcess = startPublicationContainer({
      image: publicationContainerImage,
      name: containerWebSocketName,
      packageDir: websocketContainerPackageDir,
      workspaceDir: websocketWorkspace,
      port: containerWebSocketPort,
    })
    try {
      await waitForContainerGateway(containerWebSocketUrl, containerProcess, 60_000)
      const containerWebSocketStatusResponse = await fetch(`${containerWebSocketUrl}/.well-known/arroba/publication/status`)
      const containerWebSocketStatusBody = await containerWebSocketStatusResponse.json()
      if (containerWebSocketStatusResponse.status !== 200 || typeof containerWebSocketStatusBody.runtime_session_id !== 'string') {
        throw new Error(`expected container websocket_json status with runtime session id, got ${containerWebSocketStatusResponse.status}: ${JSON.stringify(containerWebSocketStatusBody)}`)
      }
      const containerWebSocket = await invokePublicationWebSocket(
        `ws://127.0.0.1:${containerWebSocketPort}/.well-known/arroba/publication/ws`,
        { prompt: 'container-websocket-publication' },
        { waitForFinal: true },
      )
      const containerWebSocketTypes = containerWebSocket.messages.map((message) => message.type)
      const containerWebSocketBody = JSON.stringify(containerWebSocket.messages)
      if (
        !containerWebSocketTypes.includes('accepted')
        || !containerWebSocketTypes.includes('queued')
        || !containerWebSocketTypes.includes('started')
        || !containerWebSocketTypes.includes('partial')
        || !containerWebSocketTypes.includes('final')
        || (!containerWebSocketBody.includes('"value":1841') && !containerWebSocketBody.includes('\\"value\\":1841'))
        || (!containerWebSocketBody.includes('"value":1842') && !containerWebSocketBody.includes('\\"value\\":1842'))
      ) {
        throw new Error(`expected container websocket_json accepted/queued/started/partial/final with deterministic output, got ${JSON.stringify(containerWebSocket.messages)}`)
      }
      logStep('container_websocket_ok', { runtimeSessionId: containerWebSocketStatusBody.runtime_session_id, events: containerWebSocketTypes })
    } finally {
      await stopProcess(containerProcess)
      await removeDockerContainer(containerWebSocketName).catch(() => {})
      containerProcess = null
    }

    logStep('container_mcp_export')
    const mcpContainerExportDir = path.join(root, 'container-mcp-package')
    const mcpContainerPackageDir = path.join(root, 'container-mcp-portable')
    const mcpContainerExportResult = await executeShellCommand(
      parseShellCommand(`workflow publication export ${mcpPublication.id} ${mcpContainerExportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({
        workspace: mcpWorkspace,
        worktree: mcpWorkspace,
        sessionId: mcpSession.id,
        workflowId: mcpWorkflow.id,
      }),
      { client },
    )
    if (!mcpContainerExportResult.ok) {
      throw new Error(`mcp container publication export failed: ${mcpContainerExportResult.message}`)
    }
    await createContainerPortablePackage(mcpContainerExportDir, mcpContainerPackageDir)
    const containerMcpPort = await freePort()
    const containerMcpUrl = `http://127.0.0.1:${containerMcpPort}`
    const containerMcpName = `arroba-publication-mcp-${process.pid}`
    dockerContainers.push(containerMcpName)
    containerProcess = startPublicationContainer({
      image: publicationContainerImage,
      name: containerMcpName,
      packageDir: mcpContainerPackageDir,
      workspaceDir: mcpWorkspace,
      port: containerMcpPort,
    })
    try {
      await waitForContainerGateway(containerMcpUrl, containerProcess, 60_000)
      const containerMcpStatusResponse = await fetch(`${containerMcpUrl}/.well-known/arroba/publication/status`)
      const containerMcpStatusBody = await containerMcpStatusResponse.json()
      if (containerMcpStatusResponse.status !== 200 || typeof containerMcpStatusBody.runtime_session_id !== 'string') {
        throw new Error(`expected container mcp status with runtime session id, got ${containerMcpStatusResponse.status}: ${JSON.stringify(containerMcpStatusBody)}`)
      }
      const containerMcpToolsResponse = await fetch(`${containerMcpUrl}/mcp`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
      })
      const containerMcpToolsBody = await containerMcpToolsResponse.json()
      const containerMcpToolName = containerMcpToolsBody.result?.tools?.[0]?.name
      if (containerMcpToolsResponse.status !== 200 || typeof containerMcpToolName !== 'string') {
        throw new Error(`expected container MCP tools/list to expose publication tool, got ${containerMcpToolsResponse.status}: ${JSON.stringify(containerMcpToolsBody)}`)
      }
      const containerMcpCallResponse = await fetch(`${containerMcpUrl}/mcp`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 2,
          method: 'tools/call',
          params: { name: containerMcpToolName, arguments: { prompt: 'container-mcp-publication' } },
        }),
      })
      const containerMcpCallBody = await containerMcpCallResponse.json()
      const containerMcpText = containerMcpCallBody.result?.content?.[0]?.text ?? ''
      if (
        containerMcpCallResponse.status !== 200
        || containerMcpCallBody.result?.isError !== false
        || (!containerMcpText.includes('"value":1842') && !containerMcpText.includes('\\"value\\":1842'))
      ) {
        throw new Error(`expected container MCP tools/call final output, got ${containerMcpCallResponse.status}: ${JSON.stringify(containerMcpCallBody).slice(0, 1_200)}`)
      }
      logStep('container_mcp_ok', { runtimeSessionId: containerMcpStatusBody.runtime_session_id, tool: containerMcpToolName })
    } finally {
      await stopProcess(containerProcess)
      await removeDockerContainer(containerMcpName).catch(() => {})
      containerProcess = null
    }

    logStep('container_schedule_export')
    const scheduleContainerPackageDir = path.join(root, 'container-schedule-portable')
    await createContainerPortablePackage(scheduleExportDir, scheduleContainerPackageDir)
    const containerSchedulePort = await freePort()
    const containerScheduleUrl = `http://127.0.0.1:${containerSchedulePort}`
    const containerScheduleName = `arroba-publication-schedule-${process.pid}`
    dockerContainers.push(containerScheduleName)
    containerProcess = startPublicationContainer({
      image: publicationContainerImage,
      name: containerScheduleName,
      packageDir: scheduleContainerPackageDir,
      workspaceDir: scheduleWorkspace,
      port: containerSchedulePort,
    })
    try {
      await waitForContainerGateway(containerScheduleUrl, containerProcess, 60_000)
      const containerScheduleStatusResponse = await fetch(`${containerScheduleUrl}/.well-known/arroba/publication/status`)
      const containerScheduleStatusBody = await containerScheduleStatusResponse.json()
      const containerScheduleWatchdogs = publicationStatusWatchdogs(containerScheduleStatusBody)
      if (
        containerScheduleStatusResponse.status !== 200
        || typeof containerScheduleStatusBody.runtime_session_id !== 'string'
        || publicationStatusWatchdogCount(containerScheduleStatusBody) !== 1
        || containerScheduleWatchdogs[0]?.id !== schedule.id
      ) {
        throw new Error(`expected container schedule status with runtime session and schedule, got ${containerScheduleStatusResponse.status}: ${JSON.stringify(containerScheduleStatusBody)}`)
      }
      const containerScheduleOutput = await waitForPublicationStatusLatestOutput(
        containerScheduleUrl,
        scheduleRun.final_output?.message,
      )
      logStep('container_schedule_ok', {
        runtimeSessionId: containerScheduleStatusBody.runtime_session_id,
        latestOutput: containerScheduleOutput.latest_output?.message,
      })
    } finally {
      await stopProcess(containerProcess)
      await removeDockerContainer(containerScheduleName).catch(() => {})
      containerProcess = null
    }

    logStep('container_missing_requirements_fail_before_listen')
    const missingContainerPackageDir = path.join(root, 'container-missing-requirements')
    await createContainerPortablePackage(apiSseContainerExportDir, missingContainerPackageDir)
    await writeFile(path.join(missingContainerPackageDir, 'requirements.json'), JSON.stringify({
      schema_version: 1,
      skills: [{ name: 'missing-container-publication-skill' }],
      credentials: [{ name: 'missing-container-publication-credential' }],
    }, null, 2))
    const containerMissingPort = await freePort()
    const containerMissingUrl = `http://127.0.0.1:${containerMissingPort}`
    const containerMissingName = `arroba-publication-missing-${process.pid}`
    dockerContainers.push(containerMissingName)
    containerProcess = startPublicationContainer({
      image: publicationContainerImage,
      name: containerMissingName,
      packageDir: missingContainerPackageDir,
      workspaceDir: apiSseWorkspace,
      port: containerMissingPort,
    })
    const failedContainerServe = await waitForProcessExit(containerProcess, 60_000)
    await assertGatewayDoesNotListen(containerMissingUrl)
    if (failedContainerServe.code === 0) {
      throw new Error('expected container missing-requirements serve to fail')
    }
    if (!/publication requirements are missing: skill:missing-container-publication-skill, credential:missing-container-publication-credential/.test(containerProcess.logs.stderr)) {
      throw new Error(`expected container missing-requirements error, got stdout:\n${containerProcess.logs.stdout}\nstderr:\n${containerProcess.logs.stderr}`)
    }
    await removeDockerContainer(containerMissingName).catch(() => {})
    containerProcess = null
    logStep('container_missing_requirements_fail_before_listen_ok')
  } else {
    logStep('container_publication_skipped', { reason: 'set ARROBA_PUBLICATION_CONTAINER_DRILL=1 to run Docker container validation' })
  }
}
