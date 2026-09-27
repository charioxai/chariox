import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import {
  PROJECT_ENVIRONMENT_SETUP_DRILL_PIN_SCHEMA,
  PROJECT_ENVIRONMENT_SETUP_DRILL_VALIDATION_COMMANDS,
  assertProjectEnvironmentSetupDrillPreflight,
  assertReadySetupValidation,
  assertStoredDefinition,
  assertLoopbackHomeKernelUrl,
  commandDigest,
  parseProjectEnvironmentSetupDrillArgs,
  projectEnvironmentSetupDrillNativeBuildCommand,
  projectEnvironmentSetupDrillValidationCommands,
  requireVariant,
  runBoundedProjectEnvironmentSetupOperation as runBoundedProjectEnvironmentSetupOperationWithBuilders,
  setupDrillOperationId,
  setupStatusBinding,
  startSetupParameters,
  validateProjectEnvironmentSetupDrillPin,
  validateSetupDrillConfirmations,
} from './project-environment-setup-drill.mjs'

const NOW = Date.parse('2026-09-26T12:00:00.000Z')

const requestBuilders = {
  getProjectEnvironmentSetupStatusRequest: (operationId) => ({ GetProjectEnvironmentSetupStatus: { operationId } }),
  retryProjectEnvironmentSetupRequest: (operationId, sessionId) => ({ RetryProjectEnvironmentSetup: { operationId, sessionId } }),
}

function testStartRequest(targetPin, operationId, branch, validationCommands) {
  return { StartProjectEnvironmentSetup: startSetupParameters(targetPin, operationId, branch, validationCommands) }
}

function runBoundedProjectEnvironmentSetupOperation(options) {
  return runBoundedProjectEnvironmentSetupOperationWithBuilders({ ...options, requestBuilders })
}

function pin(overrides = {}) {
  return {
    schema: PROJECT_ENVIRONMENT_SETUP_DRILL_PIN_SCHEMA,
    reviewed_at: '2026-09-25T12:00:00.000Z',
    reviewed_by: 'kernel-release-owner',
    review_reference: 'MP08-disposable-worker-review',
    disposable_target: true,
    target_setup_approved: true,
    drill_owned_context: true,
    project_id: 'project-drill',
    project_owner_user_id: 'user-drill',
    session_id: 'session-drill',
    workspace_id: '/external/chariox-workspace',
    worktree_id: '/external/chariox-worktree',
    target_machine_id: 'machine-drill',
    target_kernel_id: 'kernel-drill',
    target_platform: 'linux-x86_64',
    utility_agent_id: 'agent-utility',
    utility_provider: 'codex',
    utility_account_profile: 'default',
    utility_provider_run_id: 'worker-run-utility',
    launch_agent_id: 'agent-launch',
    launch_provider: 'codex',
    launch_account_profile: 'default',
    launch_model: 'gpt-5.6-codex',
    launch_effort: 'low',
    ...overrides,
  }
}

function remoteExecution(activeRunId = null) {
  return {
    worker_kernel_id: 'kernel-drill',
    worker_machine_id: 'machine-drill',
    execution_lease_id: 'execution-lease-1',
    leased_agent_id: 'leased-agent-1',
    active_worker_provider_run_id: activeRunId,
  }
}

function preflightSnapshot(overrides = {}) {
  const targetPin = pin()
  return {
    pin: targetPin,
    machines: [{ machine_id: targetPin.target_machine_id, trust_status: 'approved', online: true }],
    kernels: [{
      kernel_id: targetPin.target_kernel_id,
      machine_id: targetPin.target_machine_id,
      accepting_remote_leases: true,
      available_providers: ['codex'],
    }],
    projects: [{
      id: targetPin.project_id,
      owner_user_id: targetPin.project_owner_user_id,
      status: 'active',
      workspace_id: targetPin.workspace_id,
      environment_definition: null,
    }],
    sessions: [{ id: targetPin.session_id, project_id: targetPin.project_id }],
    session: {
      id: targetPin.session_id,
      project_id: targetPin.project_id,
      workspace_id: targetPin.workspace_id,
      worktree_id: targetPin.worktree_id,
      status: 'Active',
      agents: [
        {
          id: targetPin.utility_agent_id,
          provider: targetPin.utility_provider,
          account_profile: targetPin.utility_account_profile,
          worktree_id: targetPin.worktree_id,
          remote_execution: remoteExecution(targetPin.utility_provider_run_id),
          is_processing: false,
        },
        {
          id: targetPin.launch_agent_id,
          provider: targetPin.launch_provider,
          account_profile: targetPin.launch_account_profile,
          worktree_id: targetPin.worktree_id,
          remote_execution: remoteExecution(),
          is_processing: false,
        },
      ],
    },
    ...overrides,
  }
}

