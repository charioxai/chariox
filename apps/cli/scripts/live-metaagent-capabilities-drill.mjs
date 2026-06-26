#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 60_000
const DEFAULT_POLL_MS = 250

function parseArgs(argv) {
  const options = {
    keepArtifactsOnFailure: true,
    preserveOnSuccess: true,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--discard-artifacts-on-failure') options.keepArtifactsOnFailure = false
    else if (arg === '--preserve-on-success') options.preserveOnSuccess = true
    else if (arg === '--discard-artifacts-on-success') options.preserveOnSuccess = false
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-metaagent-capabilities-drill.mjs [options]',
        '',
        'Runs an isolated local kernel drill for meta-mode capabilities:',
        '- verifies metaagent runtime tool policy: meta tools, read-only workspace, and recall',
        '- verifies scoped metaagent task and plan artifacts',
        '- verifies workflow-code artifact create/validate/apply/run/export/import/delete through meta runtime tools',
        '- verifies MCP install/grant/revoke/uninstall via arroba.meta.run_command',
        '- verifies skill install/grant/revoke/uninstall via arroba.meta.run_command',
        '- verifies credential handle and vault status commands without passing secrets',
        '- verifies worker runtime interaction resolution',
        '- verifies direct execution denials and session-management guardrails',
        '',
        'Options:',
        `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
        `  --poll-ms ${DEFAULT_POLL_MS}`,
        '  --keep-artifacts-on-failure',
        '  --discard-artifacts-on-failure',
        '  --preserve-on-success',
        '  --discard-artifacts-on-success',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 57500 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[metaagent-capabilities-drill] ${name}`)
  else console.log(`[metaagent-capabilities-drill] ${name}`, JSON.stringify(details))
}

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function runChecked(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

async function initGitWorktree(root) {
  await runChecked('git', ['init', '-b', 'main'], { cwd: root })
  await runChecked('git', ['config', 'user.email', 'metaagent-capabilities-drill@example.com'], { cwd: root })
  await runChecked('git', ['config', 'user.name', 'Metaagent Capabilities Drill'], { cwd: root })
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const existing = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (existing) return binary
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return binary
}

async function waitForDaemon(shellBin, kernelUrl, workspace, env) {
  const scriptPath = path.join(workspace, 'wait.arroba')
  await writeFile(scriptPath, 'session list\n', 'utf8')
  const deadline = Date.now() + 20_000
  let last = null
  while (Date.now() < deadline) {
    last = await run(process.execPath, [shellBin, 'run', scriptPath, '--kernel-url', kernelUrl, '--workspace', workspace, '--worktree', workspace], { env })
    if (last.code === 0) return
    await sleep(250)
  }
  throw new Error(`daemon did not become ready\nstdout:\n${last?.stdout ?? ''}\nstderr:\n${last?.stderr ?? ''}`)
}

function requireOutput(output, pattern, label) {
  if (!pattern.test(output)) {
    throw new Error(`missing ${label}: ${pattern}\n--- output ---\n${output}`)
  }
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

function unwrapVariant(response, ...keys) {
  return keys.map((key) => response?.[key]).find((value) => value != null) ?? response
}

function assert(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

function metaWorkflowCodeSource() {
  return `
workflow.define({
  alias: "meta_capabilities_workflow_code",
  prompt: "Complete the isolated metaagent workflow-code drill.",
  maxConcurrent: 2,
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Final output",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const worker = workflow.node({
  handle: "workflow_worker",
  agent: workflow.newAgent({ alias: "meta-workflow-worker", provider: "codex", model: "gpt-5" }),
  publicLabel: "Workflow worker",
  instructions: "Submit final output matching final_output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 0, y: 80 },
});

workflow.endpoint(worker, { handle: "entry", alias: "entry", canvas: { x: -220, y: 80 } });
`.trim()
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function launchRuntime(client, requests, sessionId, agentId, model, timeoutMs, pollMs) {
  const launched = unwrapVariant(
    await client.send(requests.launchProviderRunRequest(sessionId, 'dev-stub', 'default', model, 'low', agentId)),
    'ProviderRunLaunched',
    'ProviderRunLaunchAccepted',
  )
  const providerRun = launched.provider_run
  if (!providerRun?.id) throw new Error(`launch did not return provider run: ${JSON.stringify(launched)}`)
  const deadline = Date.now() + timeoutMs
  let last = providerRun
  while (Date.now() < deadline) {
    last = unwrap(await client.send(requests.getProviderRunRequest(providerRun.id)), 'ProviderRun').provider_run
    if (last?.runtime_mcp_server_url && last?.runtime_mcp_auth_token) return last
    if (last?.state === 'Ended') throw new Error(`provider run ended before exposing runtime MCP: ${JSON.stringify(last)}`)
    await sleep(pollMs)
  }
  throw new Error(`provider run did not expose runtime MCP binding: ${JSON.stringify(last)}`)
}

async function waitForAgentProviderRun(client, requests, sessionId, agentId, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastSession = null
  while (Date.now() < deadline) {
    lastSession = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
    const providerRunId = lastSession.active_provider_run_id
    if (providerRunId) {
      const run = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
      if (run?.agent_instance_id === agentId || run?.agent_id === agentId) {
        const deadlineForRuntime = Date.now() + Math.max(0, deadline - Date.now())
        let last = run
        while (Date.now() < deadlineForRuntime) {
          last = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
          if (last?.runtime_mcp_server_url && last?.runtime_mcp_auth_token) return last
          if (last?.state === 'Ended') throw new Error(`provider run ended before exposing runtime MCP: ${JSON.stringify(last)}`)
          await sleep(pollMs)
        }
        throw new Error(`provider run did not expose runtime MCP binding: ${JSON.stringify(last)}`)
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run for agent ${agentId}: ${JSON.stringify(lastSession)}`)
}

async function callRuntimeMcp(providerRun, method, params = {}) {
  const response = await fetch(providerRun.runtime_mcp_server_url, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${providerRun.runtime_mcp_auth_token}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: `${method}-${Date.now()}-${Math.random().toString(16).slice(2)}`,
      method,
      params,
    }),
  })
  const text = await response.text()
  let json
  try {
    json = JSON.parse(text)
  } catch {
    throw new Error(`runtime MCP response was not JSON (${response.status}): ${text}`)
  }
  if (!response.ok || json.error) throw new Error(`runtime MCP ${method} failed: ${text}`)
  return json.result
}

async function callRuntimeTool(providerRun, name, args = {}) {
  const result = await callRuntimeMcp(providerRun, 'tools/call', {
    name,
    arguments: args,
  })
  return {
    ok: !result.isError,
    payload: result.structuredContent,
    raw: result,
  }
}

async function listRuntimeToolNames(providerRun) {
  const result = await callRuntimeMcp(providerRun, 'tools/list')
  return (result.tools ?? []).map((tool) => tool.name)
}

async function assertRuntimeToolDenied(providerRun, name, args, label) {
  const result = await callRuntimeTool(providerRun, name, args)
  assert(!result.ok, label, result.payload)
  return result
}

async function verifyMetaWorkflowCodeGuides(metaRun) {
  const guideSearch = await callRuntimeTool(metaRun, 'arroba.meta.search_guides', {
    query: 'workflow code authoring apply run export import',
    tag: 'workflow-code',
    limit: 5,
  })
  assert(guideSearch.ok, 'metaagent should discover workflow-code authoring guides', guideSearch.payload)
  const searchedGuides = guideSearch.payload?.guides ?? []
  assert(
    searchedGuides.some((guide) => guide.id === 'workflows/workflow-code-authoring'),
    'workflow-code guide search should return the authoring guide',
    guideSearch.payload,
  )
  assert(
    searchedGuides.every((guide) => guide.body === undefined),
    'workflow-code guide search should return summaries without guide bodies',
    guideSearch.payload,
  )

  for (const command of [
    'arroba.meta.workflow_code.create',
    'arroba.meta.workflow_code.read',
    'arroba.meta.workflow_code.list',
    'arroba.meta.workflow_code.update',
    'arroba.meta.workflow_code.delete',
    'arroba.meta.workflow_code.validate',
    'arroba.meta.workflow_code.apply',
    'arroba.meta.workflow_code.run',
    'arroba.meta.workflow_code.export',
    'arroba.meta.workflow_code.import',
  ]) {
    const listedGuides = await callRuntimeTool(metaRun, 'arroba.meta.list_guides', {
      command,
      limit: 10,
    })
    assert(listedGuides.ok, `metaagent should list guides by workflow-code command ${command}`, listedGuides.payload)
    assert(
      (listedGuides.payload?.guides ?? []).some((guide) => guide.id === 'workflows/workflow-code-authoring'),
      `workflow-code guide list should include the authoring guide for ${command}`,
      listedGuides.payload,
    )
  }

  const authoringGuide = await callRuntimeTool(metaRun, 'arroba.meta.read_guide', {
    guide: 'workflows/workflow-code-authoring',
  })
  assert(authoringGuide.ok, 'metaagent should read the workflow-code authoring guide', authoringGuide.payload)
  const authoringBody = authoringGuide.payload?.body ?? ''
  assert(
    authoringBody.includes('workflow.node') && authoringBody.includes('workflow.endpoint') && authoringBody.includes('provider_rebindings'),
    'workflow-code authoring guide should teach the real builder API and portable provider rebinding',
    authoringGuide.payload,
  )

  const patternGuide = await callRuntimeTool(metaRun, 'arroba.meta.read_guide', {
    guide: 'workflows/workflow-code-patterns',
  })
  assert(patternGuide.ok, 'metaagent should read the workflow-code pattern guide', patternGuide.payload)
  const patternBody = patternGuide.payload?.body ?? ''
  assert(
    patternBody.includes('```js') && patternBody.includes('workflow.define') && patternBody.includes('workflow.edge'),
    'workflow-code pattern guide should embed runnable JavaScript examples',
    patternGuide.payload,
  )

  return {
    searched: searchedGuides.map((guide) => guide.id),
    authoringGuide: authoringGuide.payload?.id,
    patternGuide: patternGuide.payload?.id,
  }
}

async function waitForInteraction(client, requests, sessionId, agentId, title, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const payload = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded')
    const session = payload.session ?? payload
    last = session
    const interaction = (session.active_interactions ?? [])
      .find((entry) => entry.agent_id === agentId && entry.title === title)
    if (interaction) return interaction
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for interaction ${title}\n${JSON.stringify(last, null, 2)}`)
}

async function waitForMetaagentTask(client, requests, sessionId, metaagentId, predicate, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const payload = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded')
    const session = payload.session ?? payload
    last = session.metaagent_tasks ?? []
    const task = last.find((entry) => entry.metaagent_id === metaagentId)
    if (task && predicate(task)) return task
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for metaagent task projection\n${JSON.stringify(last, null, 2)}`)
}

async function verifyMetaWorkflowCodeTools(metaRun, metaagentId) {
  const name = `meta-capabilities-flow-${Date.now()}`
  const importedName = `${name}-imported`
  const source = metaWorkflowCodeSource()
  const updatedSource = source.replace(
    'alias: "meta_capabilities_workflow_code"',
    'alias: "meta_capabilities_workflow_code_updated"',
  )
  const providerRebindings = [{
    node: 'workflow_worker',
    provider: 'dev-stub',
    model: 'default',
  }]

  const created = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.create', {
    name,
    source,
  })
  assert(created.ok, 'metaagent should create workflow-code artifacts through runtime MCP', created.payload)
  const createdArtifact = unwrap(created.payload, 'WorkflowCodeArtifactCreated').artifact
  assert(createdArtifact?.metadata?.validation?.ok, 'created workflow-code artifact should validate', created.payload)
  assert(
    createdArtifact?.metadata?.provenance?.created_by?.metaagent_id === metaagentId,
    'created workflow-code artifact should record metaagent provenance',
    created.payload,
  )

  const listed = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.list')
  assert(
    listed.ok && (unwrap(listed.payload, 'WorkflowCodeArtifactsListed').artifacts ?? [])
      .some((artifact) => artifact.name === name),
    'workflow-code list should include the created artifact',
    listed.payload,
  )

  const read = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.read', { name })
  assert(read.ok, 'metaagent should read workflow-code artifacts through runtime MCP', read.payload)
  const readArtifact = unwrap(read.payload, 'WorkflowCodeArtifact').artifact
  assert(
    readArtifact?.definition?.workflow?.alias === 'meta_capabilities_workflow_code',
    'workflow-code read should return the saved artifact definition',
    read.payload,
  )

  const updated = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.update', {
    name,
    source: updatedSource,
  })
  assert(updated.ok, 'metaagent should update workflow-code artifacts through runtime MCP', updated.payload)
  const updatedArtifact = unwrap(updated.payload, 'WorkflowCodeArtifactUpdated').artifact
  assert(
    updatedArtifact?.definition?.workflow?.alias === 'meta_capabilities_workflow_code_updated',
    'workflow-code update should replace the saved artifact source and definition',
    updated.payload,
  )
  assert(
    updatedArtifact?.metadata?.provenance?.updated_by?.metaagent_id === metaagentId,
    'updated workflow-code artifact should record metaagent provenance',
    updated.payload,
  )
  assert(
    (updatedArtifact?.metadata?.history ?? []).some((entry) => entry.action === 'updated'),
    'workflow-code update should record artifact history',
    updated.payload,
  )

  const validated = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.validate', { name })
  assert(
    validated.ok
      && unwrap(validated.payload, 'WorkflowCodeValidated').result?.validation?.ok,
    'metaagent should validate saved workflow-code artifacts through runtime MCP',
    validated.payload,
  )

  const applied = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.apply', {
    name,
    provider_rebindings: providerRebindings,
  })
  assert(applied.ok, 'metaagent should apply saved workflow-code artifacts through runtime MCP', applied.payload)
  const appliedPayload = unwrap(applied.payload, 'WorkflowCodeApplied')
  const appliedWorkflowId = appliedPayload.result?.apply?.workflow_id
  const appliedWorkerAgentId = appliedPayload.result?.apply?.agent_ids?.workflow_worker
  assert(appliedWorkflowId, 'workflow-code apply should return a workflow id', applied.payload)
  assert(appliedWorkerAgentId, 'workflow-code apply should return the generated node agent id', applied.payload)
  assert(
    (appliedPayload.session?.workflows ?? []).some((workflow) => workflow.id === appliedWorkflowId),
    'workflow-code apply should project the created workflow into the session',
    applied.payload,
  )
  assert(
    (appliedPayload.session?.workflows ?? []).some((workflow) => (
      workflow.id === appliedWorkflowId
      && workflow.controlled_by_metaagent_id === metaagentId
    )),
    'workflow-code apply should mark the generated workflow as controlled by the metaagent',
    applied.payload,
  )
  assert(
    (appliedPayload.session?.agents ?? []).some((agent) => (
      agent.id === appliedWorkerAgentId
      && agent.provider === 'dev-stub'
      && agent.model === 'default'
      && agent.controlled_by_metaagent_id === metaagentId
    )),
    'workflow-code apply should create a rebound dev-stub/default node agent controlled by the metaagent',
    applied.payload,
  )

  const exported = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.export', { name })
  assert(exported.ok, 'metaagent should export workflow-code artifacts through runtime MCP', exported.payload)
  const exportedPackage = unwrap(exported.payload, 'WorkflowCodeArtifactExported').package
  assert(exportedPackage?.source_sha256, 'workflow-code export should include package source hash', exported.payload)

  const imported = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.import', {
    name: importedName,
    package: exportedPackage,
  })
  assert(imported.ok, 'metaagent should import workflow-code packages through runtime MCP', imported.payload)

  const run = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.run', {
    name: importedName,
    endpoint: 'entry',
    provider_rebindings: providerRebindings,
  })
  assert(run.ok, 'metaagent should run workflow-code artifacts through runtime MCP', run.payload)
  const runPayload = unwrap(run.payload, 'WorkflowCodeRun')
  assert(
    runPayload.result?.invocation?.kind === 'started' || runPayload.result?.invocation?.kind === 'enqueued',
    'workflow-code run should start or enqueue a workflow invocation',
    run.payload,
  )
  if (runPayload.result?.invocation?.kind === 'started') {
    assert(
      runPayload.result.invocation.workflow_run?.invocation_prompt === 'Complete the isolated metaagent workflow-code drill.',
      'workflow-code run should use the script-level prompt when the runtime MCP call omits prompt',
      run.payload,
    )
    assert(
      runPayload.result.invocation.workflow?.controlled_by_metaagent_id === metaagentId,
      'workflow-code run should mark the invoked generated workflow as controlled by the metaagent',
      run.payload,
    )
  }

  const deletedOriginal = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.delete', { name })
  assert(deletedOriginal.ok, 'metaagent should delete original workflow-code artifact', deletedOriginal.payload)
  const deletedImported = await callRuntimeTool(metaRun, 'arroba.meta.workflow_code.delete', { name: importedName })
  assert(deletedImported.ok, 'metaagent should delete imported workflow-code artifact', deletedImported.payload)

  return {
    artifactName: name,
    importedName,
    appliedWorkflowId,
    runInvocation: runPayload.result?.invocation?.kind,
  }
}

