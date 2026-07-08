#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, rm } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { LocalIpcClient, applyWorkflowCodeArtifactRequest, attachToSessionRequest, createSessionRequest, createWorkflowCodeArtifactRequest, deleteWorkflowCodeArtifactRequest, endSessionRequest, exportWorkflowCodePackageRequest, exportWorkflowCodeSourceRequest, getSessionStateRequest, getWorkflowCodeArtifactRequest, importWorkflowCodeArtifactRequest, installSkillRequest, listWorkflowCodeArtifactsRequest, runWorkflowCodeArtifactRequest, uninstallSkillRequest, updateWorkflowCodeArtifactRequest } from '@arroba/kernel-client'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'
import { WORKFLOW_CODE_ARTIFACT_SKILL_PREFIX, assert, buildKernel, cliRoot, defaultToyExpectation, parseArgs, providerRebindings, repoRoot, sleep, spawnedKernel, unwrap, waitForKernel, workflowCodeSource } from './lib/workflow-code-artifact-drill-runtime.mjs'
import { validateArtifactHistory, validateHetznerSecondKernelDistribution, validateSecondKernelDistribution, validateSecondKernelExampleSuite } from './lib/workflow-code-artifact-drill-distribution.mjs'
import { applyExampleSuite, runRealProviderTopologyDrill, validateApplyResult, validateSessionProjection, writeWorkflowCodeArtifactSkillSource } from './lib/workflow-code-artifact-drill-topology.mjs'
import { applyExistingAgentArtifact, applyOutputSchemaArtifact } from './lib/workflow-code-artifact-drill-artifacts.mjs'

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const skillName = `${WORKFLOW_CODE_ARTIFACT_SKILL_PREFIX}-${process.pid}-${Date.now()}`
  const source = workflowCodeSource(skillName)
  if (options.dryRun) {
    console.log(JSON.stringify({
      artifactRoot: options.artifactRoot,
      spawnDaemon: options.spawnDaemon,
      kernel: options.kernel,
      skillName,
      source,
      providerRebindings: providerRebindings(),
      exampleSuite: options.exampleSuite,
      secondKernel: options.secondKernel,
      hetznerSecondKernel: options.hetznerSecondKernel,
      realProviderTopology: options.realProviderTopology,
      providerIds: options.providerIds,
      providerModels: options.providerModels,
      providerAccounts: options.providerAccounts,
      providerEfforts: options.providerEfforts,
      hetznerHost: options.hetznerHost,
      hetznerRepo: options.hetznerRepo,
      hetznerRemoteRoot: options.hetznerRemoteRoot,
    }, null, 2))
    return
  }

  await prepareDrillArtifacts(options.artifactRoot)
  const generatedRoot = path.join(repoRoot, 'target', 'workflow-code-artifact-drill', `${process.pid}-${Date.now()}`)
  const workspace = options.workspace ?? path.join(generatedRoot, 'workspace')
  const worktree = options.worktree ?? path.join(generatedRoot, 'worktree')
  const skillSourceRoot = path.join(generatedRoot, 'skills')
  await mkdir(workspace, { recursive: true })
  await mkdir(worktree, { recursive: true })
  const skillSourceDir = await writeWorkflowCodeArtifactSkillSource(skillSourceRoot, skillName)

  let passed = false
  let failure = null
  let daemonChild = null
  let sessionId = null
  let attachmentId = null
  let kernelUrl = options.kernel ?? 'ws://127.0.0.1:43284'
  let summary = {}
  let client = null
  let installedSkill = false

  try {
    if (options.spawnDaemon) {
      const spawned = spawnedKernel('workflow-code-drill', generatedRoot)
      kernelUrl = spawned.kernelUrl
      daemonChild = spawn(buildKernel(), [], {
        cwd: repoRoot,
        env: spawned.env,
        stdio: ['ignore', 'ignore', 'inherit'],
      })
    }

    client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await waitForKernel(client, workspace, worktree, options.timeoutMs)

    const session = unwrap(
      await client.send(createSessionRequest(workspace, worktree, 'workflow-code-artifact-drill', undefined, null, 'off')),
      'SessionCreated',
    ).session
    sessionId = session.id
    const attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `workflow-code-artifact-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    attachmentId = attachment.id

    const skillInstall = unwrap(
      await client.send(installSkillRequest(workspace, skillSourceDir)),
      'SkillInstalled',
    )
    assert(skillInstall?.skill?.name === skillName, 'workflow-code artifact drill skill should install', skillInstall)
    installedSkill = true

    const artifactName = `artifact-drill-${Date.now()}`
    const importedName = `${artifactName}-imported`
    const nodePath = process.execPath

    const created = unwrap(
      await client.send(createWorkflowCodeArtifactRequest(session.id, artifactName, nodePath, source)),
      'WorkflowCodeArtifactCreated',
    ).artifact
    assert(created?.metadata?.validation?.ok, 'created artifact should validate', created?.metadata?.validation)
    validateArtifactHistory(created, ['created'])

    const listedAfterCreate = unwrap(
      await client.send(listWorkflowCodeArtifactsRequest(session.id)),
      'WorkflowCodeArtifactsListed',
    ).artifacts
    assert(
      (listedAfterCreate ?? []).some((artifact) => artifact.name === artifactName),
      'created artifact should appear in workflow-code artifact list',
      listedAfterCreate,
    )

    const updatedSource = source.replace('workflow_code_artifact_drill', 'workflow_code_artifact_drill_updated')
    const updated = unwrap(
      await client.send(updateWorkflowCodeArtifactRequest(session.id, artifactName, nodePath, updatedSource)),
      'WorkflowCodeArtifactUpdated',
    ).artifact
    assert(updated?.metadata?.validation?.ok, 'updated artifact should validate', updated?.metadata?.validation)
    assert(
      updated?.definition?.workflow?.alias === 'workflow_code_artifact_drill_updated',
      'updated artifact should persist revised workflow-code source',
      updated,
    )
    validateArtifactHistory(updated, ['created', 'updated'])

    const appliedResponse = unwrap(
      await client.send(applyWorkflowCodeArtifactRequest(session.id, artifactName, providerRebindings())),
      'WorkflowCodeApplied',
    )
    const defaultExpected = defaultToyExpectation(skillName)
    const firstApply = validateApplyResult(appliedResponse.result, 'artifact apply', defaultExpected)
    validateSessionProjection(appliedResponse.session, firstApply, 'artifact apply', defaultExpected)

    const exported = unwrap(
      await client.send(exportWorkflowCodePackageRequest(session.id, artifactName)),
      'WorkflowCodePackageExported',
    ).package
    assert(exported?.source_sha256, 'exported package should include source hash', exported)
    assert(exported?.definition_sha256, 'exported package should include compiled definition hash', exported)

    const liveInlineSource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        session.id,
        { kind: 'workflow', workflow_ref: firstApply.workflow_id },
        'inline',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(liveInlineSource?.source, 'live workflow inline source export should include source', liveInlineSource)
    assert(liveInlineSource.source_path === 'workflow.js', 'live workflow inline source path should be workflow.js', liveInlineSource)

    const liveDirectorySource = unwrap(
      await client.send(exportWorkflowCodeSourceRequest(
        session.id,
        { kind: 'workflow', workflow_ref: firstApply.workflow_id },
        'directory',
      )),
      'WorkflowCodeSourceExported',
    ).export
    assert(liveDirectorySource?.source_path === 'workflow.js', 'live workflow directory source export should include workflow.js', liveDirectorySource)
    assert(
      (liveDirectorySource.files ?? []).some((file) => file.path === 'manifest.json'),
      'live workflow directory source export should include manifest.json',
      liveDirectorySource,
    )

    await client.send(deleteWorkflowCodeArtifactRequest(session.id, artifactName))

    const imported = unwrap(
      await client.send(importWorkflowCodeArtifactRequest(session.id, exported, nodePath, {
        name: importedName,
        overwrite: false,
      })),
      'WorkflowCodeArtifactImported',
    ).artifact
    assert(imported?.metadata?.validation?.ok, 'imported artifact should validate', imported?.metadata?.validation)

    const outputSchemaArtifact = await applyOutputSchemaArtifact(client, session, nodePath, options.timeoutMs)

    const runResponse = unwrap(
      await client.send(runWorkflowCodeArtifactRequest(session.id, importedName, 'Run the imported workflow-code artifact.', {
        endpoint: 'entry',
        providerRebindings: providerRebindings(),
      })),
      'WorkflowCodeRun',
    )
    const runApply = validateApplyResult(runResponse.result.apply, 'artifact run', defaultExpected)
    validateSessionProjection(runResponse.session, runApply, 'artifact run', defaultExpected)
    const invocation = runResponse.result.invocation
    assert(invocation?.workflow_run || invocation?.queued_prompt, 'artifact run should invoke or enqueue a workflow run', invocation)

    const readBack = unwrap(
      await client.send(getWorkflowCodeArtifactRequest(session.id, importedName)),
      'WorkflowCodeArtifact',
    ).artifact
    validateArtifactHistory(readBack, ['imported', 'run'])

    const stateResponse = await client.send(getSessionStateRequest(session.id))
    const state = unwrap(stateResponse, 'SessionStateLoaded')?.session
      ?? unwrap(stateResponse, 'SessionState')?.session
    assert((state?.workflows ?? []).some((workflow) => workflow.id === runApply.workflow_id), 'run workflow should be in loaded session state')
    const existingAgentArtifact = await applyExistingAgentArtifact(client, session, nodePath, workspace)
    let exampleSuite = []
    let exampleSuiteLiveExports = []
    if (options.exampleSuite) {
      const exampleWorkspace = path.join(generatedRoot, 'example-suite', 'workspace')
      const exampleWorktree = path.join(generatedRoot, 'example-suite', 'worktree')
      const exampleSession = unwrap(
        await client.send(createSessionRequest(exampleWorkspace, exampleWorktree, 'workflow-code-example-suite-drill', undefined, null, 'off')),
        'SessionCreated',
      ).session
      try {
        const exampleSuiteResult = await applyExampleSuite(client, exampleSession.id, nodePath, exampleWorkspace, options.timeoutMs)
        exampleSuite = exampleSuiteResult.results
        exampleSuiteLiveExports = exampleSuiteResult.liveExports
      } finally {
        await client.send(endSessionRequest(exampleSession.id)).catch(() => {})
      }
    }
    let realProviderTopology = null
    if (options.realProviderTopology) {
      const realProviderWorkspace = path.join(generatedRoot, 'real-provider-topology', 'workspace')
      const realProviderWorktree = path.join(generatedRoot, 'real-provider-topology', 'worktree')
      const realProviderSession = unwrap(
        await client.send(createSessionRequest(realProviderWorkspace, realProviderWorktree, 'workflow-code-real-provider-topology-drill', undefined, null, 'off')),
        'SessionCreated',
      ).session
      try {
        realProviderTopology = await runRealProviderTopologyDrill(
          client,
          realProviderSession.id,
          nodePath,
          options,
        )
      } finally {
        await client.send(endSessionRequest(realProviderSession.id)).catch(() => {})
      }
    }
    if ((options.secondKernel || options.hetznerSecondKernel) && installedSkill) {
      await client.send(uninstallSkillRequest(workspace, skillName)).catch(() => {})
      installedSkill = false
    }
    const secondKernel = options.secondKernel
      ? await validateSecondKernelDistribution({
        packageExport: exported,
        inlineSource: liveInlineSource,
        directorySource: liveDirectorySource,
        sourceWorkflowId: firstApply.workflow_id,
        skillSourceDir,
        skillName,
        generatedRoot,
        nodePath,
        timeoutMs: options.timeoutMs,
      })
      : null
    const exampleSuiteSecondKernel = options.secondKernel && exampleSuiteLiveExports.length > 0
      ? await validateSecondKernelExampleSuite({
        liveExports: exampleSuiteLiveExports,
        generatedRoot,
        nodePath,
        timeoutMs: options.timeoutMs,
      })
      : null
    const hetznerSecondKernel = options.hetznerSecondKernel
      ? await validateHetznerSecondKernelDistribution({
        packageExport: exported,
        inlineSource: liveInlineSource,
        directorySource: liveDirectorySource,
        sourceWorkflowId: firstApply.workflow_id,
        liveExports: exampleSuiteLiveExports,
        skillSourceDir,
        skillName,
        generatedRoot,
        timeoutMs: options.timeoutMs,
        options,
      })
      : null

    summary = {
      sessionId: session.id,
      attachmentId,
      createdArtifact: artifactName,
      importedArtifact: importedName,
      appliedWorkflowId: firstApply.workflow_id,
      runWorkflowId: runApply.workflow_id,
      runInvocation: invocation.workflow_run ? 'started' : 'enqueued',
      generatedAgents: Object.values(runApply.agent_ids ?? {}),
      schemaRefs: runApply.schema_refs,
      existingAgentArtifact,
      outputSchemaArtifact,
      exampleSuite,
      realProviderTopology,
      secondKernel,
      exampleSuiteSecondKernel,
      hetznerSecondKernel,
    }
    console.log(JSON.stringify(summary, null, 2))

    if (installedSkill) {
      await client.send(uninstallSkillRequest(workspace, skillName)).catch(() => {})
      installedSkill = false
    }
    await client.send(endSessionRequest(session.id)).catch(() => {})
    await client.close()
    passed = true
  } catch (error) {
    failure = error
    console.error(error)
    if (client && sessionId) {
      if (installedSkill) {
        await client.send(uninstallSkillRequest(workspace, skillName)).catch(() => {})
      }
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    if (client) {
      await client.close().catch(() => {})
    }
    process.exitCode = 1
  } finally {
    if (daemonChild) {
      daemonChild.kill('SIGTERM')
      await sleep(1000)
    }
    if (passed || !options.keepArtifactsOnFailure) {
      await rm(generatedRoot, { recursive: true, force: true }).catch(() => {})
    }
    await finalizeDrillArtifacts({
      rootDir: options.artifactRoot,
      passed,
      preserveOnFailure: options.keepArtifactsOnFailure,
      preserveOnSuccess: options.preserveOnSuccess,
      failure,
      metadata: {
        drill: 'workflow-code-artifact',
        kernelUrl,
        workspace,
        worktree,
        sessionId,
        attachmentId,
        summary,
      },
    })
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  await main()
}