function setupStatus(overrides = {}) {
  return {
    operation_id: 'project-setup-live-stored-123e4567-e89b-42d3-a456-426614174000',
    project_id: 'project-drill',
    session_id: 'session-drill',
    agent_id: 'agent-utility',
    worker_id: 'machine-drill',
    platform: 'linux-x86_64',
    phase: 'requested',
    attempt: 1,
    progress_percent: 0,
    definition_digest: null,
    validation: null,
    message: null,
    failure_code: null,
    failure_message: null,
    retryable: false,
    created_at_ms: 1_000,
    updated_at_ms: 1_000,
    ...overrides,
  }
}

function readyStatus(overrides = {}) {
  const status = setupStatus({ phase: 'ready', progress_percent: 100, definition_digest: 'sha256:definition', updated_at_ms: 2_000 })
  status.validation = {
    worker_id: status.worker_id,
    platform: status.platform,
    commands: ['rustc --version'].map((command) => ({
      command_digest: commandDigest(command),
      exit_code: 0,
      stdout_bytes: 10,
      stderr_bytes: 0,
    })),
  }
  return { ...status, ...overrides }
}

test('reviewed target pin and exact destructive confirmations fail closed', () => {
  const targetPin = pin()
  assert.equal(validateProjectEnvironmentSetupDrillPin(targetPin, { nowMs: NOW }), targetPin)
  assert.equal(assertLoopbackHomeKernelUrl('ws://127.0.0.1:43120/kernel'), 'ws://127.0.0.1:43120/kernel')
  assert.throws(() => assertLoopbackHomeKernelUrl('ws://worker.example:43120/kernel'), /numeric loopback/)
  assert.throws(() => validateProjectEnvironmentSetupDrillPin(pin({ disposable_target: false }), { nowMs: NOW }), /disposable/)
  assert.throws(() => validateProjectEnvironmentSetupDrillPin(pin({ target_setup_approved: false }), { nowMs: NOW }), /setup mutations/)
  assert.throws(() => validateProjectEnvironmentSetupDrillPin(pin({ launch_provider: 'dev-stub' }), { nowMs: NOW }), /official provider/)
  assert.throws(() => validateProjectEnvironmentSetupDrillPin(pin({ reviewed_at: '2026-08-01T12:00:00.000Z' }), { nowMs: NOW }), /older than 30 days/)
  const confirmations = {
    '--confirm-pin-sha256': 'sha256:reviewed-pin',
    '--confirm-target': targetPin.target_machine_id,
    '--confirm-delete-session': targetPin.session_id,
    '--confirm-delete-project': targetPin.project_id,
  }
  assert.doesNotThrow(() => validateSetupDrillConfirmations(confirmations, targetPin, 'sha256:reviewed-pin'))
  assert.throws(() => validateSetupDrillConfirmations(confirmations, targetPin, 'sha256:changed-pin'), /pin digest confirmation/)
  assert.throws(() => validateSetupDrillConfirmations({ ...confirmations, '--confirm-target': 'another-machine' }, targetPin, 'sha256:reviewed-pin'), /does not match/)
})

test('shared setup helpers bind the reviewed identity and select the first present response variant', () => {
  const targetPin = pin()
  assert.deepEqual(setupStatusBinding(targetPin, 'operation-drill'), {
    operation_id: 'operation-drill',
    project_id: targetPin.project_id,
    session_id: targetPin.session_id,
    agent_id: targetPin.utility_agent_id,
    worker_id: targetPin.target_machine_id,
    platform: targetPin.target_platform,
  })
  assert.deepEqual(requireVariant({ Missing: null, Found: { value: 1 }, Later: { value: 2 } }, 'Missing', 'Found', 'Later'), { value: 1 })
  assert.throws(() => requireVariant({ Missing: null }, 'Missing'), /none of the expected response variants: Missing/)
})