async function cleanupSession(kernelUrl, sessionId) {
  if (!sessionId) return
  const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
  const { endSessionRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const client = new LocalIpcClient(kernelUrl)
  try {
    await client.send(endSessionRequest(sessionId)).catch(() => {})
  } finally {
    await client.close().catch(() => {})
  }
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null || child.signalCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null && child.signalCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-metaagent-capabilities-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const scriptsDir = path.join(rootDir, 'scripts')
  const skillDir = path.join(rootDir, 'iso-skill')
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const shellBin = path.join(repoRoot, 'apps/shell/dist/shell.js')
  const env = {
    ...process.env,
    HOME: home,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `metaagent-capabilities-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
    ARROBA_MANAGED_CAPABILITY_ROOT: path.join(rootDir, 'capabilities'),
  }

  let daemon = null
  let client = null
  let sessionId = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    await mkdir(scriptsDir, { recursive: true })
    await mkdir(skillDir, { recursive: true })
    await writeFile(
      path.join(skillDir, 'SKILL.md'),
      '---\nname: iso-skill\ndescription: Isolated metaagent capability drill skill\n---\nUse this only for the isolated metaagent capability drill.\n',
      'utf8',
    )
    await writeFile(
      path.join(workspace, 'README.md'),
      '# Metaagent capabilities fixture\n\nThe metaagent should be able to read this planning context.\n',
      'utf8',
    )
    await initGitWorktree(workspace)

    const kernelBinary = await buildKernel()
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForDaemon(shellBin, kernelUrl, workspace, env)
    log('daemon-ready', { kernelUrl })

    const setupScript = path.join(scriptsDir, 'setup.arroba')
    await writeFile(setupScript, [
      'set provider dev-stub',
      'set model metaagent-capabilities-default',
      'session new $workspace as session',
      'session permissions yolo',
      'agent list',
    ].join('\n'), 'utf8')
    const setup = await run(process.execPath, [
      shellBin,
      'run',
      setupScript,
      '--kernel-url',
      kernelUrl,
      '--workspace',
      workspace,
      '--worktree',
      workspace,
      '--var',
      `workspace=${workspace}`,
    ], { env })
    if (setup.code !== 0) {
      throw new Error(`setup script failed\nstdout:\n${setup.stdout}\nstderr:\n${setup.stderr}`)
    }
    sessionId = setup.stdout.match(/bound \$session = (\S+)/)?.[1] ?? null
    assert(sessionId, 'setup script did not bind session id', { stdout: setup.stdout })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `metaagent-capabilities-drill-${Date.now()}`)), 'SessionAttached').attachment
    await client.subscribeToKernelEvents(sessionId, attachment.id)

    const sessionState = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
    const agents = sessionState.agents ?? []
    const metaagent = agents.find((agent) => agent.id === sessionState.focused_agent_id) ?? agents[0]
    assert(metaagent, 'session should contain a default regular agent', { agents })
    assert(!metaagent.meta_mode, 'default agent must start outside meta mode', metaagent)

    await client.send(requests.submitPromptRequest(
      sessionId,
      attachment.id,
      metaagent.id,
      '/meta Coordinate the isolated metaagent capability validation. Keep the task active while the kernel drill verifies runtime capabilities.',
      [],
    ))
    const metaRun = await waitForAgentProviderRun(client, requests, sessionId, metaagent.id, options.timeoutMs, options.pollMs)
    assert(metaRun.execution_mode === 'plan', 'metaagent provider run must be forced to plan mode', { metaRun })
    const metaSession = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
    const metaModeAgent = (metaSession.agents ?? []).find((agent) => agent.id === metaagent.id)
    assert(metaModeAgent?.meta_mode, 'same regular agent should enter meta mode after /meta prompt', metaModeAgent)

    const metaTools = await listRuntimeToolNames(metaRun)
    assert(metaTools.length > 0, 'metaagent runtime should expose runtime tools', { metaTools })
    for (const expectedTool of [
      'arroba.meta.read_task',
      'arroba.meta.update_task',
      'arroba.meta.read_plan',
      'arroba.meta.update_plan',
      'arroba.meta.complete_task',
      'arroba.meta.mark_blocked',
      'arroba.meta.search_guides',
      'arroba.meta.list_guides',
      'arroba.meta.read_guide',
      'arroba.read_artifact',
      'arroba.search_recall',
      'arroba.query_recall',
      'arroba.meta.workflow_code.create',
      'arroba.meta.workflow_code.read',
      'arroba.meta.workflow_code.list',
      'arroba.meta.workflow_code.update',
      'arroba.meta.workflow_code.delete',
      'arroba.meta.workflow_code.validate',
      'arroba.meta.workflow_code.apply',
      'arroba.meta.workflow_code.run',
      'arroba.meta.workflow_code.export',
      'arroba.meta.workflow_code.import',
    ]) {
      assert(metaTools.includes(expectedTool), `metaagent runtime should expose ${expectedTool}`, { metaTools })
    }
    assert(
      metaTools.every((tool) => tool.startsWith('arroba.meta.') || tool === 'arroba.read_artifact' || tool.startsWith('arroba.search_recall') || tool.startsWith('arroba.query_recall')),
      'metaagent runtime must expose only meta, read-only workspace, and recall tools',
      { metaTools },
    )

    const workerSpawn = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: 'agent spawn worker metaagent-capabilities-worker',
    })
    assert(workerSpawn.ok, 'metaagent should spawn an owned regular worker', workerSpawn.payload)
    const worker = workerSpawn.payload?.response?.agent
    assert(worker?.id && !worker.meta_mode, 'metaagent worker spawn should return a standard agent', workerSpawn.payload)

    const readTaskInitial = await callRuntimeTool(metaRun, 'arroba.meta.read_task')
    assert(readTaskInitial.ok && readTaskInitial.payload?.status === 'active', 'metaagent task should be active after /meta activation', readTaskInitial.payload)
    const updateTask = await callRuntimeTool(metaRun, 'arroba.meta.update_task', {
      markdown: 'Coordinate the isolated capability validation.',
    })
    assert(updateTask.ok && updateTask.payload?.task?.task_markdown?.includes('Coordinate the isolated'), 'metaagent should update its scoped task document', updateTask.payload)
    const updatePlan = await callRuntimeTool(metaRun, 'arroba.meta.update_plan', {
      markdown: '- Verify planning context\n- Delegate capability checks',
    })
    assert(updatePlan.ok && updatePlan.payload?.plan_markdown?.includes('Delegate capability checks'), 'metaagent should update its scoped plan document', updatePlan.payload)
    const readPlan = await callRuntimeTool(metaRun, 'arroba.meta.read_plan')
    assert(readPlan.ok && readPlan.payload?.plan_markdown?.includes('Verify planning context'), 'metaagent should read its scoped plan document', readPlan.payload)
    const projectedTask = await waitForMetaagentTask(
      client,
      requests,
      sessionId,
      metaagent.id,
      (task) => task.task_markdown.includes('isolated capability'),
      options.timeoutMs,
      options.pollMs,
    )
    assert(projectedTask, 'session snapshot should expose scoped metaagent task state')

    const artifactRead = await callRuntimeTool(metaRun, 'arroba.read_artifact', { path: 'README.md' })
    assert(artifactRead.ok, 'metaagent should be able to read workspace artifacts for planning', artifactRead.payload)
    const recallSearch = await callRuntimeTool(metaRun, 'arroba.search_recall', { query: 'metaagent capability drill', mode: 'keyword', limit: 3 })
    assert(recallSearch.ok, 'metaagent should be able to search recall for planning', recallSearch.payload)
    const recallQuery = await callRuntimeTool(metaRun, 'arroba.query_recall', { text: 'metaagent capability drill', limit: 3 })
    assert(recallQuery.ok, 'metaagent should be able to query recall for planning', recallQuery.payload)

    const deniedToolCalls = [
      ['arroba.write_artifact', { path: 'README.md', content_text: 'forbidden', domain: 'text' }, 'workspace artifact writes must be denied'],
      ['arroba.register_script_path', { name: 'forbidden-script', path: '/tmp/forbidden-script.js' }, 'script registration must be denied'],
      ['arroba.register_connector_path', { name: 'forbidden-connector', path: '/tmp/forbidden-connector' }, 'connector registration must be denied'],
      ['arroba.request_extension', { kind: 'mcp', name: 'iso-mcp' }, 'user MCP requests must be denied'],
      ['arroba.request_credential_secret', { credential_id: 'iso-credential' }, 'raw credential secret requests must be denied'],
      ['arroba.http_request_with_credential', { credential_id: 'iso-credential', url: 'https://example.invalid' }, 'credential-backed HTTP execution must be denied'],
      ['arroba.slice_screenshot', {}, 'slice runtime tools must be denied'],
      ['ack_workflow_turn', { status: 'complete' }, 'workflow-node runtime tools must be denied'],
    ]
    const deniedRuntimeTools = []
    for (const [name, args, label] of deniedToolCalls) {
      const denied = await assertRuntimeToolDenied(metaRun, name, args, `metaagent ${label}`)
      deniedRuntimeTools.push({ name, payload: denied.payload })
    }
    log('direct-runtime-tool-denials-passed', { tools: deniedRuntimeTools.map((entry) => entry.name) })

    const overview = await callRuntimeTool(metaRun, 'arroba.meta.session_overview')
    assert(overview.ok, 'session_overview should succeed', overview.payload)
    assert((overview.payload?.agents?.owned ?? []).some((agent) => agent.id === worker.id), 'session_overview should include owned worker', overview.payload)

    const workflowCodeGuideSummary = await verifyMetaWorkflowCodeGuides(metaRun)
    log('workflow-code-guides-passed', workflowCodeGuideSummary)

    const workflowCodeSummary = await verifyMetaWorkflowCodeTools(metaRun, metaagent.id)
    log('workflow-code-capabilities-passed', workflowCodeSummary)

    const mcpConfig = {
      name: 'iso-mcp',
      transport: {
        type: 'stdio',
        command: process.execPath,
        args: ['-e', 'process.exit(0)'],
        env: {},
        credential_env: {},
        env_vars: [],
      },
      enabled: true,
      required: false,
    }
    const mcpInstall = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: `mcp install-json '${JSON.stringify(mcpConfig)}'`,
    })
    assert(mcpInstall.ok, 'metaagent should install MCP definitions through kernel policy', mcpInstall.payload)
    const mcpGrant = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'mcp grant worker iso-mcp' })
    assert(mcpGrant.ok, 'metaagent should grant MCP to owned worker', mcpGrant.payload)
    const mcpSelfGrant = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: `mcp grant ${metaagent.id} iso-mcp` })
    assert(!mcpSelfGrant.ok, 'metaagent must not grant MCPs to itself', mcpSelfGrant.payload)
    const mcpRevoke = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'mcp revoke worker iso-mcp' })
    assert(mcpRevoke.ok, 'metaagent should revoke MCP from owned worker', mcpRevoke.payload)
    const mcpUninstall = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'mcp uninstall iso-mcp' })
    assert(mcpUninstall.ok, 'metaagent should uninstall MCP definitions through kernel policy', mcpUninstall.payload)
    log('mcp-capabilities-passed')

    const skillInstall = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: `skill install ${skillDir}` })
    assert(skillInstall.ok, 'metaagent should install skill packages through kernel policy', skillInstall.payload)
    const skillGrant = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'skill grant worker iso-skill' })
    assert(skillGrant.ok, 'metaagent should grant skill to owned worker', skillGrant.payload)
    const skillSelfGrant = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: `skill grant ${metaagent.id} iso-skill` })
    assert(!skillSelfGrant.ok, 'metaagent must not grant skills to itself', skillSelfGrant.payload)
    const skillRevoke = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'skill revoke worker iso-skill' })
    assert(skillRevoke.ok, 'metaagent should revoke skill from owned worker', skillRevoke.payload)
    const skillUninstall = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'skill uninstall iso-skill' })
    assert(skillUninstall.ok, 'metaagent should uninstall skill packages through kernel policy', skillUninstall.payload)
    log('skill-capabilities-passed')

    const credential = {
      id: 'iso-credential',
      description: 'Isolated metaagent capability drill credential handle',
      source: { type: 'vault', key: 'iso-credential' },
      allowed_hosts: [],
      allowed_uses: ['http'],
      injection: { kind: 'header', name: 'authorization', value: 'Bearer ${secret}' },
    }
    const credentialUpsert = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: `credential upsert-json '${JSON.stringify(credential)}'`,
    })
    assert(credentialUpsert.ok, 'metaagent should create credential handles without secret values', credentialUpsert.payload)
    const credentialGet = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'credential get iso-credential' })
    assert(credentialGet.ok, 'metaagent should inspect credential handle metadata', credentialGet.payload)
    const vaultStatus = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'credential vault status' })
    assert(vaultStatus.ok, 'metaagent should inspect vault status', vaultStatus.payload)
    const secretSentinel = 'super-secret-drill-value'
    const secretDenied = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: `credential set-secret iso-credential ${secretSentinel}` })
    assert(!secretDenied.ok, 'metaagent must not pass secret values through run_command', secretDenied.payload)
    assert(!JSON.stringify(secretDenied.payload ?? {}).includes(secretSentinel), 'secret denial payload must not echo raw secret values', secretDenied.payload)
    const credentialRemove = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'credential remove iso-credential' })
    assert(credentialRemove.ok, 'metaagent should remove credential handles', credentialRemove.payload)
    log('credential-capabilities-passed')

    const sliceDenied = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'slice list' })
    assert(!sliceDenied.ok, 'metaagent slice management must be denied', sliceDenied.payload)
    const sessionDenied = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'session new' })
    assert(!sessionDenied.ok, 'metaagent session creation must be denied', sessionDenied.payload)

    const interactionTitle = `Metaagent Capabilities Permission ${Date.now()}`
    const interactionPromise = client.send(requests.requestNativeProviderInteractionRequest(
      sessionId,
      worker.id,
      `metaagent-capabilities-interaction-${Date.now()}`,
      interactionTitle,
      'The isolated capabilities drill asks the metaagent to approve this owned worker interaction.',
      30,
    ))
    const interaction = await waitForInteraction(client, requests, sessionId, worker.id, interactionTitle, options.timeoutMs, options.pollMs)
    const resolution = await callRuntimeTool(metaRun, 'arroba.meta.resolve_runtime_interaction', {
      interaction_id: interaction.id,
      choice_id: 'allow_once',
    })
    assert(resolution.ok, 'metaagent should resolve owned worker interactions', resolution.payload)
    const interactionResult = unwrapVariant(await interactionPromise, 'NativeProviderInteractionResolved', 'RuntimeInteractionResolved')
    assert(
      interactionResult?.resolution?.choice_id === 'allow_once' || interactionResult?.choice_id === 'allow_once',
      'worker interaction should resolve with allow_once',
      interactionResult,
    )
    log('worker-interaction-resolution-passed')

    const completeTask = await callRuntimeTool(metaRun, 'arroba.meta.complete_task', { summary: 'Capability drill task finished.' })
    assert(completeTask.ok && completeTask.payload?.status === 'completed', 'metaagent should mark its task completed', completeTask.payload)

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'metaagent-capabilities-drill',
      kernelUrl,
      sessionId,
      metaagentId: metaagent.id,
      workerId: worker.id,
      workflowCode: workflowCodeSummary,
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) await client.close().catch(() => {})
    await cleanupSession(kernelUrl, sessionId)
    await terminateChild(daemon)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      preserveOnSuccess: options.preserveOnSuccess,
      failure,
      metadata: {
        drill: 'metaagent-capabilities',
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        kernelUrl,
        sessionId,
        workspace,
        scriptsDir,
        skillDir,
      },
      log,
    })
  }
  log('passed')
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exit(1)
})
