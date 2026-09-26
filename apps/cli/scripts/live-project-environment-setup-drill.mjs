#!/usr/bin/env node
import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { lstat, mkdir, readFile, readdir, realpath, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import {
  PROJECT_ENVIRONMENT_SETUP_DRILL_CONTROL_TIMEOUT_MS,
  PROJECT_ENVIRONMENT_SETUP_DRILL_POLL_MS,
  assertProjectEnvironmentSetupDrillPreflight,
  assertReadySetupValidation,
  assertStoredDefinition,
  assertLoopbackHomeKernelUrl,
  commandDigest,
  getSetupStatusFromResponse,
  parseProjectEnvironmentSetupDrillArgs,
  projectEnvironmentSetupDrillValidationCommands,
  providerCapability,
  runBoundedProjectEnvironmentSetupOperation,
  safeSetupStatusEvidence,
  setupDrillOperationId,
  startSetupRequest,
  validateProjectEnvironmentSetupDrillPin,
  validateSetupDrillConfirmations,
} from './lib/project-environment-setup-drill.mjs'
import { cleanupProjectEnvironmentSetupDrillContext } from './lib/project-environment-setup-drill-cleanup.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const PROVIDER_RUN_TIMEOUT_MS = 90_000
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

const { LocalIpcClient } = await import(pathToFileURL(path.join(repoRoot, 'packages/kernel-client/dist/ipc.js')).href)
const requests = await import(pathToFileURL(path.join(repoRoot, 'packages/kernel-client/dist/ipc-requests.js')).href)

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-project-environment-setup-drill.mjs --home-kernel ws://127.0.0.1:PORT/kernel --target-pin /external/reviewed-target.json',
    '  --scratch-dir /external/scratch --evidence-dir /external/evidence',
    '  --confirm-pin-sha256 sha256:PIN_FILE_DIGEST --confirm-target MACHINE_ID',
    '  --confirm-delete-session SESSION_ID',
    '  --confirm-delete-project PROJECT_ID --allow-target-setup --allow-provider-launch [--allow-one-retry]',
    '',
    'Runs only against an existing approved remote worker and a reviewed pin file.',
    'The pin must mark the target disposable and its Project/session/agents drill-owned.',
    'Both exact delete confirmations are required because cleanup destroys those pinned agents, session, and Project.',
    'The worker machine, enrollment, and materialized worktree are retained for root-owned post-campaign disposition.',
    'The setup operations use the home kernel API; no worker endpoint is accepted.',
    'The official launch agent is launched only after both setup operations report Ready.',
    'Pin fields: schema, reviewed_at, reviewed_by, review_reference, disposable_target, target_setup_approved, drill_owned_context, project_id, project_owner_user_id, session_id, workspace_id, worktree_id, target_machine_id, target_kernel_id, target_platform, utility_agent_id, utility_provider, utility_account_profile, utility_provider_run_id, launch_agent_id, launch_provider, launch_account_profile, launch_model, launch_effort.',
    'This script does not provision, reimage, or delete the worker machine.',
    '',
    `Each setup operation is limited to 15 minutes; target validation has kernel-enforced 120 second command and 300 second aggregate bounds.`,
    `Rust workspace Cargo check uses at most 2 build jobs and a unique target directory removed by a worker shell trap.`,
  ].join('\n'))
}

function requireVariant(response, ...variants) {
  for (const variant of variants) {
    if (response?.[variant] != null) return response[variant]
  }
  throw new Error(`home kernel returned none of the expected response variants: ${variants.join(', ')}`)
}