test('CLI requires the reviewed pin, external paths, and provider launch consent', () => {
  const required = [
    '--home-kernel', 'ws://127.0.0.1:43120/kernel',
    '--target-pin', '/external/target.json',
    '--scratch-dir', '/external/scratch',
    '--evidence-dir', '/external/evidence',
    '--confirm-pin-sha256', 'sha256:reviewed-pin',
    '--confirm-target', 'machine-drill',
    '--confirm-delete-session', 'session-drill',
    '--confirm-delete-project', 'project-drill',
    '--allow-target-setup',
    '--allow-provider-launch',
  ]
  assert.equal(parseProjectEnvironmentSetupDrillArgs(required).allowProviderLaunch, true)
  assert.equal(parseProjectEnvironmentSetupDrillArgs(required).allowTargetSetup, true)
  assert.equal(parseProjectEnvironmentSetupDrillArgs([...required, '--allow-one-retry']).allowOneRetry, true)
  assert.throws(() => parseProjectEnvironmentSetupDrillArgs(required.slice(0, -1)), /allow-provider-launch/)
  assert.throws(() => parseProjectEnvironmentSetupDrillArgs([...required, '--timeout-ms', '999999999']), /unknown option/)
})

test('target preflight binds approved machine, kernel, project, session, leases, and dormant launch agent', () => {
  const result = assertProjectEnvironmentSetupDrillPreflight(preflightSnapshot())
  assert.equal(result.machineId, 'machine-drill')
  assert.equal(result.utilityProviderRunId, 'worker-run-utility')
  assert.equal(result.workerLeases.length, 2)
  assert.throws(() => assertProjectEnvironmentSetupDrillPreflight(preflightSnapshot({
    machines: [{ machine_id: 'machine-drill', trust_status: 'pending', online: true }],
  })), /not approved/)
  assert.throws(() => assertProjectEnvironmentSetupDrillPreflight(preflightSnapshot({
    sessions: [{ id: 'session-drill', project_id: 'project-drill' }, { id: 'other', project_id: 'project-drill' }],
  })), /shared with another session/)
  assert.throws(() => assertProjectEnvironmentSetupDrillPreflight(preflightSnapshot({
    session: {
      ...preflightSnapshot().session,
      agents: preflightSnapshot().session.agents.map((agent) => agent.id === 'agent-launch'
        ? { ...agent, remote_execution: remoteExecution('already-running') }
        : agent),
    },
  })), /already has a provider run before Ready/)
})

test('cold request omits a definition and stored request omits both definition and validation override', () => {
  const targetPin = pin()
  const runId = '123e4567-e89b-42d3-a456-426614174000'
  const coldId = setupDrillOperationId(runId, 'cold')
  const storedId = setupDrillOperationId(runId, 'stored')
  const cold = startSetupParameters(targetPin, coldId, 'cold')
  const stored = startSetupParameters(targetPin, storedId, 'stored')
  assert.equal('definition' in cold, false)
  assert.deepEqual(cold.validationCommands, PROJECT_ENVIRONMENT_SETUP_DRILL_VALIDATION_COMMANDS)
  assert.equal('definition' in stored, false)
  assert.equal('validationCommands' in stored, false)
})

test('native kernel build command is run-id scoped and retains bounded worker limits', () => {
  const runId = '123e4567-e89b-42d3-a456-426614174000'
  const commands = projectEnvironmentSetupDrillValidationCommands(runId)
  const build = commands.at(-1)
  assert.match(build, new RegExp(`chariox-project-environment-setup-${runId}`))
  assert.match(build, /\.chariox-project-setup-owner/)
  assert.match(build, /trap finish_build EXIT/)
  assert.match(build, /CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 cargo build --locked --jobs 2 --package chariox-kernel --bin chariox-kernel/)
  assert.match(build, /rm -rf "\$build_dir"; \[ ! -e "\$build_dir" \]/)
  assert.notMatch(build, /cargo check/)
  assert.ok(build.length < 8_192)
  assert.throws(() => projectEnvironmentSetupDrillValidationCommands('../../other'), /UUID/)
  const syntax = spawnSync('/bin/sh', ['-n', '-c', build], { encoding: 'utf8' })
  assert.equal(syntax.status, 0, syntax.stderr)
})

