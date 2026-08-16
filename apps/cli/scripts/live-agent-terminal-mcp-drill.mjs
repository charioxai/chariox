#!/usr/bin/env node
import { mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const root = path.join(os.tmpdir(), `chariox-agent-terminal-${process.pid}-${Date.now()}`)
const workspace = path.join(root, 'workspace')
const kernelPort = 49000 + Math.floor(Math.random() * 500)
const kernelUrl = `ws://127.0.0.1:${kernelPort}`

function start(command, args, env, stdio = ['ignore', 'pipe', 'pipe']) {
  const child = spawn(command, args, { cwd: repoRoot, env, stdio })
  let stderr = ''
  child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
  return { child, get stderr() { return stderr } }
}

async function waitForKernel(LocalIpcClient, listSessionsRequest) {
  const deadline = Date.now() + 20_000
  let lastError
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close()
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

function sendJsonLine(child, request) {
  return new Promise((resolve, reject) => {
    let buffer = ''
    const onData = (chunk) => {
      buffer += chunk.toString()
      const newline = buffer.indexOf('\n')
      if (newline < 0) return
      const line = buffer.slice(0, newline)
      child.stdout.off('data', onData)
      try { resolve(JSON.parse(line)) } catch (error) { reject(error) }
    }
    child.stdout.on('data', onData)
    child.stdin.write(`${JSON.stringify(request)}\n`)
  })
}

function assert(condition, message, details) {
  if (!condition) throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
}

async function main() {
  await mkdir(workspace, { recursive: true })
  const env = {
    ...process.env,
    HOME: path.join(root, 'home'),
    CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(kernelPort + 1),
    CHARIOX_OPENCODE_PORT: String(kernelPort + 2),
    CHARIOX_CODEX_PORT: String(kernelPort + 3),
    CHARIOX_DAEMON_ID: `agent-terminal-drill-${process.pid}`,
    CHARIOX_SESSION_HISTORY_DIR: path.join(root, 'history'),
    CHARIOX_TEST_TUI: '1',
  }
  let kernel
  let peer
  let client
  try {
    const [{ LocalIpcClient }, requests] = await Promise.all([
      import('../../../packages/kernel-client/dist/ipc.js'),
      import('../../../packages/kernel-client/dist/ipc-requests.js'),
    ])
    kernel = start(path.join(repoRoot, 'target', 'debug', 'chariox-kernel'), [], env)
    await waitForKernel(LocalIpcClient, requests.listSessionsRequest)
    peer = start(process.execPath, [path.join(repoRoot, 'apps/shell/dist/agent-terminal-main.js')], {
      ...env,
      CHARIOX_KERNEL_URL: kernelUrl,
    }, ['pipe', 'pipe', 'pipe'])
    const initialized = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 1, method: 'initialize' })
    assert(initialized.result?.serverInfo?.name === 'chariox-agent-terminal', 'MCP initialize failed', initialized)
    assert(/chariox_search/.test(initialized.result?.instructions ?? '') && /explicit workspace, worktree/.test(initialized.result?.instructions ?? ''), 'MCP initialize did not publish agent-terminal usage instructions', initialized)
    const listed = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 2, method: 'tools/list' })
    const names = listed.result?.tools?.map((tool) => tool.name) ?? []
    assert(JSON.stringify(names) === JSON.stringify(['chariox_search', 'chariox_describe', 'chariox_execute', 'chariox_wait', 'chariox_status']), 'MCP tool list drifted', names)
    const search = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name: 'chariox_search', arguments: { query: 'workflow runs', limit: 20 } } })
    const searchPayload = JSON.parse(search.result.content[0].text)
    assert(searchPayload.results.length <= 20, 'agent search exceeded requested bound', searchPayload)
    const workflowOperation = searchPayload.results.find((result) => result.id === 'workflow-runs')
    assert(workflowOperation, 'canonical registry did not expose the workflow runs operation', searchPayload)
    const described = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 31, method: 'tools/call', params: { name: 'chariox_describe', arguments: { operation_id: workflowOperation.id } } })
    const describedPayload = JSON.parse(described.result.content[0].text)
    assert(describedPayload.operation?.id === workflowOperation.id && describedPayload.operation?.input_schema, 'operation description was incomplete', describedPayload)
    assert(
      describedPayload.operation?.search_aliases?.length > 0
        && describedPayload.operation?.intents?.length > 0
        && describedPayload.operation?.examples?.length > 0,
      'operation description must include bounded discovery metadata',
      describedPayload,
    )
    const domainQueries = [
      ['credential vault', /credential|vault/i],
      ['slice lifecycle', /slice/i],
      ['remote machine', /remote|machine/i],
      ['publication deployment', /publication|deployment/i],
      ['session history', /history|outline/i],
    ]
    for (const [query, matcher] of domainQueries) {
      const domainSearch = await sendJsonLine(peer.child, {
        jsonrpc: '2.0',
        id: `domain-${query}`,
        method: 'tools/call',
        params: { name: 'chariox_search', arguments: { query, limit: 5 } },
      })
      const domainPayload = JSON.parse(domainSearch.result.content[0].text)
      assert(domainPayload.results.length > 0 && domainPayload.results.some((result) => matcher.test(`${result.id} ${result.description}`)), `agent search returned no bounded ${query} domain result`, domainPayload)
      const domainOperation = domainPayload.results[0]
      const domainDescription = await sendJsonLine(peer.child, {
        jsonrpc: '2.0',
        id: `describe-${query}`,
        method: 'tools/call',
        params: { name: 'chariox_describe', arguments: { operation_id: domainOperation.id } },
      })
      const domainDescriptionPayload = JSON.parse(domainDescription.result.content[0].text)
      assert(domainDescriptionPayload.operation?.id === domainOperation.id && domainDescriptionPayload.operation?.input_schema, `agent describe returned no schema for ${query}`, domainDescriptionPayload)
    }
    // The filesystem roots and kernel resource identifiers are distinct
    // context fields, even though this disposable kernel uses the same path
    // values for both identities.
    const context = { workspace, worktree: workspace, workspace_id: workspace, worktree_id: workspace }
    const healthSearch = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 34, method: 'tools/call', params: { name: 'chariox_search', arguments: { query: 'daemon health', limit: 5 } } })
    const healthOperation = JSON.parse(healthSearch.result.content[0].text).results.find((result) => result.id === 'kernel-health')
    assert(healthOperation, 'human-equivalent kernel health operation was not searchable', healthSearch)
    const healthRun = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 35, method: 'tools/call', params: { name: 'chariox_execute', arguments: { operation_id: healthOperation.id, registry_revision: searchPayload.revision, context } } })
    const healthPayload = JSON.parse(healthRun.result.content[0].text)
    assert(healthPayload.ok && healthPayload.registry_revision === searchPayload.revision && /remote runtime authority|projection invariants/i.test(healthPayload.output), 'kernel health operation did not execute with the searched registry revision', healthPayload)
    const structuredCreate = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 36, method: 'tools/call', params: { name: 'chariox_execute', arguments: { operation_id: 'terminal.create_session', input: { alias: 'structured-agent-session' }, context } } })
    const structuredCreatePayload = JSON.parse(structuredCreate.result.content[0].text)
    assert(structuredCreatePayload.ok && /SessionCreated|session/i.test(structuredCreatePayload.output), 'generated structured session contract did not execute', structuredCreatePayload)
    const created = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 4, method: 'tools/call', params: { name: 'chariox_execute', arguments: { command: 'session new --dir . as shared', context } } })
    const createdPayload = JSON.parse(created.result.content[0].text)
    assert(createdPayload.ok, 'agent session creation failed', createdPayload)
    assert(createdPayload.context.session_id && createdPayload.context.attachment_id, 'agent session did not return explicit attachment context', createdPayload)
    const status = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 32, method: 'tools/call', params: { name: 'chariox_status', arguments: { context: createdPayload.context } } })
    const statusPayload = JSON.parse(status.result.content[0].text)
    assert(statusPayload.connected && statusPayload.registry_revision, 'agent status did not return registry state', statusPayload)
    const operationRun = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 33, method: 'tools/call', params: { name: 'chariox_execute', arguments: { operation_id: workflowOperation.id, context: createdPayload.context } } })
    const operationRunPayload = JSON.parse(operationRun.result.content[0].text)
    assert(operationRunPayload.ok && /workflow/i.test(operationRunPayload.output), 'structured operation execution failed', operationRunPayload)
    client = new LocalIpcClient(kernelUrl)
    const sessions = await client.send(requests.listSessionsRequest())
    const sessionRecords = sessions.SessionsListed?.sessions ?? sessions.Sessions?.sessions ?? []
    assert(sessionRecords.some((session) => session.id === createdPayload.context.session_id), 'other terminal did not observe agent-created session', sessions)
    const workflow = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 6, method: 'tools/call', params: { name: 'chariox_execute', arguments: { command: 'workflow new shared-agent-flow as workflow', context: createdPayload.context } } })
    const workflowPayload = JSON.parse(workflow.result.content[0].text)
    assert(workflowPayload.ok && workflowPayload.context.workflow_id, 'agent workflow creation failed', workflowPayload)
    const structuredWorkflow = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 37, method: 'tools/call', params: { name: 'chariox_execute', arguments: { operation_id: 'terminal.create_workflow', input: { alias: 'structured-agent-flow' }, context: createdPayload.context } } })
    const structuredWorkflowPayload = JSON.parse(structuredWorkflow.result.content[0].text)
    assert(structuredWorkflowPayload.ok && /WorkflowCreated|workflow/i.test(structuredWorkflowPayload.output), 'generated structured workflow contract did not execute', structuredWorkflowPayload)
    const workflowList = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 7, method: 'tools/call', params: { name: 'chariox_execute', arguments: { command: 'workflow list', context: workflowPayload.context } } })
    const workflowListPayload = JSON.parse(workflowList.result.content[0].text)
    assert(workflowListPayload.ok && workflowListPayload.output.includes('shared-agent-flow'), 'agent workflow list did not observe its own workflow', workflowListPayload)
    const readOperations = [
      ['ListSessions', undefined, context],
      ['GetDaemonHealth', undefined, context],
      ['GetProviderCatalog', undefined, context],
      ['ListCredentials', undefined, context],
      ['GetCredentialVaultStatus', undefined, context],
      ['ListSlices', undefined, context],
      ['ListRemoteMachines', undefined, context],
      ['GetSessionHistoryOutline', { latest_prompt_count: 5 }, createdPayload.context],
      ['ListWorkflows', undefined, createdPayload.context],
      ['ListWorkflowPublications', undefined, createdPayload.context],
    ]
    for (const [variant, input, operationContext] of readOperations) {
      const operationId = `terminal.${variant.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`).replace(/^_/, '')}`
      const read = await sendJsonLine(peer.child, {
        jsonrpc: '2.0',
        id: `read-${variant}`,
        method: 'tools/call',
        params: { name: 'chariox_execute', arguments: { operation_id: operationId, ...(input === undefined ? {} : { input }), context: operationContext } },
      })
      const readPayload = JSON.parse(read.result.content[0].text)
      assert(readPayload.ok, `agent terminal read operation ${variant} failed`, readPayload)
    }
    const oldKernel = kernel
    oldKernel.child.kill('SIGKILL')
    await new Promise((resolve) => oldKernel.child.once('exit', resolve))
    kernel = start(path.join(repoRoot, 'target', 'debug', 'chariox-kernel'), [], env)
    await waitForKernel(LocalIpcClient, requests.listSessionsRequest)
    const recoveredWorkflows = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 38, method: 'tools/call', params: { name: 'chariox_execute', arguments: { command: 'workflow list', context: workflowPayload.context } } })
    const recoveredWorkflowsPayload = JSON.parse(recoveredWorkflows.result.content[0].text)
    assert(recoveredWorkflowsPayload.ok && /shared-agent-flow|structured-agent-flow/i.test(recoveredWorkflowsPayload.output), 'agent terminal did not recover durable workflow state after kernel restart', recoveredWorkflowsPayload)
    const recoveredMutation = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 39, method: 'tools/call', params: { name: 'chariox_execute', arguments: { operation_id: 'terminal.create_workflow', input: { alias: 'recovered-agent-flow' }, context: workflowPayload.context } } })
    const recoveredMutationPayload = JSON.parse(recoveredMutation.result.content[0].text)
    assert(recoveredMutationPayload.ok && /WorkflowCreated|workflow/i.test(recoveredMutationPayload.output), 'agent terminal did not recover a stale attachment for a post-restart mutation', recoveredMutationPayload)
    const shellScriptPath = path.join(root, 'mixed-shell.chariox')
    await writeFile(shellScriptPath, `session use ${createdPayload.context.session_id}\nworkflow new mixed-shell-flow as workflow\n`, 'utf8')
    const shell = start(process.execPath, [path.join(repoRoot, 'apps/shell/dist/shell.js'), 'run', shellScriptPath, '--kernel-url', kernelUrl, '--workspace', workspace, '--worktree', workspace], env, ['pipe', 'pipe', 'pipe'])
    let shellOutput = ''
    let shellError = ''
    shell.child.stdout.on('data', (chunk) => { shellOutput += chunk.toString() })
    shell.child.stderr.on('data', (chunk) => { shellError += chunk.toString() })
    shell.child.stdin.end()
    const shellExit = await new Promise((resolve) => shell.child.once('exit', (code) => resolve(code)))
    assert(shellExit === 0, 'second terminal shell workflow mutation failed', { shellOutput, shellError })
    assert(shellOutput.includes('mixed-shell-flow'), 'second terminal shell did not report its workflow mutation', { shellOutput, shellError })
    const agentWorkflowList = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 8, method: 'tools/call', params: { name: 'chariox_execute', arguments: { command: 'workflow list', context: workflowPayload.context } } })
    const agentWorkflowListPayload = JSON.parse(agentWorkflowList.result.content[0].text)
    assert(agentWorkflowListPayload.ok && agentWorkflowListPayload.output.includes('mixed-shell-flow'), 'agent terminal did not observe shell-created workflow', agentWorkflowListPayload)
    const sharedState = await client.send(requests.getSessionStateRequest(createdPayload.context.session_id))
    const sharedSession = sharedState.SessionState?.session ?? sharedState.SessionStateLoaded?.session
    assert(sharedSession?.workflows?.some((entry) => entry.alias === 'shared-agent-flow' || entry.name === 'shared-agent-flow'), 'other terminal did not observe agent-created workflow', sharedState)
    assert(sharedSession?.workflows?.some((entry) => entry.alias === 'mixed-shell-flow' || entry.name === 'mixed-shell-flow'), 'other terminal did not observe shell-created workflow', sharedState)
    const denied = await sendJsonLine(peer.child, { jsonrpc: '2.0', id: 5, method: 'tools/call', params: { name: 'chariox_execute', arguments: { command: 'agent focus some-agent', context: createdPayload.context } } })
    assert(denied.result?.isError === true, 'agent focus unexpectedly succeeded', denied)
    console.log(JSON.stringify({ ok: true, tools: names, catalog_results: searchPayload.results.length, session_id: createdPayload.context.session_id }))
  } finally {
    await client?.close().catch(() => {})
    peer?.child.stdin?.end()
    peer?.child.kill('SIGTERM')
    kernel?.child.kill('SIGTERM')
    await rm(root, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exitCode = 1
})
