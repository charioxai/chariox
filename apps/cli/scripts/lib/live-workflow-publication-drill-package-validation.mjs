import path from 'node:path'
import { readFile, writeFile } from 'node:fs/promises'
const { createDefaultShellContext, parseShellCommand } = await import('../../../../packages/kernel-client/dist/shell-core.js')
const { executeShellCommand } = await import('../../../../packages/kernel-client/dist/shell-executor.js')
const { endSessionRequest, getSessionStateRequest } = await import('../../../../packages/kernel-client/dist/ipc-requests.js')
import { hasAcceptedRunMetadata, logStep, publicationStatusWatchdogCount, publicationStatusWatchdogs, repoRoot, run, startProcess, startServeWithProviderPrompt, stopProcess, variant, waitForProcessExit } from './live-workflow-publication-drill-runtime.mjs'
import { assertGatewayDoesNotListen, assertPackageDoesNotContain, assertPublicationRuntimeSessionHidden, createUnavailableProviderPackage, waitForGateway, waitForPublicationStatusLatestOutput, waitForScheduledWorkflowRun } from './live-workflow-publication-drill-waiters.mjs'
import { runContainerPublicationValidation } from './live-workflow-publication-drill-containers.mjs'

export async function runPublicationPackageValidation({
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
}) {
  let gateway = null
  try {
    logStep('export_publication_package')
    const exportDir = path.join(root, 'exported-publication')
    const exportResult = await executeShellCommand(
      parseShellCommand(`workflow publication export ${publication.id} ${exportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({
        workspace,
        worktree: workspace,
        sessionId: session.id,
        workflowId: workflow.id,
      }),
      { client },
    )
    if (!exportResult.ok) {
      throw new Error(`publication export failed: ${exportResult.message}`)
    }
    await assertPackageDoesNotContain(exportDir, [
      relayToken,
    ])
    gateway = startProcess(
      cliBinary,
      ['serve', exportDir, String(gatewayPort), '--kernel-url', kernelUrl],
      {
        ...env,
        HOST: '127.0.0.1',
      },
      'arroba-serve-exported',
    )
    await waitForPackageGateway(gatewayUrl, gateway)
    const exportedStatusResponse = await fetch(`${gatewayUrl}/.well-known/arroba/publication/status`)
    const exportedStatusBody = await exportedStatusResponse.json()
    const exportedRuntimeSessionId = exportedStatusBody.runtime_session_id
    if (exportedStatusResponse.status !== 200 || typeof exportedRuntimeSessionId !== 'string') {
      throw new Error(`expected exported publication status with runtime session id, got ${exportedStatusResponse.status}: ${JSON.stringify(exportedStatusBody)}`)
    }
    sessionIds.push(exportedRuntimeSessionId)
    await assertPublicationRuntimeSessionHidden(client, exportedRuntimeSessionId)
    const exportedResponse = await fetch(`${gatewayUrl}/qa/exported-publication`)
    const exportedBody = await exportedResponse.json()
    if (exportedResponse.status !== 202 || !hasAcceptedRunMetadata(exportedBody)) {
      throw new Error(`expected exported package HTTP 202, got ${exportedResponse.status}: ${JSON.stringify(exportedBody)}`)
    }
    await stopProcess(gateway)
    gateway = null
    await client.send(endSessionRequest(exportedRuntimeSessionId)).catch(() => {})

    logStep('invoke_ipc_exported')
    const ipcResult = await run(process.execPath, [
      path.join(repoRoot, 'apps/server/dist/workflow-call.js'),
      '--package',
      path.join(exportDir, 'publication.json'),
      '--input',
      JSON.stringify({ task: 'ipc-exported-publication' }),
      '--mode',
      'async',
    ], { env })
    if (ipcResult.code !== 0) {
      throw new Error(`expected IPC exported invocation to succeed\nstdout:\n${ipcResult.stdout}\nstderr:\n${ipcResult.stderr}`)
    }
    const ipcBody = JSON.parse(ipcResult.stdout)
    if (!ipcBody.accepted || !hasAcceptedRunMetadata(ipcBody)) {
      throw new Error(`expected IPC accepted run metadata, got ${ipcResult.stdout}`)
    }
    if (typeof ipcBody.runtime_session_id !== 'string') {
      throw new Error(`expected IPC output to include runtime_session_id, got ${ipcResult.stdout}`)
    }
    sessionIds.push(ipcBody.runtime_session_id)
    await assertPublicationRuntimeSessionHidden(client, ipcBody.runtime_session_id)
    await client.send(endSessionRequest(ipcBody.runtime_session_id)).catch(() => {})

    logStep('serve_provider_override_prompt')
    const overrideExportDir = path.join(root, 'exported-publication-provider-override')
    const overridePackage = await createUnavailableProviderPackage(exportDir, overrideExportDir)
    gateway = startServeWithProviderPrompt({
      cliBinary,
      packageDir: overrideExportDir,
      port: gatewayPort,
      kernelUrl,
      env: {
        ...env,
        HOST: '127.0.0.1',
      },
      provider: 'dev-stub',
      model: 'publication-drill-model',
      effort: 'low',
    })
    await waitForPackageGateway(gatewayUrl, gateway)
    const overrideStatusResponse = await fetch(`${gatewayUrl}/.well-known/arroba/publication/status`)
    const overrideStatusBody = await overrideStatusResponse.json()
    const overrideRuntimeSessionId = overrideStatusBody.runtime_session_id
    if (overrideStatusResponse.status !== 200 || typeof overrideRuntimeSessionId !== 'string') {
      throw new Error(`expected override publication status with runtime session id, got ${overrideStatusResponse.status}: ${JSON.stringify(overrideStatusBody)}`)
    }
    sessionIds.push(overrideRuntimeSessionId)
    await assertPublicationRuntimeSessionHidden(client, overrideRuntimeSessionId)
    const overrideBindings = JSON.parse(await readFile(path.join(overrideExportDir, 'bindings.local.json'), 'utf8'))
    const overrideReplacement = overrideBindings.provider_model_overrides
      ?.find((candidate) => candidate.agent_id === overridePackage.agentId)
      ?.replacement
    if (
      overrideReplacement?.provider !== 'dev-stub'
      || overrideReplacement?.model !== 'publication-drill-model'
      || overrideReplacement?.effort !== 'low'
    ) {
      throw new Error(`expected provider override to persist dev-stub/publication-drill-model/low, got ${JSON.stringify(overrideBindings)}`)
    }
    const overrideResponse = await fetch(`${gatewayUrl}/qa/provider-override-publication`)
    const overrideBody = await overrideResponse.json()
    if (overrideResponse.status !== 202 || !hasAcceptedRunMetadata(overrideBody)) {
      throw new Error(`expected provider override package HTTP 202, got ${overrideResponse.status}: ${JSON.stringify(overrideBody)}`)
    }
    if (
      !gateway.logs.stdout.includes('Replacement provider:')
      || !gateway.logs.stdout.includes('Replacement model')
      || !gateway.logs.stdout.includes('Replacement effort')
    ) {
      throw new Error(`expected provider override prompt transcript, got stdout:\n${gateway.logs.stdout}\nstderr:\n${gateway.logs.stderr}`)
    }
    await stopProcess(gateway)
    gateway = null
    await client.send(endSessionRequest(overrideRuntimeSessionId)).catch(() => {})
    logStep('serve_provider_override_prompt_ok', { agentId: overridePackage.agentId })
    await client.send(endSessionRequest(session.id)).catch(() => {})

    logStep('schedule_publication_export')
    const scheduleExportDir = path.join(root, 'exported-schedule-publication')
    const scheduleExportResult = await executeShellCommand(
      parseShellCommand(`workflow publication export ${schedulePublication.id} ${scheduleExportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({
        workspace: scheduleWorkspace,
        worktree: scheduleWorkspace,
        sessionId: scheduleSession.id,
        workflowId: scheduleWorkflow.id,
      }),
      { client },
    )
    if (!scheduleExportResult.ok) {
      throw new Error(`schedule publication export failed: ${scheduleExportResult.message}`)
    }
    const scheduleSnapshot = JSON.parse(await readFile(path.join(scheduleExportDir, 'workflow.snapshot.json'), 'utf8'))
    if (scheduleSnapshot.schedules?.[0]?.id !== schedule.id) {
      throw new Error(`expected exported schedule ${schedule.id}, got ${JSON.stringify(scheduleSnapshot.schedules)}`)
    }
    scheduleSnapshot.schedules[0].next_run_at_ms = 0
    await writeFile(path.join(scheduleExportDir, 'workflow.snapshot.json'), `${JSON.stringify(scheduleSnapshot, null, 2)}\n`)
    gateway = startProcess(
      cliBinary,
      ['serve', scheduleExportDir, String(gatewayPort), '--kernel-url', kernelUrl],
      {
        ...env,
        HOST: '127.0.0.1',
      },
      'arroba-serve-schedule',
    )
    await waitForPackageGateway(gatewayUrl, gateway)
    const statusResponse = await fetch(`${gatewayUrl}/.well-known/arroba/publication/status`)
    const statusBody = await statusResponse.json()
    const runtimeSessionId = statusBody.runtime_session_id
    if (statusResponse.status !== 200 || typeof runtimeSessionId !== 'string') {
      throw new Error(`expected publication status with runtime session id, got ${statusResponse.status}: ${JSON.stringify(statusBody)}`)
    }
    const statusWatchdogs = publicationStatusWatchdogs(statusBody)
    if (publicationStatusWatchdogCount(statusBody) !== 1 || statusWatchdogs[0]?.id !== schedule.id) {
      throw new Error(`expected publication status to expose schedule ${schedule.id}, got ${JSON.stringify(statusBody)}`)
    }
    if (statusWatchdogs[0].last_status !== 'warming_up') {
      throw new Error(`expected publication schedule ${schedule.id} to honor materialization warm-up, got ${JSON.stringify(statusWatchdogs[0])}`)
    }
    sessionIds.push(runtimeSessionId)
    await assertPublicationRuntimeSessionHidden(client, runtimeSessionId)
    const scheduleRuntimeSession = variant(
      await client.send(getSessionStateRequest(runtimeSessionId)),
      'SessionState',
    ).session
    const scheduleRuntimeWorkspacePrefix = `${scheduleExportDir}.runtime-`
    if (
      !scheduleRuntimeSession.workspace_id?.startsWith(scheduleRuntimeWorkspacePrefix)
      || scheduleRuntimeSession.worktree_id !== scheduleRuntimeSession.workspace_id
      || scheduleRuntimeSession.workspace_id === scheduleWorkspace
    ) {
      throw new Error(`expected isolated schedule package runtime workspace under ${scheduleRuntimeWorkspacePrefix}, got ${JSON.stringify({
        workspace_id: scheduleRuntimeSession.workspace_id,
        worktree_id: scheduleRuntimeSession.worktree_id,
      })}`)
    }
    const scheduleRun = await waitForScheduledWorkflowRun(client, runtimeSessionId, scheduleWorkflow.id, {
      requireOutput: true,
      timeoutMs: 360_000,
    })
    const statusAfterRun = await waitForPublicationStatusLatestOutput(gatewayUrl, scheduleRun.final_output?.message)
    logStep('schedule_publication_ok', {
      runtimeSessionId,
      workflowRunId: scheduleRun.id,
      status: scheduleRun.status,
      latestOutput: statusAfterRun.latest_output?.message,
    })
    await stopProcess(gateway)
    gateway = null

    await runContainerPublicationValidation({
      enabled: process.env.ARROBA_PUBLICATION_CONTAINER_DRILL === '1',
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
    })

    logStep('missing_requirements_fail_before_listen')
    await writeFile(path.join(exportDir, 'requirements.json'), JSON.stringify({
      schema_version: 1,
      skills: [{ name: 'missing-publication-skill' }],
      credentials: [{ name: 'missing-publication-credential' }],
    }, null, 2))
    gateway = startProcess(
      cliBinary,
      ['serve', exportDir, String(gatewayPort), '--kernel-url', kernelUrl],
      {
        ...env,
        HOST: '127.0.0.1',
      },
      'arroba-serve-missing-requirements',
    )
    const failedServe = await waitForProcessExit(gateway)
    await assertGatewayDoesNotListen(gatewayUrl)
    if (failedServe.code === 0) {
      throw new Error('expected missing-requirements serve to fail')
    }
    if (!/publication requirements are missing: skill:missing-publication-skill, credential:missing-publication-credential/.test(gateway.logs.stderr)) {
      throw new Error(`expected missing-requirements error, got stderr:\n${gateway.logs.stderr}`)
    }
    gateway = null
  } finally {
    await stopProcess(gateway).catch(() => {})
  }
}

async function waitForPackageGateway(baseUrl, gateway) {
  try {
    await waitForGateway(baseUrl)
  } catch (error) {
    throw new Error([
      `publication package gateway did not become ready: ${error instanceof Error ? error.message : String(error)}`,
      `process=${gateway?.name ?? 'unknown'} exit=${gateway?.exitCode ?? 'running'}`,
      `stdout:\n${gateway?.logs?.stdout ?? ''}`,
      `stderr:\n${gateway?.logs?.stderr ?? ''}`,
    ].join('\n'))
  }
}