test('generated build shell runs Cargo and cc fixtures, preserves failures, and removes only owned scratch', () => {
  const runId = '123e4567-e89b-42d3-a456-426614174000'
  const fixtureRoot = mkdtempSync(join(tmpdir(), 'chariox-project-native-build-fixture-'))
  const fakeBin = join(fixtureRoot, 'bin')
  const cargoArgs = join(fixtureRoot, 'cargo-args')
  const cargoEnvironment = join(fixtureRoot, 'cargo-environment')
  const ccArgs = join(fixtureRoot, 'cc-args')
  const ccOutput = join(fixtureRoot, 'cc-output')
  const buildDir = join(fixtureRoot, `chariox-project-environment-setup-${runId}`)
  const collisionRunId = `${runId.slice(0, -1)}1`
  const unownedDir = join(fixtureRoot, `chariox-project-environment-setup-${collisionRunId}`)
  mkdirSync(fakeBin)
  const cargoPath = join(fakeBin, 'cargo')
  const ccPath = join(fakeBin, 'cc')
  writeFileSync(cargoPath, [
    '#!/bin/sh',
    'set -eu',
    'printf \'%s\\n\' "$@" > "$FAKE_CARGO_ARGS"',
    'printf \'%s\\n\' "$CARGO_BUILD_JOBS" "$CARGO_INCREMENTAL" "$CARGO_TARGET_DIR" > "$FAKE_CARGO_ENV"',
    'mkdir -p "$CARGO_TARGET_DIR"',
    'cc -o "$CARGO_TARGET_DIR/linked-native-fixture"',
    'exit "$FAKE_CARGO_EXIT"',
    '',
  ].join('\n'), { mode: 0o700 })
  writeFileSync(ccPath, [
    '#!/bin/sh',
    'set -eu',
    'printf \'%s\\n\' "$@" > "$FAKE_CC_ARGS"',
    '[ "$#" -eq 2 ] && [ "$1" = "-o" ] || exit 51',
    'printf \'linked\\n\' > "$2"',
    'printf \'%s\\n\' "$2" > "$FAKE_CC_OUTPUT"',
    '',
  ].join('\n'), { mode: 0o700 })
  chmodSync(cargoPath, 0o700)
  chmodSync(ccPath, 0o700)

  const execute = (fakeCargoExit) => spawnSync('/bin/sh', ['-c', projectEnvironmentSetupDrillNativeBuildCommand(runId)], {
    encoding: 'utf8',
    timeout: 5_000,
    env: {
      ...process.env,
      PATH: `${fakeBin}:${process.env.PATH ?? '/usr/bin:/bin'}`,
      TMPDIR: fixtureRoot,
      FAKE_CARGO_ARGS: cargoArgs,
      FAKE_CARGO_ENV: cargoEnvironment,
      FAKE_CARGO_EXIT: String(fakeCargoExit),
      FAKE_CC_ARGS: ccArgs,
      FAKE_CC_OUTPUT: ccOutput,
    },
  })

  try {
    const success = execute(0)
    assert.equal(success.error, undefined, success.error?.message)
    assert.equal(success.status, 0, success.stderr)
    assert.deepEqual(readFileSync(cargoArgs, 'utf8').trim().split('\n'), [
      'build', '--locked', '--jobs', '2', '--package', 'chariox-kernel', '--bin', 'chariox-kernel',
    ])
    assert.deepEqual(readFileSync(cargoEnvironment, 'utf8').trim().split('\n'), [
      '2', '0', join(buildDir, 'target'),
    ])
    assert.equal(readFileSync(ccArgs, 'utf8').trim(), `-o\n${join(buildDir, 'target', 'linked-native-fixture')}`)
    assert.equal(readFileSync(ccOutput, 'utf8').trim(), join(buildDir, 'target', 'linked-native-fixture'))
    assert.equal(existsSync(buildDir), false, 'successful shell exit must prove owned target scratch was removed')

    const failedCargo = execute(29)
    assert.equal(failedCargo.error, undefined, failedCargo.error?.message)
    assert.equal(failedCargo.status, 29, 'Cargo failure must survive the EXIT cleanup trap')
    assert.equal(existsSync(buildDir), false, 'failed Cargo command must still remove its owned scratch')

    mkdirSync(unownedDir)
    writeFileSync(join(unownedDir, '.chariox-project-setup-owner'), 'another-run')
    writeFileSync(join(unownedDir, 'keep'), 'preserve')
    rmSync(cargoArgs, { force: true })
    const collision = spawnSync('/bin/sh', ['-c', projectEnvironmentSetupDrillNativeBuildCommand(collisionRunId)], {
      encoding: 'utf8',
      timeout: 5_000,
      env: {
        ...process.env,
        PATH: `${fakeBin}:${process.env.PATH ?? '/usr/bin:/bin'}`,
        TMPDIR: fixtureRoot,
        FAKE_CARGO_ARGS: cargoArgs,
        FAKE_CARGO_ENV: cargoEnvironment,
        FAKE_CARGO_EXIT: '0',
        FAKE_CC_ARGS: ccArgs,
        FAKE_CC_OUTPUT: ccOutput,
      },
    })
    assert.equal(collision.error, undefined, collision.error?.message)
    assert.equal(collision.status, 73, 'an unowned run-id directory must stop before Cargo')
    assert.equal(existsSync(cargoArgs), false, 'Cargo must not run when the run-id directory belongs to another run')
    assert.equal(readFileSync(join(unownedDir, 'keep'), 'utf8'), 'preserve')
  } finally {
    rmSync(fixtureRoot, { recursive: true, force: true })
  }
})

