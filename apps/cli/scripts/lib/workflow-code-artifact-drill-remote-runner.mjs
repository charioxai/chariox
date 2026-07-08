export function remoteWorkflowCodeRunnerSource() {
  return String.raw`#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { closeSync, openSync, readFileSync } from 'node:fs'
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

export function parseArgs(argv) {
  const options = { repo: null, bundle: null, timeoutMs: 120_000 }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--repo') options.repo = argv[++index]
    else if (arg === '--bundle') options.bundle = argv[++index]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else throw new Error('unknown option: ' + arg)
  }
  if (!options.repo || !options.bundle) throw new Error('--repo and --bundle are required')
  return options
}

export function assert(condition, message, details) {
  if (!condition) throw new Error(message + (details ? '\n' + JSON.stringify(details, null, 2) : ''))
}

export function unwrap(response, key) {
  return response?.[key] ?? response
}

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const sha256Hex = (value) => createHash('sha256').update(value).digest('hex')

export function stage(message) {
  console.error('[workflow-code-hetzner] ' + message)
}

export function expectationFromDefinition(definition) {
  return {
    nodes: definition.nodes?.length ?? 0,
    agents: definition.nodes?.length ?? 0,
    edges: definition.edges?.length ?? 0,
    endpoints: definition.endpoints?.length ?? 0,
    queues: (definition.queues?.length ?? 0) || 1,
    schedules: definition.schedules?.length ?? definition.watchdogs?.length ?? 0,
    requiredSchemas: (definition.schemas ?? []).map((schema) => schema.handle),
  }
}

export function rebindingsForDefinition(definition) {
  return (definition.nodes ?? []).map((node) => ({ node: node.handle, provider: 'dev-stub', model: 'default' }))
}

export function validateApplyResult(result, label, expected) {
  assert(result?.compile?.validation?.ok, label + ' compile validation failed', result?.compile?.validation)
  const apply = result.apply
  assert(apply?.workflow_id, label + ' did not return workflow id', result)
  assert(Object.keys(apply.node_ids ?? {}).length === expected.nodes, label + ' node count mismatch', apply)
  assert(Object.keys(apply.agent_ids ?? {}).length === expected.agents, label + ' agent count mismatch', apply)
  assert(Object.keys(apply.edge_ids ?? {}).length === expected.edges, label + ' edge count mismatch', apply)
  assert(Object.keys(apply.endpoint_ids ?? {}).length === expected.endpoints, label + ' endpoint count mismatch', apply)
  assert(Object.keys(apply.queue_ids ?? {}).length === expected.queues, label + ' queue count mismatch', apply)
  assert(Object.keys(apply.schedule_ids ?? apply.watchdog_ids ?? {}).length === expected.schedules, label + ' schedule count mismatch', apply)
  for (const schemaHandle of expected.requiredSchemas) {
    assert(apply.schema_refs?.[schemaHandle], label + ' missing schema ' + schemaHandle, apply)
  }
  assert(apply.canvas_layout_applied === true, label + ' should apply canvas layout', apply)
  return apply
}

export function validateSessionProjection(session, apply, label) {
  const workflow = (session.workflows ?? []).find((entry) => entry.id === apply.workflow_id)
  assert(workflow, label + ' workflow should appear in session projection', { workflowId: apply.workflow_id })
  assert(workflow.canvas_layout, label + ' workflow should include canvas layout', workflow)
  return workflow
}

export async function waitForKernel(client, requests, workspace, worktree, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const session = unwrap(
        await withTimeout(
          client.send(requests.createSessionRequest(workspace, worktree, 'workflow-code-hetzner-ready-probe', undefined, null, 'off')),
          3_000,
          'kernel readiness request timed out',
        ),
        'SessionCreated',
      ).session
      await client.send(requests.endSessionRequest(session.id)).catch(() => {})
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error('kernel did not become ready: ' + (lastError?.message ?? 'unknown error'))
}

export function withTimeout(promise, timeoutMs, message) {
  let timer = null
  return Promise.race([
    promise.finally(() => {
      if (timer) clearTimeout(timer)
    }),
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), timeoutMs)
    }),
  ])
}

export async function send(client, request, label, timeoutMs = 30_000) {
  stage(label)
  return await withTimeout(client.send(request), timeoutMs, label + ' timed out')
}

export async function startKernel(repo, bundle, timeoutMs, LocalIpcClient, requests) {
  stage('starting isolated kernel')
  const port = 52000 + Math.floor(Math.random() * 1000)
  const runId = 'workflow-code-hetzner-second-kernel-' + process.pid + '-' + Date.now()
  const root = path.join(bundle, 'remote-kernel-root')
  const workspace = path.join(bundle, 'workspace')
  const worktree = path.join(bundle, 'worktree')
  const stdoutPath = path.join(bundle, 'kernel.stdout.log')
  const stderrPath = path.join(bundle, 'kernel.stderr.log')
  await mkdir(workspace, { recursive: true })
  await mkdir(worktree, { recursive: true })
  const stdoutFd = openSync(stdoutPath, 'a')
  const stderrFd = openSync(stderrPath, 'a')
  const child = spawn(path.join(repo, 'apps/kernel/target/debug/arroba-kernel'), [], {
    cwd: repo,
    env: {
      ...process.env,
      HOME: path.join(root, 'home'),
      XDG_CONFIG_HOME: path.join(root, 'xdg-config'),
      XDG_STATE_HOME: path.join(root, 'xdg-state'),
      XDG_CACHE_HOME: path.join(root, 'xdg-cache'),
      ARROBA_HOME: path.join(root, 'arroba-home'),
      ARROBA_KERNEL_PORT: String(port),
      ARROBA_MCP_PORT: String(port + 1000),
      ARROBA_OPENCODE_PORT: String(port + 2000),
      ARROBA_CODEX_PORT: String(port + 2001),
      ARROBA_DAEMON_SOCKET: path.join(os.tmpdir(), runId + '.sock'),
      ARROBA_DAEMON_ID: runId,
    },
    stdio: ['ignore', stdoutFd, stderrFd],
  })
  closeSync(stdoutFd)
  closeSync(stderrFd)
  const client = new LocalIpcClient('ws://127.0.0.1:' + port, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    await waitForKernel(client, requests, workspace, worktree, timeoutMs)
    stage('isolated kernel ready on ws://127.0.0.1:' + port)
    return { child, client, kernelUrl: 'ws://127.0.0.1:' + port, workspace, worktree, stdoutPath, stderrPath }
  } catch (error) {
    child.kill('SIGTERM')
    await client.close().catch(() => {})
    throw error
  }
}

export function printKernelLogTail(kernel) {
  for (const [label, filePath] of [['stdout', kernel.stdoutPath], ['stderr', kernel.stderrPath]]) {
    try {
      const contents = readFileSync(filePath, 'utf8')
      if (contents.trim()) {
        console.error('[workflow-code-hetzner] kernel ' + label + ' tail:')
        console.error(contents.slice(-8000))
      }
    } catch {
      // Best-effort diagnostics only.
    }
  }
}

export async function writeSourceDirectoryExport(workspace, exportResult, label) {
  stage(label + ': writing source directory export')
  const files = exportResult.files ?? []
  assert(files.some((file) => file.path === 'workflow.js'), label + ' source directory should include workflow.js', exportResult)
  assert(files.some((file) => file.path === 'manifest.json'), label + ' source directory should include manifest.json', exportResult)
  for (const file of files) {
    assert(sha256Hex(file.contents) === file.sha256, label + ' file hash mismatch for ' + file.path, file)
    const target = path.join(workspace, file.path)
    await mkdir(path.dirname(target), { recursive: true })
    await writeFile(target, file.contents, 'utf8')
  }
}

export async function validateSourceExportOnKernel(client, requests, session, nodePath, exportResult, label) {
  assert(exportResult?.source, label + ' should include source', exportResult)
  assert(sha256Hex(exportResult.source) === exportResult.source_sha256, label + ' source hash mismatch', exportResult)
  const validated = unwrap(
    await send(client, requests.validateWorkflowCodeRequest(session.id, nodePath, exportResult.source), label + ': validate source'),
    'WorkflowCodeValidated',
  ).result
  assert(validated?.validation?.ok, label + ' should validate', validated?.validation)
  const expected = expectationFromDefinition(validated.definition)
  const endpointHandle = validated.definition.endpoints?.[0]?.handle
  assert(endpointHandle, label + ' should define an endpoint handle', validated.definition)
  const appliedResponse = unwrap(
    await send(client, requests.applyWorkflowCodeRequest(session.id, nodePath, exportResult.source), label + ': apply source'),
    'WorkflowCodeApplied',
  )
  const apply = validateApplyResult(appliedResponse.result, label + ' apply', expected)
  validateSessionProjection(appliedResponse.session, apply, label + ' apply')
  const runResponse = unwrap(
    await send(
      client,
      requests.runWorkflowCodeRequest(session.id, nodePath, exportResult.source, 'Run ' + label + '.', { endpoint: endpointHandle }),
      label + ': run source',
      60_000,
    ),
    'WorkflowCodeRun',
  )
  const runApply = validateApplyResult(runResponse.result.apply, label + ' run', expected)
  validateSessionProjection(runResponse.session, runApply, label + ' run')
  assert(runResponse.result.invocation?.workflow_run || runResponse.result.invocation?.queued_prompt, label + ' should invoke or enqueue', runResponse.result.invocation)
  assert(apply.workflow_id !== runApply.workflow_id, label + ' run should apply a fresh workflow', { apply, runApply })
  return {
    validatedAlias: validated.definition.workflow?.alias ?? null,
    applyWorkflowId: apply.workflow_id,
    runWorkflowId: runApply.workflow_id,
    runInvocation: runResponse.result.invocation.workflow_run ? 'started' : 'enqueued',
  }
}

export function topologyRuntimeExpectation(definition) {
  return {
    ...expectationFromDefinition(definition),
  }
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

export function validateTopologyRuntimeResult(exampleName, run) {
  assert(run?.status === 'Completed', 'topology ' + exampleName + ' should complete', run)
  assert(run.final_output_valid !== false, 'topology ' + exampleName + ' final output should be schema-valid', run)
  const finalOutput = parseWorkflowOutputMessage(run.final_output)
  assert(finalOutput && typeof finalOutput === 'object', 'topology ' + exampleName + ' should produce structured final output', run.final_output)
  const expectations = {
    'adversarial-verification.js': { decision: 'accept' },
    'evaluator-optimizer.js': { accepted: true },
    'fan-out-synthesize.js': { source_count: 2 },
    'generate-filter.js': { selected_count: 1 },
    'loop-until-done.js': { iterations: 2 },
    'orchestrator-workers.js': { delegated: true },
    'parallelization.js': { reviewer_count: 2 },
    'prompt-chaining.js': { answer: 'refined draft accepted' },
    'routing.js': { specialist: 'code' },
    'tournament.js': { winner: 'a' },
  }[exampleName]
  assert(expectations, 'missing topology runtime expectation for ' + exampleName)
  for (const [field, value] of Object.entries(expectations)) {
    assert(finalOutput[field] === value, 'topology ' + exampleName + ' final output field ' + field + ' mismatch', {
      finalOutput,
      expected: value,
    })
  }
  return finalOutput
}

export async function waitForProviderRunReady(client, requests, providerRunId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const providerRun = unwrap(
      await send(client, requests.getProviderRunRequest(providerRunId), 'get provider run ' + providerRunId, 10_000),
      'ProviderRun',
    )?.provider_run
    if (providerRun?.state && providerRun.state !== 'Starting') {
      if (providerRun.state !== 'Running' && providerRun.state !== 'Parked') {
        throw new Error('provider run ' + providerRunId + ' reached unexpected state ' + providerRun.state)
      }
      return providerRun
    }
    await sleep(250)
  }
  throw new Error('provider run ' + providerRunId + ' did not become ready')
}

export async function waitForCompletedWorkflowRun(client, requests, sessionId, workflowRunId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastRun = null
  while (Date.now() < deadline) {
    const stateResponse = await send(client, requests.getSessionStateRequest(sessionId), 'load session state for workflow run ' + workflowRunId, 10_000)
    const state = unwrap(stateResponse, 'SessionStateLoaded')?.session
      ?? unwrap(stateResponse, 'SessionState')?.session
    const run = (state?.workflow_runs ?? []).find((entry) => entry.id === workflowRunId)
    if (run) {
      lastRun = run
      if (['Completed', 'Failed', 'Stopped'].includes(run.status)) {
        return run
      }
    }
    await sleep(500)
  }
  throw new Error('workflow run ' + workflowRunId + ' did not complete before timeout' + (lastRun ? '\n' + JSON.stringify(lastRun, null, 2) : ''))
}

export async function completeAppliedTopologyWorkflow(client, requests, sessionId, exampleName, definition, apply, timeoutMs) {
  const endpoint = (definition.endpoints ?? []).find((entry) => entry.handle === 'entry' || entry.alias === 'entry')
    ?? definition.endpoints?.[0]
  const entryNodeHandle = endpoint?.entry_node
  assert(entryNodeHandle, 'topology ' + exampleName + ' should resolve entry node', { definition, apply })
  const runtimeAgentEntries = Object.entries(apply.agent_ids ?? {})
    .sort(([left], [right]) => {
      if (left === entryNodeHandle) return 1
      if (right === entryNodeHandle) return -1
      return left.localeCompare(right)
    })
  for (const [handle, agentId] of runtimeAgentEntries) {
    const launchResponse = unwrap(
      await send(
        client,
        requests.launchProviderRunRequest(sessionId, 'dev-stub', 'default', 'workflow-code-topology-node', 'low', agentId),
        'launch topology provider ' + exampleName + ':' + handle,
        30_000,
      ),
      'ProviderRunLaunchAccepted',
    )
    assert(launchResponse?.provider_run?.id, 'topology ' + exampleName + ' should launch provider for node ' + handle, launchResponse)
    await waitForProviderRunReady(client, requests, launchResponse.provider_run.id, timeoutMs)
  }
  const endpointId = apply.endpoint_ids?.[endpoint.handle]
  assert(endpointId, 'topology ' + exampleName + ' should resolve entry endpoint', { endpoint, apply })
  const entryAgentId = apply.agent_ids?.[entryNodeHandle]
  assert(entryAgentId, 'topology ' + exampleName + ' should resolve entry agent', { entryNodeHandle, apply })
  await send(client, requests.focusAgentRequest(sessionId, entryAgentId), 'focus topology entry agent ' + exampleName, 10_000)
  const invokeResponse = unwrap(
    await send(
      client,
      requests.invokeWorkflowEndpointRequest(sessionId, apply.workflow_id, endpointId, 'Run ' + exampleName + ' Hetzner topology validation.'),
      'invoke topology workflow ' + exampleName,
      30_000,
    ),
    'WorkflowRunInvoked',
  )
  const workflowRun = invokeResponse?.workflow_run
  assert(workflowRun?.id, 'topology ' + exampleName + ' should start a workflow run', invokeResponse)
  const completed = await waitForCompletedWorkflowRun(client, requests, sessionId, workflowRun.id, timeoutMs)
  return {
    workflowRunId: completed.id,
    finalOutput: validateTopologyRuntimeResult(exampleName, completed),
  }
}

export async function validateTopologySourceExportOnKernel(client, requests, session, nodePath, workspace, exportResult, exampleName, label, timeoutMs) {
  assert(exportResult?.source, label + ' should include source', exportResult)
  assert(sha256Hex(exportResult.source) === exportResult.source_sha256, label + ' source hash mismatch', exportResult)
  if (exportResult.format === 'directory') {
    await writeSourceDirectoryExport(workspace, exportResult, label)
  }
  const validated = unwrap(
    await send(client, requests.validateWorkflowCodeRequest(session.id, nodePath, exportResult.source), label + ': validate topology source', 60_000),
    'WorkflowCodeValidated',
  ).result
  assert(validated?.validation?.ok, label + ' should validate', validated?.validation)
  const appliedResponse = unwrap(
    await send(client, requests.applyWorkflowCodeRequest(session.id, nodePath, exportResult.source), label + ': apply topology source', 60_000),
    'WorkflowCodeApplied',
  )
  const expected = topologyRuntimeExpectation(validated.definition)
  const apply = validateApplyResult(appliedResponse.result, label + ' apply', expected)
  validateSessionProjection(appliedResponse.session, apply, label + ' apply')
  assert(apply.workflow_id !== exportResult.source_workflow_id, label + ' apply workflow id must be fresh', apply)
  const completed = await completeAppliedTopologyWorkflow(client, requests, session.id, exampleName, validated.definition, apply, timeoutMs)
  return {
    applyWorkflowId: apply.workflow_id,
    workflowRunId: completed.workflowRunId,
    finalOutput: completed.finalOutput,
    sourceSha256: exportResult.source_sha256,
  }
}

export async function main() {
  const options = parseArgs(process.argv.slice(2))
  stage('loading bundled kernel client and workflow-code exports')
  const ipcModule = await import(pathToFileURL(path.join(options.bundle, 'kernel-client-dist/ipc.js')))
  const requests = await import(pathToFileURL(path.join(options.bundle, 'kernel-client-dist/ipc-requests.js')))
  const packageExport = JSON.parse(await readFile(path.join(options.bundle, 'package-export.json'), 'utf8'))
  const inlineSource = JSON.parse(await readFile(path.join(options.bundle, 'inline-source.json'), 'utf8'))
  const directorySource = JSON.parse(await readFile(path.join(options.bundle, 'directory-source.json'), 'utf8'))
  const topologyLiveExports = JSON.parse(await readFile(path.join(options.bundle, 'topology-live-exports.json'), 'utf8'))
  const metadata = JSON.parse(await readFile(path.join(options.bundle, 'metadata.json'), 'utf8'))
  const kernel = await startKernel(options.repo, options.bundle, options.timeoutMs, ipcModule.LocalIpcClient, requests)
  const sessionIds = []
  let installedSkill = false
  try {
    const session = unwrap(
      await send(
        kernel.client,
        requests.createSessionRequest(kernel.workspace, kernel.worktree, 'workflow-code-hetzner-second-kernel', undefined, null, 'off'),
        'create validation session',
      ),
      'SessionCreated',
    ).session
    sessionIds.push(session.id)
    const skillRoot = path.join(options.bundle, 'skill')
    const skillDirs = await readdir(skillRoot)
    const skillSourceDir = path.join(skillRoot, skillDirs[0])
    const skillInstall = unwrap(
      await send(kernel.client, requests.installSkillRequest(kernel.workspace, skillSourceDir), 'install bundled drill skill'),
      'SkillInstalled',
    )
    assert(skillInstall?.skill?.name === metadata.skillName, 'Hetzner kernel should install workflow-code drill skill', skillInstall)
    installedSkill = true
    const nodePath = process.execPath
    const importedName = 'hetzner-package-' + Date.now()
    const imported = unwrap(
      await send(
        kernel.client,
        requests.importWorkflowCodePackageRequest(session.id, packageExport, nodePath, { name: importedName, overwrite: false }),
        'import workflow-code package',
        120_000,
      ),
      'WorkflowCodePackageImported',
    ).artifact
    assert(imported?.metadata?.validation?.ok, 'Hetzner package import should validate', imported?.metadata?.validation)
    const packageExpected = expectationFromDefinition(packageExport.definition)
    const packageRebindings = rebindingsForDefinition(packageExport.definition)
    const packageApplyResponse = unwrap(
      await send(
        kernel.client,
        requests.applyWorkflowCodeArtifactRequest(session.id, importedName, packageRebindings),
        'apply workflow-code package',
      ),
      'WorkflowCodeApplied',
    )
    const packageApply = validateApplyResult(packageApplyResponse.result, 'Hetzner package', packageExpected)
    validateSessionProjection(packageApplyResponse.session, packageApply, 'Hetzner package')
    const packageRun = unwrap(
      await send(
        kernel.client,
        requests.runWorkflowCodeArtifactRequest(session.id, importedName, 'Run the Hetzner package workflow.', { endpoint: 'entry', providerRebindings: packageRebindings }),
        'run workflow-code package',
        60_000,
      ),
      'WorkflowCodeRun',
    )
    const packageRunApply = validateApplyResult(packageRun.result.apply, 'Hetzner package run', packageExpected)
    validateSessionProjection(packageRun.session, packageRunApply, 'Hetzner package run')
    assert(packageApply.workflow_id !== metadata.sourceWorkflowId, 'Hetzner package workflow id must be fresh', { packageApply, metadata })
    assert(packageRunApply.workflow_id !== metadata.sourceWorkflowId, 'Hetzner package run workflow id must be fresh', { packageRunApply, metadata })

    const inline = await validateSourceExportOnKernel(kernel.client, requests, session, nodePath, inlineSource, 'Hetzner inline source')
    await writeSourceDirectoryExport(kernel.workspace, directorySource, 'Hetzner source directory')
    const directory = await validateSourceExportOnKernel(kernel.client, requests, session, nodePath, directorySource, 'Hetzner source directory')
    await send(kernel.client, requests.endSessionRequest(session.id), 'end package/source validation session', 10_000)
    const endedSessionIndex = sessionIds.indexOf(session.id)
    if (endedSessionIndex !== -1) {
      sessionIds.splice(endedSessionIndex, 1)
    }
    const topologySession = unwrap(
      await send(
        kernel.client,
        requests.createSessionRequest(kernel.workspace, kernel.worktree, 'workflow-code-hetzner-topology-suite', undefined, null, 'off'),
        'create topology validation session',
      ),
      'SessionCreated',
    ).session
    sessionIds.push(topologySession.id)
    const topologySourceRuns = []
    for (const liveExport of topologyLiveExports ?? []) {
      const inlineTopology = await validateTopologySourceExportOnKernel(
        kernel.client,
        requests,
        topologySession,
        nodePath,
        kernel.workspace,
        { ...liveExport.inlineSource, source_workflow_id: liveExport.sourceWorkflowId },
        liveExport.exampleName,
        'Hetzner ' + liveExport.exampleName + ' live inline source',
        options.timeoutMs,
      )
      const directoryTopology = await validateTopologySourceExportOnKernel(
        kernel.client,
        requests,
        topologySession,
        nodePath,
        kernel.workspace,
        { ...liveExport.directorySource, source_workflow_id: liveExport.sourceWorkflowId },
        liveExport.exampleName,
        'Hetzner ' + liveExport.exampleName + ' live source directory',
        options.timeoutMs,
      )
      topologySourceRuns.push({
        example: liveExport.exampleName,
        sourceWorkflowId: liveExport.sourceWorkflowId,
        inline: inlineTopology,
        directory: directoryTopology,
      })
    }
    console.log(JSON.stringify({
      kernelUrl: kernel.kernelUrl,
      packageSourceSessionId: session.id,
      topologySessionId: topologySession.id,
      packageImportedArtifact: importedName,
      packageApplyWorkflowId: packageApply.workflow_id,
      packageRunWorkflowId: packageRunApply.workflow_id,
      inline,
      directory,
      topologySourceRuns,
    }, null, 2))
  } catch (error) {
    printKernelLogTail(kernel)
    throw error
  } finally {
    for (const sessionId of sessionIds) {
      if (installedSkill) {
        await send(
          kernel.client,
          requests.uninstallSkillRequest(kernel.workspace, metadata.skillName),
          'uninstall bundled drill skill',
          10_000,
        ).catch(() => {})
        installedSkill = false
      }
      await send(kernel.client, requests.endSessionRequest(sessionId), 'end validation session', 10_000).catch(() => {})
    }
    await kernel.client.close().catch(() => {})
    kernel.child.kill('SIGTERM')
    await sleep(1000)
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
`
}