async function sendWithTimeout(client, request, label, timeoutMs = PROJECT_ENVIRONMENT_SETUP_DRILL_CONTROL_TIMEOUT_MS) {
  let timer
  try {
    return await Promise.race([
      client.send(request),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out`)), timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function withClient(createClient, callback) {
  const client = await createClient()
  try {
    return await callback(client)
  } finally {
    await client.close().catch(() => {})
  }
}

function newHomeClient(endpoint) {
  return new LocalIpcClient(endpoint, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
}

async function loadExternalInputs(options) {
  const homeKernel = assertLoopbackHomeKernelUrl(options['--home-kernel'])
  const pinPath = await externalFilePath(options['--target-pin'], 'target pin')
  const scratchDir = await externalDirectoryPath(options['--scratch-dir'], 'scratch directory')
  const evidenceDir = await externalDirectoryPath(options['--evidence-dir'], 'evidence directory')
  assertDisjointRoots(scratchDir, evidenceDir)
  const pinBytes = await readFile(pinPath)
  const pinStat = await lstat(pinPath)
  assert.ok(pinStat.isFile() && !pinStat.isSymbolicLink(), 'target pin must be a regular file, not a symlink')
  const pinSha256 = sha256(pinBytes)
  const pin = validateProjectEnvironmentSetupDrillPin(JSON.parse(pinBytes.toString('utf8')))
  validateSetupDrillConfirmations(options, pin, pinSha256)
  return {
    homeKernel,
    pin,
    pinPath,
    pinSha256,
    scratchDir,
    evidenceDir,
  }
}

async function externalFilePath(value, label) {
  assert.ok(path.isAbsolute(value), `${label} path must be absolute`)
  const originalStat = await lstat(value)
  assert.ok(originalStat.isFile() && !originalStat.isSymbolicLink(), `${label} must be a regular file, not a symlink`)
  const resolved = await realpath(value)
  assertOutsideRepository(resolved, label)
  return resolved
}

async function externalDirectoryPath(value, label) {
  assert.ok(path.isAbsolute(value), `${label} path must be absolute`)
  const resolved = await realpath(value)
  const stat = await lstat(resolved)
  assert.ok(stat.isDirectory(), `${label} must be an existing directory`)
  assertOutsideRepository(resolved, label)
  return resolved
}

function assertOutsideRepository(candidate, label) {
  const relative = path.relative(repoRoot, candidate)
  assert.ok(relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative), `${label} must be outside the repository`)
}

function assertDisjointRoots(left, right) {
  const contains = (parent, child) => {
    const relative = path.relative(parent, child)
    return relative === '' || (!relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative))
  }
  assert.ok(!contains(left, right) && !contains(right, left), 'scratch and evidence directories must be separate non-nested roots')
}

async function createOwnedScratch(scratchDir, runId) {
  const runDir = path.join(scratchDir, `project-environment-setup-${runId}`)
  await mkdir(runDir, { mode: 0o700 })
  const ownerPath = path.join(runDir, '.chariox-project-environment-setup-drill-owner')
  await writeFile(ownerPath, `${runId}\n`, { encoding: 'utf8', flag: 'wx', mode: 0o600 })
  return { runDir, ownerPath, runId }
}

async function cleanupOwnedScratch(ownership) {
  if (!ownership) return false
  try {
    const runDir = await realpath(ownership.runDir)
    const expectedParent = path.dirname(ownership.runDir)
    if (runDir !== ownership.runDir || path.dirname(runDir) !== expectedParent) return false
    const ownerPath = path.join(runDir, '.chariox-project-environment-setup-drill-owner')
    const ownerStat = await lstat(ownerPath)
    if (!ownerStat.isFile() || ownerStat.isSymbolicLink()) return false
    if ((await readFile(ownerPath, 'utf8')) !== `${ownership.runId}\n`) return false
    const entries = await readdir(runDir)
    if (entries.length !== 1 || entries[0] !== path.basename(ownerPath)) return false
    await rm(runDir, { recursive: true })
    return true
  } catch {
    return false
  }
}

async function preflight(createClient, pin) {
  return await withClient(createClient, async (client) => {
    const machines = requireVariant(
      await sendWithTimeout(client, requests.listRemoteMachinesRequest(), 'list enrolled remote machines'),
      'RemoteMachinesListed',
    ).machines ?? []
    const kernels = requireVariant(
      await sendWithTimeout(client, requests.listRemoteMachineKernelsRequest(pin.target_machine_id), 'list target worker kernels'),
      'RemoteMachineKernelsListed',
    ).kernels ?? []
    const projects = requireVariant(
      await sendWithTimeout(client, requests.listProjectsRequest(false), 'list Projects'),
      'ProjectsListed',
    ).projects ?? []
    const sessions = requireVariant(
      await sendWithTimeout(client, requests.listSessionsRequest(), 'list sessions'),
      'SessionsListed',
    ).sessions ?? []
    const state = requireVariant(
      await sendWithTimeout(client, requests.getSessionStateRequest(pin.session_id), 'get pinned session state'),
      'SessionState', 'SessionStateLoaded',
    )
    const summary = assertProjectEnvironmentSetupDrillPreflight({
      pin,
      machines,
      kernels,
      projects,
      sessions,
      session: state.session,
    })
    return { summary, project: projects.find((entry) => entry.id === pin.project_id) }
  })
}

function makeStatusRecorder(operationEvidence) {
  const seen = new Set()
  return (status, source) => {
    const record = safeSetupStatusEvidence(status, source)
    const key = JSON.stringify(record)
    if (seen.has(key)) return
    const important = ['ready', 'failed', 'cancelled'].includes(status.phase) || ['idempotent_start_replay', 'reconnect', 'retry'].includes(source)
    if (operationEvidence.statuses.length >= 128 && !important) return
    seen.add(key)
    operationEvidence.statuses.push(record)
  }
}

async function readProject(createClient, projectId) {
  return await withClient(createClient, async (client) => {
    const projects = requireVariant(
      await sendWithTimeout(client, requests.listProjectsRequest(false), 'read Project definition'),
      'ProjectsListed',
    ).projects ?? []
    return projects.find((project) => project.id === projectId) ?? null
  })
}

async function assertProjectSetupReadyAgain(createClient, operationId, expected, commands) {
  return await withClient(createClient, async (client) => {
    const response = await sendWithTimeout(client, { GetProjectEnvironmentSetupStatus: { operationId } }, 'recheck Ready setup status')
    const status = getSetupStatusFromResponse(response)
    assertReadySetupValidation(status, { expected, commands })
    return status
  })
}

async function launchOfficialProvider(createClient, pin, readyStatuses, evidence) {
  for (const status of readyStatuses) {
    assert.equal(status.phase, 'ready', 'official provider launch was attempted before every setup branch was Ready')
    assert.ok(status.validation, 'official provider launch was attempted without target validation')
  }
  const launchRequestedAtMs = Date.now()
  evidence.readyOperations = readyStatuses.map((status) => ({
    operationId: status.operation_id,
    attempt: status.attempt,
    phase: status.phase,
    definitionDigest: status.definition_digest,
    updatedAtMs: status.updated_at_ms,
  }))
  const launch = await withClient(createClient, async (client) => {
    const state = requireVariant(
      await sendWithTimeout(client, requests.getSessionStateRequest(pin.session_id), 'verify idle launch agent'),
      'SessionState', 'SessionStateLoaded',
    ).session
    const agent = (state.agents ?? []).find((candidate) => candidate.id === pin.launch_agent_id)
    assert.ok(agent, 'pinned launch agent disappeared before provider launch')
    assert.equal(agent.remote_execution?.worker_machine_id, pin.target_machine_id, 'launch agent moved to a different worker before provider launch')
    assert.equal(agent.remote_execution?.worker_kernel_id, pin.target_kernel_id, 'launch agent moved to a different worker kernel before provider launch')
    assert.equal(agent.remote_execution?.active_worker_provider_run_id ?? null, null, 'launch agent already has a provider run before Ready-gated launch')

    const response = await sendWithTimeout(client, requests.launchProviderRunRequest(
      pin.session_id,
      pin.launch_provider,
      pin.launch_account_profile,
      pin.launch_model,
      pin.launch_effort,
      pin.launch_agent_id,
    ), 'launch official provider after Ready')
    const providerRun = requireVariant(response, 'ProviderRunLaunched', 'ProviderRunLaunchAccepted').provider_run
    assert.ok(providerRun?.id, 'home kernel accepted provider launch without a run identity')
    assert.equal(providerRun.session_id, pin.session_id, 'provider run launched in a different session')
    assert.equal(providerRun.agent_instance_id, pin.launch_agent_id, 'provider run launched for a different agent')
    assert.equal(providerCapability(providerRun.provider), providerCapability(pin.launch_provider), 'provider run changed provider')
    return providerRun
  })

  evidence.requestedAtMs = launchRequestedAtMs
  evidence.afterReady = true
  evidence.provider = pin.launch_provider
  evidence.accountProfile = pin.launch_account_profile
  evidence.model = pin.launch_model
  evidence.effort = pin.launch_effort
  evidence.agentId = pin.launch_agent_id
  evidence.providerRunId = launch.id

  let lastRun = null
  const deadline = Date.now() + PROVIDER_RUN_TIMEOUT_MS
  await withClient(createClient, async (client) => {
    while (Date.now() < deadline) {
      const response = await sendWithTimeout(client, requests.getProviderRunRequest(launch.id), 'read official provider run')
      lastRun = requireVariant(response, 'ProviderRun').provider_run
      if (lastRun?.state === 'Running') return
      if (['Ended', 'Failed', 'Cancelled'].includes(lastRun?.state)) {
        throw new Error('official provider run ended before reaching Running')
      }
      await sleep(500)
    }
    throw new Error('official provider run did not reach Running within 90 seconds')
  })
  assert.equal(lastRun?.id, launch.id, 'provider run identity changed while waiting for Running')
  evidence.state = lastRun.state
  evidence.startedAtMs = lastRun.started_at_ms ?? null
}

async function main() {
  const parsed = parseProjectEnvironmentSetupDrillArgs(process.argv.slice(2))
  if (parsed.help) {
    printHelp()
    return
  }

  const input = await loadExternalInputs(parsed)
  const runId = randomUUID()
  const validationCommands = projectEnvironmentSetupDrillValidationCommands(runId)
  const ownedScratch = await createOwnedScratch(input.scratchDir, runId)
  const evidencePath = path.join(input.evidenceDir, `project-environment-setup-live-${runId}.json`)
  const createClient = async () => newHomeClient(input.homeKernel)
  const evidence = {
    schema: 'chariox.project-environment-setup-live-drill.evidence.v1',
    runId,
    startedAt: new Date().toISOString(),
    targetPinSha256: input.pinSha256,
    homeTransport: 'loopback-websocket-to-home-kernel-only',
    target: {
      machineId: input.pin.target_machine_id,
      kernelId: input.pin.target_kernel_id,
      platform: input.pin.target_platform,
    },
    executionBounds: {
      setupOperationDeadlineMs: 15 * 60 * 1_000,
      maximumRetriesPerOperation: 1,
      statusPollIntervalMs: PROJECT_ENVIRONMENT_SETUP_DRILL_POLL_MS,
      kernelCommandDeadlineMs: 120_000,
      kernelAggregateValidationDeadlineMs: 300_000,
      cargoBuildJobs: 2,
      cargoIncremental: false,
      targetDirectory: 'drill-run-id-bound-worker-temp-directory-with-owner-marker-and-exit-trap',
      nativeDependencyBuild: 'cargo check --workspace builds bundled libsqlite3-sys SQLite C code',
      validationCommandDigests: validationCommands.map(commandDigest),
      targetResourceTelemetry: 'unavailable-through-public-setup-status-api',
    },
    context: {
      projectId: input.pin.project_id,
      sessionId: input.pin.session_id,
      workspaceId: input.pin.workspace_id,
      worktreeId: input.pin.worktree_id,
      utilityAgentId: input.pin.utility_agent_id,
      launchAgentId: input.pin.launch_agent_id,
    },
    targetSetupApproved: input.pin.target_setup_approved,
    setupOperations: [],
    providerLaunch: { attempted: false, afterReady: false },
    cleanup: {
      scope: 'drill-owned utility/launch agents, session, Project, and local scratch only',
      targetWorkerDisposition: 'retained for root-owned post-campaign disposal and review',
      complete: false,
      operationsSettled: false,
      blockedDeletion: false,
      destroyedAgentIds: [],
      deletedSessionId: null,
      deletedProjectId: null,
      errors: [],
    },
    limitations: [
      'Target CPU, memory, and disk telemetry are not exposed by the setup status API; the drill enforces kernel command deadlines and Cargo build-job limits but does not claim resource-metric proof.',
      'Retry is observed only if the home kernel returns a retryable setup failure; the drill does not inject a failure. --allow-one-retry permits at most one service-authorized retry.',
      'A kernel hard timeout can SIGKILL the worker shell before its EXIT trap runs; the run-id owner marker allows exact cleanup on a retry, but a terminal timeout with no retry can leave that uniquely marked temp directory because the public home API has no target-filesystem cleanup request.',
      'The live worker/provider/build gate remains open until root runs this campaign against its reviewed disposable target and reviews the evidence.',
      'Worker machine enrollment and materialized target worktree are not deleted or certified clean by the home-kernel setup API; root owns their post-campaign disposition.',
    ],
  }

  const operationIds = []
  let mutationMayHaveStarted = false
  let passed = false
  let failureCode = null
  try {
    const checked = await preflight(createClient, input.pin)
    evidence.preflight = checked.summary

    const coldOperationId = setupDrillOperationId(runId, 'cold')
    const coldEvidence = {
      branch: 'cold_utility_generated',
      operationId: coldOperationId,
      request: { definitionSupplied: false, validationCommandsSupplied: true },
      attempts: [],
      statuses: [],
      retries: 0,
    }
    evidence.setupOperations.push(coldEvidence)
    operationIds.push(coldOperationId)
    mutationMayHaveStarted = true
    const coldResult = await runBoundedProjectEnvironmentSetupOperation({
      createClient,
      pin: input.pin,
      request: startSetupRequest(input.pin, coldOperationId, 'cold', validationCommands),
      allowOneRetry: parsed.allowOneRetry,
      onStatus: makeStatusRecorder(coldEvidence),
    })
    coldEvidence.retries = coldResult.retries
    coldEvidence.attempts = coldResult.attemptHistory
    coldEvidence.idempotentStartReplayAttempt = coldResult.replayedAttempt
    coldEvidence.reconnectRecoveredAttempt = coldResult.recoveredAttempt
    const generatedProject = await readProject(createClient, input.pin.project_id)
    const generatedDefinition = assertStoredDefinition(generatedProject, input.pin.target_platform, validationCommands)
    const coldValidationCommands = generatedProject.environment_definition.validation_commands
    const coldExpected = setupBinding(input.pin, coldOperationId)
    const coldReady = assertReadySetupValidation(coldResult.status, {
      expected: coldExpected,
      commands: coldValidationCommands,
    })
    coldEvidence.readyObservedAtMs = Date.now()
    coldEvidence.readyDefinitionDigest = coldReady.definition_digest
    coldEvidence.persistedDefinition = generatedDefinition
    evidence.coldUtilityGeneratedDefinitionProven = true

    const storedOperationId = setupDrillOperationId(runId, 'stored')
    const storedEvidence = {
      branch: 'stored_repeatable_definition',
      operationId: storedOperationId,
      request: { definitionSupplied: false, validationCommandsSupplied: false },
      attempts: [],
      statuses: [],
      retries: 0,
    }
    evidence.setupOperations.push(storedEvidence)
    operationIds.push(storedOperationId)
    const storedProject = await readProject(createClient, input.pin.project_id)
    const storedBeforeStart = assertStoredDefinition(storedProject, input.pin.target_platform, validationCommands)
    const storedValidationCommands = storedProject.environment_definition.validation_commands
    assert.deepEqual(storedBeforeStart.validationCommandDigests, generatedDefinition.validationCommandDigests, 'stored definition command identity changed after cold setup')
    storedEvidence.persistedDefinition = storedBeforeStart
    mutationMayHaveStarted = true
    const storedResult = await runBoundedProjectEnvironmentSetupOperation({
      createClient,
      pin: input.pin,
      request: startSetupRequest(input.pin, storedOperationId, 'stored'),
      idempotentStartReplay: true,
      allowOneRetry: parsed.allowOneRetry,
      onStatus: makeStatusRecorder(storedEvidence),
    })
    storedEvidence.retries = storedResult.retries
    storedEvidence.attempts = storedResult.attemptHistory
    storedEvidence.idempotentStartReplayAttempt = storedResult.replayedAttempt
    storedEvidence.reconnectRecoveredAttempt = storedResult.recoveredAttempt
    const storedReady = assertReadySetupValidation(storedResult.status, {
      expected: setupBinding(input.pin, storedOperationId),
      commands: storedValidationCommands,
    })
    storedEvidence.readyObservedAtMs = Date.now()
    storedEvidence.readyDefinitionDigest = storedReady.definition_digest
    assert.equal(storedReady.definition_digest, coldReady.definition_digest, 'stored-definition run did not reuse the utility-generated definition digest')
    evidence.storedDefinitionReuseProven = true

    const currentColdReady = await assertProjectSetupReadyAgain(createClient, coldOperationId, coldExpected, coldValidationCommands)
    const currentStoredReady = await assertProjectSetupReadyAgain(createClient, storedOperationId, setupBinding(input.pin, storedOperationId), storedValidationCommands)
    const readyStatuses = [currentColdReady, currentStoredReady]
    evidence.providerLaunch.attempted = true
    await launchOfficialProvider(createClient, input.pin, readyStatuses, evidence.providerLaunch)
    evidence.readyBeforeOfficialProviderLaunch = true
    passed = true
  } catch (error) {
    failureCode = typeof error?.code === 'string' ? error.code : 'live_setup_drill_failed'
    evidence.failureCode = failureCode
  } finally {
    if (mutationMayHaveStarted) {
      try {
        await cleanupProjectEnvironmentSetupDrillContext({
          createClient,
          pin: input.pin,
          operationIds,
          expectedOwnership: evidence.preflight,
          evidence: evidence.cleanup,
        })
      } catch {
        evidence.cleanup.errors.push('cleanup_transport_or_kernel_failure')
        evidence.cleanup.blockedDeletion = true
      }
    } else {
      evidence.cleanup.complete = false
      evidence.cleanup.skipped = 'no setup mutation was attempted'
    }
    evidence.cleanup.scratchRemoved = await cleanupOwnedScratch(ownedScratch)
    evidence.cleanup.complete = evidence.cleanup.complete === true && evidence.cleanup.scratchRemoved
    evidence.completedAt = new Date().toISOString()
    evidence.outcome = passed && evidence.cleanup.complete ? 'completed_scoped_drill' : 'incomplete_or_failed'
    try {
      await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, { encoding: 'utf8', flag: 'wx', mode: 0o600 })
    } catch {
      failureCode ??= 'evidence_write_failed'
      evidence.outcome = 'incomplete_or_failed'
    }
  }

  if (!passed || !evidence.cleanup.complete || failureCode) {
    console.error(`project environment setup live drill ${evidence.outcome}; failure=${failureCode ?? 'cleanup_incomplete'}; evidence=${evidencePath}`)
    process.exitCode = 1
    return
  }
  console.log(JSON.stringify({ outcome: evidence.outcome, evidencePath, target: evidence.target, setupOperations: evidence.setupOperations.map(({ operationId, attempts, retries, readyDefinitionDigest }) => ({ operationId, attempts, retries, readyDefinitionDigest })), providerRunId: evidence.providerLaunch.providerRunId, cleanup: evidence.cleanup }, null, 2))
}

function setupBinding(pin, operationId) {
  return {
    operation_id: operationId,
    project_id: pin.project_id,
    session_id: pin.session_id,
    agent_id: pin.utility_agent_id,
    worker_id: pin.target_machine_id,
    platform: pin.target_platform,
  }
}

function sha256(bytes) {
  return `sha256:${createHash('sha256').update(bytes).digest('hex')}`
}

await main()
