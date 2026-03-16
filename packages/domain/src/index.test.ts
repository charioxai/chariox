import assert from 'node:assert/strict';
import test from 'node:test';

import {
  CLIENT_CAPABILITY_LEVELS,
  DAEMON_STATUSES,
  PROVIDER_RUN_STATES,
  SESSION_STATUSES,
  type ControllerLease,
  type DaemonInstance,
  type Machine,
  type ProviderRun,
  type Schedule,
  type Session,
  type SessionAttachment,
  type User,
  type Workspace,
  type Worktree,
} from './index.js';

test('runtime enum constants stay aligned with the M0 domain baseline', () => {
  assert.deepEqual(DAEMON_STATUSES, ['online', 'offline']);
  assert.deepEqual(SESSION_STATUSES, ['created', 'active', 'parked', 'ended']);
  assert.deepEqual(PROVIDER_RUN_STATES, ['starting', 'running', 'parked', 'ended']);
  assert.deepEqual(CLIENT_CAPABILITY_LEVELS, [
    'full_terminal',
    'interactive_structured',
    'message_transport',
    'automation_only',
  ]);
});

test('core entity shapes serialize with the expected identifiers and relations', () => {
  const user: User = { id: 'user_1' };
  const machine: Machine = { id: 'machine_1', userId: user.id, hostname: 'builder.local' };
  const workspace: Workspace = {
    id: 'workspace_1',
    userId: user.id,
    name: 'arroba',
    rootPath: '/workspace/arroba',
  };
  const worktree: Worktree = {
    id: 'worktree_1',
    workspaceId: workspace.id,
    gitBranch: 'main',
    absolutePath: '/workspace/arroba',
  };
  const daemon: DaemonInstance = {
    id: 'daemon_1',
    machineId: machine.id,
    osUser: 'miguel',
    version: '0.1.0',
    status: 'online',
  };
  const session: Session = {
    id: 'session_1',
    workspaceId: workspace.id,
    worktreeId: worktree.id,
    hostMachineId: machine.id,
    hostDaemonId: daemon.id,
    status: 'active',
    activeProviderRunId: 'run_1',
  };
  const providerRun: ProviderRun = {
    id: 'run_1',
    sessionId: session.id,
    provider: 'codex',
    accountProfile: 'default',
    model: 'gpt-5',
    state: 'running',
  };
  const attachment: SessionAttachment = {
    id: 'attachment_1',
    sessionId: session.id,
    clientId: 'client_1',
    capabilityLevel: 'full_terminal',
    isObserver: false,
  };
  const lease: ControllerLease = {
    id: 'lease_1',
    sessionId: session.id,
    attachmentId: attachment.id,
    grantedAt: '2026-03-16T12:00:00.000Z',
    expiresAt: null,
  };
  const schedule: Schedule = {
    id: 'schedule_1',
    sessionId: session.id,
    cron: '0 * * * *',
    enabled: true,
  };

  const snapshot = JSON.parse(
    JSON.stringify({ user, machine, workspace, worktree, daemon, session, providerRun, attachment, lease, schedule }),
  ) as {
    session: Session;
    providerRun: ProviderRun;
    attachment: SessionAttachment;
    lease: ControllerLease;
    schedule: Schedule;
  };

  assert.equal(snapshot.session.activeProviderRunId, snapshot.providerRun.id);
  assert.equal(snapshot.attachment.sessionId, snapshot.session.id);
  assert.equal(snapshot.lease.attachmentId, snapshot.attachment.id);
  assert.equal(snapshot.schedule.sessionId, snapshot.session.id);
});