test('preflight accepts a utility-generated definition that retains required bounded checks', () => {
  const definition = {
    origin: 'utility_generated',
    source: 'commands',
    target_platform: 'linux-x86_64',
    validation_commands: [...PROJECT_ENVIRONMENT_SETUP_DRILL_VALIDATION_COMMANDS],
  }
  assert.equal(assertStoredDefinition({ environment_definition: definition }, 'linux-x86_64').origin, 'utility_generated')
  const expanded = { ...definition, validation_commands: [...definition.validation_commands, 'cargo test --workspace --locked --jobs 2'] }
  assert.equal(assertStoredDefinition({ environment_definition: expanded }, 'linux-x86_64').validationCommandCount, definition.validation_commands.length + 1)
  assert.throws(() => assertStoredDefinition({ environment_definition: { ...definition, origin: 'user_authored' } }, 'linux-x86_64'), /did not originate/)
  assert.throws(() => assertStoredDefinition({ environment_definition: { ...definition, target_platform: 'macos-aarch64' } }, 'linux-x86_64'), /platform changed/)
  assert.throws(() => assertStoredDefinition({ environment_definition: { ...definition, validation_commands: ['cargo check --workspace --locked --jobs 2'] } }, 'linux-x86_64'), /omitted the required validation command/)
})

test('Ready requires every requested bounded target validation command to pass on the pinned worker', () => {
  const commands = ['rustc --version']
  const expected = {
    operation_id: setupStatus().operation_id,
    project_id: 'project-drill',
    session_id: 'session-drill',
    agent_id: 'agent-utility',
    worker_id: 'machine-drill',
    platform: 'linux-x86_64',
  }
  assert.equal(assertReadySetupValidation(readyStatus(), { expected, commands }).phase, 'ready')
  assert.throws(() => assertReadySetupValidation(readyStatus({
    validation: {
      worker_id: 'machine-drill',
      platform: 'linux-x86_64',
      commands: [{ command_digest: commandDigest(commands[0]), exit_code: 1, stdout_bytes: 0, stderr_bytes: 15 }],
    },
  }), { expected, commands }), /failed/)
  assert.throws(() => assertReadySetupValidation(readyStatus({ platform: 'another-platform' }), { expected, commands }), /platform changed/)
})

test('Cargo check cannot satisfy the native kernel build Ready gate', () => {
  const requiredCommands = PROJECT_ENVIRONMENT_SETUP_DRILL_VALIDATION_COMMANDS
  const checkOnlyCommands = requiredCommands.map((command) => command.replace(
    'cargo build --locked --jobs 2 --package chariox-kernel --bin chariox-kernel',
    'cargo check --workspace --locked --jobs 2',
  ))
  const status = readyStatus({
    validation: {
      worker_id: 'machine-drill',
      platform: 'linux-x86_64',
      commands: checkOnlyCommands.map((command) => ({
        command_digest: commandDigest(command),
        exit_code: 0,
        stdout_bytes: 10,
        stderr_bytes: 0,
      })),
    },
  })
  assert.throws(() => assertReadySetupValidation(status, {
    expected: {
      operation_id: status.operation_id,
      project_id: 'project-drill',
      session_id: 'session-drill',
      agent_id: 'agent-utility',
      worker_id: 'machine-drill',
      platform: 'linux-x86_64',
    },
    commands: requiredCommands,
  }), /commands changed or were reordered/)
})

