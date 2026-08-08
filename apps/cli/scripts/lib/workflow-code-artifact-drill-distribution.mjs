import { execFile, spawn } from 'node:child_process'
import { cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'
import { LocalIpcClient, applyWorkflowCodeArtifactRequest, applyWorkflowCodeRequest, createSessionRequest, endSessionRequest, importWorkflowCodePackageRequest, installSkillRequest, runWorkflowCodeArtifactRequest, runWorkflowCodeRequest, uninstallSkillRequest, validateWorkflowCodeRequest } from '@arroba/kernel-client'
import { assertHetznerArrobaBinaries, runHetznerCommand, shellQuote } from './native-tui-remote-execution.mjs'
import { assert, buildKernel, expectationFromDefinition, rebindingsForDefinition, repoRoot, sha256Hex, sleep, spawnedKernel, topologyRuntimeExpectation, unwrap, waitForKernel, writeSourceDirectoryExport } from './workflow-code-artifact-drill-runtime.mjs'
import { completeAppliedTopologyWorkflow, validateApplyResult, validateLiveExportedTopologyDefinition, validateSessionProjection } from './workflow-code-artifact-drill-topology.mjs'
import { remoteWorkflowCodeRunnerSource } from './workflow-code-artifact-drill-remote-runner.mjs'

const execFileAsync = promisify(execFile)

export { sha256Hex, writeSourceDirectoryExport }

export function validateArtifactHistory(artifact, expectedActions) {
  const actions = (artifact?.metadata?.history ?? []).map((entry) => entry.action)
  for (const action of expectedActions) {
    assert(actions.includes(action), `artifact history should include ${action}`, actions)
  }
}

export async function startIsolatedKernel(label, rootDir, workspace, worktree, timeoutMs) {
  await mkdir(workspace, { recursive: true })
  await mkdir(worktree, { recursive: true })
  const spawned = spawnedKernel(label, rootDir)
  const daemonChild = spawn(buildKernel(), [], {
    cwd: repoRoot,
    env: spawned.env,
    stdio: ['ignore', 'ignore', 'inherit'],
  })
  const client = new LocalIpcClient(spawned.kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    await waitForKernel(client, workspace, worktree, timeoutMs)
    return { client, daemonChild, kernelUrl: spawned.kernelUrl }
  } catch (error) {
    daemonChild.kill('SIGTERM')
    await client.close().catch(() => {})
    throw error
  }
}

export async function validateSourceExportOnKernel(client, session, nodePath, exportResult, label, timeoutMs) {
  assert(exportResult?.source, `${label} should include source`, exportResult)
  assert(
    sha256Hex(exportResult.source) === exportResult.source_sha256,
    `${label} source hash should match contents`,
    exportResult,
  )
  const validated = unwrap(
    await client.send(validateWorkflowCodeRequest(session.id, nodePath, exportResult.source)),
    'WorkflowCodeValidated',
  ).result
  assert(validated?.validation?.ok, `${label} should validate on second kernel`, validated?.validation)
  const expected = expectationFromDefinition(validated.definition)
  const endpointHandle = validated.definition.endpoints?.[0]?.handle
  assert(endpointHandle, `${label} should define an endpoint handle`, validated.definition)
  const appliedResponse = unwrap(
    await client.send(applyWorkflowCodeRequest(session.id, nodePath, exportResult.source)),
    'WorkflowCodeApplied',
  )
  const apply = validateApplyResult(appliedResponse.result, `${label} apply`, expected)
  validateSessionProjection(appliedResponse.session, apply, `${label} apply`, expected)
  const runResponse = unwrap(
    await client.send(runWorkflowCodeRequest(session.id, nodePath, exportResult.source, `Run ${label}.`, {
      endpoint: endpointHandle,
    })),
    'WorkflowCodeRun',
  )
  const runApply = validateApplyResult(runResponse.result.apply, `${label} run`, expected)
  validateSessionProjection(runResponse.session, runApply, `${label} run`, expected)
  assert(runResponse.result.invocation?.workflow_run || runResponse.result.invocation?.queued_prompt, `${label} should invoke or enqueue`, runResponse.result.invocation)
  assert(apply.workflow_id !== runApply.workflow_id, `${label} run should apply a fresh workflow`, { apply, runApply })
  return {
    validatedAlias: validated.definition.workflow?.alias ?? null,
    applyWorkflowId: apply.workflow_id,
    runWorkflowId: runApply.workflow_id,
    runInvocation: runResponse.result.invocation.workflow_run ? 'started' : 'enqueued',
    timeoutMs,
  }
}

export async function validateSecondKernelDistribution({
  packageExport,
  inlineSource,
  directorySource,
  sourceWorkflowId,
  skillSourceDir,
  skillName,
  generatedRoot,
  nodePath,
  timeoutMs,
}) {
  const secondRoot = path.join(generatedRoot, 'second-kernel')
  const secondWorkspace = path.join(secondRoot, 'workspace')
  const secondWorktree = path.join(secondRoot, 'worktree')
  const { client, daemonChild, kernelUrl } = await startIsolatedKernel(
    'workflow-code-second-kernel-drill',
    secondRoot,
    secondWorkspace,
    secondWorktree,
    timeoutMs,
  )
  let sessionId = null
  let installedSkill = false
  try {
    const session = unwrap(
      await client.send(createSessionRequest(secondWorkspace, secondWorktree, 'workflow-code-second-kernel-drill', undefined, null, 'off')),
      'SessionCreated',
    ).session
    sessionId = session.id
    const skillInstall = unwrap(
      await client.send(installSkillRequest(secondWorkspace, skillSourceDir)),
      'SkillInstalled',
    )
    assert(skillInstall?.skill?.name === skillName, 'second kernel should install workflow-code drill skill', skillInstall)
    installedSkill = true

    const importedName = `second-kernel-package-${Date.now()}`
    const imported = unwrap(
      await client.send(importWorkflowCodePackageRequest(session.id, packageExport, nodePath, {
        name: importedName,
        overwrite: false,
      })),
      'WorkflowCodePackageImported',
    ).artifact
    assert(imported?.metadata?.validation?.ok, 'second kernel package import should validate', imported?.metadata?.validation)
    const packageExpected = expectationFromDefinition(packageExport.definition)
    const packageRebindings = rebindingsForDefinition(packageExport.definition)
    const packageApplyResponse = unwrap(
      await client.send(applyWorkflowCodeArtifactRequest(session.id, importedName, packageRebindings)),
      'WorkflowCodeApplied',
    )
    const packageApply = validateApplyResult(packageApplyResponse.result, 'second kernel package', packageExpected)
    validateSessionProjection(packageApplyResponse.session, packageApply, 'second kernel package', packageExpected)
    const packageRun = unwrap(
      await client.send(runWorkflowCodeArtifactRequest(session.id, importedName, 'Run the second-kernel package workflow.', {
        endpoint: 'entry',
        providerRebindings: packageRebindings,
      })),
      'WorkflowCodeRun',
    )
    const packageRunApply = validateApplyResult(packageRun.result.apply, 'second kernel package run', packageExpected)
    validateSessionProjection(packageRun.session, packageRunApply, 'second kernel package run', packageExpected)
    assert(packageApply.workflow_id !== sourceWorkflowId, 'second kernel package workflow id must be fresh', { packageApply, sourceWorkflowId })
    assert(packageRunApply.workflow_id !== sourceWorkflowId, 'second kernel package run workflow id must be fresh', { packageRunApply, sourceWorkflowId })

    const inline = await validateSourceExportOnKernel(
      client,
      session,
      nodePath,
      inlineSource,
      'second kernel inline source',
      timeoutMs,
    )

    const directoryManifest = await writeSourceDirectoryExport(
      secondWorkspace,
      directorySource,
      'second kernel source directory',
    )
    const directory = await validateSourceExportOnKernel(
      client,
      session,
      nodePath,
      directorySource,
      'second kernel source directory',
      timeoutMs,
    )

    return {
      kernelUrl,
      sessionId: session.id,
      packageImportedArtifact: importedName,
      packageApplyWorkflowId: packageApply.workflow_id,
      packageRunWorkflowId: packageRunApply.workflow_id,
      inline,
      directory,
      directoryManifest,
    }
  } finally {
    if (client && sessionId) {
      if (installedSkill) {
        await client.send(uninstallSkillRequest(secondWorkspace, skillName)).catch(() => {})
      }
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
    daemonChild.kill('SIGTERM')
    await sleep(1000)
  }
}

export async function validateTopologySourceExportOnKernel(client, session, nodePath, workspace, exportResult, exampleName, label, timeoutMs) {
  assert(exportResult?.source, `${label} should include source`, exportResult)
  assert(
    sha256Hex(exportResult.source) === exportResult.source_sha256,
    `${label} source hash should match contents`,
    exportResult,
  )
  const validated = unwrap(
    await client.send(validateWorkflowCodeRequest(session.id, nodePath, exportResult.source)),
    'WorkflowCodeValidated',
  ).result
  assert(validated?.validation?.ok, `${label} should validate on second kernel`, validated?.validation)
  validateLiveExportedTopologyDefinition(exampleName, validated.definition, validated.validation)
  const appliedResponse = unwrap(
    await client.send(applyWorkflowCodeRequest(session.id, nodePath, exportResult.source)),
    'WorkflowCodeApplied',
  )
  const expected = topologyRuntimeExpectation(validated.definition)
  const apply = validateApplyResult(appliedResponse.result, `${label} apply`, expected)
  validateSessionProjection(appliedResponse.session, apply, `${label} apply`, expected)
  assert(apply.workflow_id !== exportResult.source_workflow_id, `${label} apply workflow id must be fresh`, apply)
  const completed = await completeAppliedTopologyWorkflow(
    client,
    session.id,
    exampleName,
    validated.definition,
    apply,
    timeoutMs,
  )
  return {
    applyWorkflowId: apply.workflow_id,
    workflowRunId: completed.workflowRunId,
    runtime: completed.runtime,
    tuiOutline: completed.tuiOutline,
    sourceSha256: exportResult.source_sha256,
    directoryManifest: exportResult.format === 'directory'
      ? await writeSourceDirectoryExport(workspace, exportResult, label)
      : null,
  }
}

export async function validateSecondKernelExampleSuite({
  liveExports,
  generatedRoot,
  nodePath,
  timeoutMs,
}) {
  if ((liveExports ?? []).length === 0) return null
  const secondRoot = path.join(generatedRoot, 'second-kernel-example-suite')
  const secondWorkspace = path.join(secondRoot, 'workspace')
  const secondWorktree = path.join(secondRoot, 'worktree')
  const { client, daemonChild, kernelUrl } = await startIsolatedKernel(
    'workflow-code-second-kernel-example-suite-drill',
    secondRoot,
    secondWorkspace,
    secondWorktree,
    timeoutMs,
  )
  let sessionId = null
  try {
    const session = unwrap(
      await client.send(createSessionRequest(secondWorkspace, secondWorktree, 'workflow-code-second-kernel-example-suite-drill', undefined, null, 'off')),
      'SessionCreated',
    ).session
    sessionId = session.id
    const examples = []
    for (const liveExport of liveExports) {
      const inlineSource = {
        ...liveExport.inlineSource,
        source_workflow_id: liveExport.sourceWorkflowId,
      }
      const directorySource = {
        ...liveExport.directorySource,
        source_workflow_id: liveExport.sourceWorkflowId,
      }
      const inline = await validateTopologySourceExportOnKernel(
        client,
        session,
        nodePath,
        secondWorkspace,
        inlineSource,
        liveExport.exampleName,
        `second kernel ${liveExport.exampleName} live inline source`,
        timeoutMs,
      )
      await writeSourceDirectoryExport(
        secondWorkspace,
        directorySource,
        `second kernel ${liveExport.exampleName} live source directory`,
      )
      const directory = await validateTopologySourceExportOnKernel(
        client,
        session,
        nodePath,
        secondWorkspace,
        directorySource,
        liveExport.exampleName,
        `second kernel ${liveExport.exampleName} live source directory`,
        timeoutMs,
      )
      examples.push({
        example: liveExport.exampleName,
        sourceWorkflowId: liveExport.sourceWorkflowId,
        inline,
        directory,
      })
    }
    return {
      kernelUrl,
      sessionId: session.id,
      examples,
    }
  } finally {
    if (client && sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
    daemonChild.kill('SIGTERM')
    await sleep(1000)
  }
}

export async function validateHetznerSecondKernelDistribution({
  packageExport,
  inlineSource,
  directorySource,
  sourceWorkflowId,
  liveExports,
  skillSourceDir,
  skillName,
  generatedRoot,
  timeoutMs,
  options,
}) {
  const localBundle = path.join(generatedRoot, 'hetzner-second-kernel-bundle')
  const remoteBundle = path.posix.join(
    options.hetznerRemoteRoot,
    `workflow-code-second-kernel-${process.pid}-${Date.now()}`,
  )
  await writeHetznerWorkflowCodeBundle(localBundle, {
    packageExport,
    inlineSource,
    directorySource,
    sourceWorkflowId,
    liveExports,
    skillName,
    skillSourceDir,
  })
  await assertHetznerCheckoutMatchesLocal(options)
  await assertHetznerArrobaBinaries({
    hetznerHost: options.hetznerHost,
    hetznerKey: options.hetznerKey,
    hetznerRepo: options.hetznerRepo,
  })
  await runHetznerCommand(options, [
    `rm -rf ${shellQuote(remoteBundle)}`,
    `mkdir -p ${shellQuote(remoteBundle)}`,
  ].join(' && '))
  await execFileAsync('scp', [
    '-i',
    options.hetznerKey,
    '-o',
    'BatchMode=yes',
    '-o',
    'StrictHostKeyChecking=accept-new',
    '-r',
    `${localBundle}/.`,
    `${options.hetznerHost}:${remoteBundle}/`,
  ], { maxBuffer: 4 * 1024 * 1024 })
  try {
    const command = [
      `cd ${shellQuote(options.hetznerRepo)}`,
      `runner_cmd=${shellQuote(`node ${shellQuote(path.posix.join(remoteBundle, 'remote-runner.mjs'))} --repo ${shellQuote(options.hetznerRepo)} --bundle ${shellQuote(remoteBundle)} --timeout-ms ${Number(timeoutMs)}`)}`,
      `if command -v timeout >/dev/null 2>&1; then timeout --kill-after=5s ${Math.ceil(Number(timeoutMs) / 1000) + 90}s bash -lc "$runner_cmd"; else bash -lc "$runner_cmd"; fi`,
    ].join(' && ')
    const stdout = await runHetznerCommand(options, command)
    const jsonStart = stdout.lastIndexOf('\n{')
    const summaryText = (jsonStart >= 0 ? stdout.slice(jsonStart + 1) : stdout).trim()
    const summary = JSON.parse(summaryText)
    assert(summary?.packageApplyWorkflowId !== sourceWorkflowId, 'Hetzner package apply workflow id must be fresh', summary)
    assert(summary?.inline?.applyWorkflowId !== sourceWorkflowId, 'Hetzner inline source workflow id must be fresh', summary)
    assert(summary?.directory?.applyWorkflowId !== sourceWorkflowId, 'Hetzner directory source workflow id must be fresh', summary)
    return {
      remoteHost: options.hetznerHost,
      remoteRepo: options.hetznerRepo,
      remoteBundle,
      ...summary,
    }
  } finally {
    await runHetznerCommand(options, `rm -rf ${shellQuote(remoteBundle)}`).catch(() => {})
  }
}

export async function assertHetznerCheckoutMatchesLocal(options) {
  const [{ stdout: localHeadRaw }, { stdout: localCommitEpochRaw }] = await Promise.all([
    execFileAsync('git', ['rev-parse', 'HEAD'], { cwd: repoRoot }),
    execFileAsync('git', ['show', '-s', '--format=%ct', 'HEAD'], { cwd: repoRoot }),
  ])
  const localHead = localHeadRaw.trim()
  const localCommitEpoch = Number(localCommitEpochRaw.trim())
  const remoteInfo = await runHetznerCommand(options, [
    `cd ${shellQuote(options.hetznerRepo)}`,
    `printf 'head=%s\\n' "$(git rev-parse HEAD)"`,
    `printf 'kernel_mtime=%s\\n' "$(stat -c %Y apps/kernel/target/debug/arroba-kernel 2>/dev/null || printf 0)"`,
  ].join(' && '))
  const info = Object.fromEntries(remoteInfo
    .trim()
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      const separator = line.indexOf('=')
      return [line.slice(0, separator), line.slice(separator + 1)]
    }))
  if (info.head !== localHead) {
    throw new Error([
      `Hetzner checkout ${options.hetznerRepo} is not on the local workflow-code validation commit.`,
      `local HEAD: ${localHead}`,
      `remote HEAD: ${info.head ?? 'unknown'}`,
      'Update/build the remote checkout before running --hetzner-second-kernel.',
    ].join('\n'))
  }
  const kernelMtime = Number(info.kernel_mtime ?? 0)
  if (!Number.isFinite(kernelMtime) || kernelMtime < localCommitEpoch) {
    throw new Error([
      `Hetzner kernel binary is older than local HEAD at ${options.hetznerRepo}.`,
      `local HEAD epoch: ${localCommitEpoch}`,
      `remote kernel mtime: ${info.kernel_mtime ?? 'unknown'}`,
      'Rebuild apps/kernel/target/debug/arroba-kernel on Hetzner before running --hetzner-second-kernel.',
    ].join('\n'))
  }
}

export async function writeHetznerWorkflowCodeBundle(localBundle, {
  packageExport,
  inlineSource,
  directorySource,
  sourceWorkflowId,
  liveExports,
  skillName,
  skillSourceDir,
}) {
  await rm(localBundle, { recursive: true, force: true })
  await mkdir(localBundle, { recursive: true })
  await writeFile(path.join(localBundle, 'package-export.json'), JSON.stringify(packageExport, null, 2), 'utf8')
  await writeFile(path.join(localBundle, 'package.json'), JSON.stringify({ type: 'module' }, null, 2), 'utf8')
  await writeFile(path.join(localBundle, 'inline-source.json'), JSON.stringify(inlineSource, null, 2), 'utf8')
  await writeFile(path.join(localBundle, 'directory-source.json'), JSON.stringify(directorySource, null, 2), 'utf8')
  await writeFile(path.join(localBundle, 'topology-live-exports.json'), JSON.stringify(liveExports ?? [], null, 2), 'utf8')
  await writeFile(path.join(localBundle, 'metadata.json'), JSON.stringify({ sourceWorkflowId, skillName }, null, 2), 'utf8')
  await copyDirectory(skillSourceDir, path.join(localBundle, 'skill', path.basename(skillSourceDir)))
  await cp(path.join(repoRoot, 'packages/kernel-client/dist'), path.join(localBundle, 'kernel-client-dist'), {
    recursive: true,
    force: true,
  })
  await mkdir(path.join(localBundle, 'node_modules'), { recursive: true })
  await cp(path.join(repoRoot, 'packages/kernel-client/node_modules/ws'), path.join(localBundle, 'node_modules/ws'), {
    recursive: true,
    dereference: true,
    force: true,
  })
  await writeFile(path.join(localBundle, 'remote-runner.mjs'), remoteWorkflowCodeRunnerSource(), 'utf8')
}

export async function copyDirectory(sourceDir, targetDir) {
  await mkdir(targetDir, { recursive: true })
  for (const entry of await readdir(sourceDir, { withFileTypes: true })) {
    const sourcePath = path.join(sourceDir, entry.name)
    const targetPath = path.join(targetDir, entry.name)
    if (entry.isDirectory()) {
      await copyDirectory(sourcePath, targetPath)
    } else if (entry.isFile()) {
      await writeFile(targetPath, await readFile(sourcePath), 'utf8')
    }
  }
}