test('stored setup replays the same operation idempotently, reconnects, and recovers the same attempt', async () => {
  const targetPin = pin()
  const operationId = setupStatus().operation_id
  const request = testStartRequest(targetPin, operationId, 'stored')
  const requests = []
  let clientNumber = 0
  const clientFactory = async () => {
    const number = clientNumber++
    return {
      async send(frame) {
        requests.push({ client: number, frame })
        if ('StartProjectEnvironmentSetup' in frame) return { ProjectEnvironmentSetupStarted: { status: setupStatus() } }
        if ('GetProjectEnvironmentSetupStatus' in frame) {
          const gets = requests.filter((entry) => entry.client === number && 'GetProjectEnvironmentSetupStatus' in entry.frame).length
          return { ProjectEnvironmentSetupStatus: { status: gets === 1 ? setupStatus({ phase: 'preparing', progress_percent: 20 }) : readyStatus() } }
        }
        throw new Error('unexpected request')
      },
      async close() {},
    }
  }
  const statusSources = []
  const result = await runBoundedProjectEnvironmentSetupOperation({
    createClient: clientFactory,
    pin: targetPin,
    request,
    idempotentStartReplay: true,
    pollMs: 250,
    delay: async () => {},
    onStatus: (_status, source) => statusSources.push(source),
  })
  assert.equal(result.status.phase, 'ready')
  assert.equal(result.status.attempt, 1)
  assert.equal(result.replayedAttempt, 1)
  assert.equal(result.recoveredAttempt, 1)
  assert.equal(clientNumber, 2)
  assert.deepEqual(statusSources, ['start', 'idempotent_start_replay', 'reconnect', 'poll'])
  assert.equal(requests.filter((entry) => 'StartProjectEnvironmentSetup' in entry.frame).length, 2)
  assert.equal(requests.filter((entry) => 'GetProjectEnvironmentSetupStatus' in entry.frame).length, 2)
  assert.equal(requests.every((entry) => entry.frame.GetProjectEnvironmentSetupStatus?.operationId === operationId || 'StartProjectEnvironmentSetup' in entry.frame), true)
})

test('one explicitly permitted service-authorized retry advances exactly one attempt', async () => {
  const targetPin = pin()
  const firstFailed = setupStatus({ phase: 'failed', failure_code: 'transient_worker_failure', retryable: true, updated_at_ms: 1_500 })
  const retryRequested = setupStatus({ attempt: 2, phase: 'requested', progress_percent: 0, retryable: false, updated_at_ms: 2_000 })
  const ready = readyStatus({ attempt: 2, updated_at_ms: 3_000 })
  let gotAfterReconnect = false
  const sent = []
  const factory = async () => ({
    async send(request) {
      sent.push(request)
      if ('StartProjectEnvironmentSetup' in request) return { ProjectEnvironmentSetupStarted: { status: firstFailed } }
      if ('RetryProjectEnvironmentSetup' in request) return { ProjectEnvironmentSetupRetried: { status: retryRequested } }
      if ('GetProjectEnvironmentSetupStatus' in request) {
        if (!gotAfterReconnect) {
          gotAfterReconnect = true
          return { ProjectEnvironmentSetupStatus: { status: firstFailed } }
        }
        return { ProjectEnvironmentSetupStatus: { status: ready } }
      }
      throw new Error('unexpected request')
    },
    async close() {},
  })
  const result = await runBoundedProjectEnvironmentSetupOperation({
    createClient: factory,
    pin: targetPin,
    request: testStartRequest(targetPin, firstFailed.operation_id, 'stored'),
    allowOneRetry: true,
    idempotentStartReplay: false,
    pollMs: 250,
    delay: async () => {},
  })
  assert.equal(result.retries, 1)
  assert.deepEqual(result.attemptHistory, [1, 2])
  assert.equal(result.status.attempt, 2)
  assert.equal(sent.filter((request) => 'RetryProjectEnvironmentSetup' in request).length, 1)
})

test('setup operation rejects a different operation identity on reconnect', async () => {
  let reconnect = false
  const factory = async () => ({
    async send(request) {
      if ('StartProjectEnvironmentSetup' in request) return { ProjectEnvironmentSetupStarted: { status: setupStatus() } }
      if ('GetProjectEnvironmentSetupStatus' in request) {
        reconnect = true
        return { ProjectEnvironmentSetupStatus: { status: setupStatus({ operation_id: 'another-operation' }) } }
      }
      throw new Error('unexpected request')
    },
    async close() {},
  })
  await assert.rejects(runBoundedProjectEnvironmentSetupOperation({
    createClient: factory,
    pin: pin(),
    request: testStartRequest(pin(), setupStatus().operation_id, 'stored'),
    delay: async () => {},
  }), /operation_id changed from the reviewed binding/)
  assert.equal(reconnect, true)
})

test('setup operation rejects attempt drift on reconnect', async () => {
  const factory = async () => ({
    async send(request) {
      if ('StartProjectEnvironmentSetup' in request) return { ProjectEnvironmentSetupStarted: { status: setupStatus() } }
      if ('GetProjectEnvironmentSetupStatus' in request) {
        return { ProjectEnvironmentSetupStatus: { status: setupStatus({ attempt: 2 }) } }
      }
      throw new Error('unexpected request')
    },
    async close() {},
  })
  await assert.rejects(runBoundedProjectEnvironmentSetupOperation({
    createClient: factory,
    pin: pin(),
    request: testStartRequest(pin(), setupStatus().operation_id, 'stored'),
    delay: async () => {},
  }), /attempt changed unexpectedly/)
})

test('setup operation times out at its injected deadline', async () => {
  let now = 0
  const factory = async () => ({
    async send(request) {
      if ('StartProjectEnvironmentSetup' in request) return { ProjectEnvironmentSetupStarted: { status: setupStatus() } }
      if ('GetProjectEnvironmentSetupStatus' in request) {
        return { ProjectEnvironmentSetupStatus: { status: setupStatus({ phase: 'preparing', progress_percent: 20 }) } }
      }
      throw new Error('unexpected request')
    },
    async close() {},
  })
  await assert.rejects(runBoundedProjectEnvironmentSetupOperation({
    createClient: factory,
    pin: pin(),
    request: testStartRequest(pin(), setupStatus().operation_id, 'stored'),
    timeoutMs: 250,
    pollMs: 250,
    nowMs: () => now,
    delay: async (ms) => { now += ms },
  }), (error) => error.code === 'setup_operation_timeout')
})

test('setup operation does not retry a nonretryable failure', async () => {
  const failed = setupStatus({ phase: 'failed', failure_code: 'invalid_definition', retryable: false, updated_at_ms: 1_500 })
  const factory = async () => ({
    async send(request) {
      if ('StartProjectEnvironmentSetup' in request) return { ProjectEnvironmentSetupStarted: { status: failed } }
      if ('GetProjectEnvironmentSetupStatus' in request) return { ProjectEnvironmentSetupStatus: { status: failed } }
      throw new Error('unexpected request')
    },
    async close() {},
  })
  await assert.rejects(runBoundedProjectEnvironmentSetupOperation({
    createClient: factory,
    pin: pin(),
    request: testStartRequest(pin(), failed.operation_id, 'stored'),
    allowOneRetry: true,
    delay: async () => {},
  }), (error) => error.code === 'setup_failed_non_retryable')
})

test('setup operation enforces the one-retry ceiling even when attempt two also fails retryably', async () => {
  const failedFirst = setupStatus({ phase: 'failed', failure_code: 'transient_one', retryable: true, updated_at_ms: 1_500 })
  const retryRequested = setupStatus({ attempt: 2, phase: 'requested', progress_percent: 0, retryable: false, updated_at_ms: 2_000 })
  const failedSecond = setupStatus({ attempt: 2, phase: 'failed', failure_code: 'transient_two', retryable: true, updated_at_ms: 2_500 })
  const retries = []
  let statusReads = 0
  const factory = async () => ({
    async send(request) {
      if ('StartProjectEnvironmentSetup' in request) return { ProjectEnvironmentSetupStarted: { status: failedFirst } }
      if ('RetryProjectEnvironmentSetup' in request) {
        retries.push(request)
        return { ProjectEnvironmentSetupRetried: { status: retryRequested } }
      }
      if ('GetProjectEnvironmentSetupStatus' in request) {
        statusReads += 1
        return { ProjectEnvironmentSetupStatus: { status: statusReads === 1 ? failedFirst : failedSecond } }
      }
      throw new Error('unexpected request')
    },
    async close() {},
  })
  await assert.rejects(runBoundedProjectEnvironmentSetupOperation({
    createClient: factory,
    pin: pin(),
    request: testStartRequest(pin(), failedFirst.operation_id, 'stored'),
    allowOneRetry: true,
    delay: async () => {},
  }), (error) => error.code === 'retryable_setup_failure_without_retry_authorization')
  assert.equal(retries.length, 1)
})
