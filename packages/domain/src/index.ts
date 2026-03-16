export type EntityId = string;

export const DAEMON_STATUSES = ['online', 'offline'] as const;
export const SESSION_STATUSES = ['created', 'active', 'parked', 'ended'] as const;
export const PROVIDER_RUN_STATES = ['starting', 'running', 'parked', 'ended'] as const;
export const CLIENT_CAPABILITY_LEVELS = [
  'full_terminal',
  'interactive_structured',
  'message_transport',
  'automation_only',
] as const;

export type DaemonStatus = (typeof DAEMON_STATUSES)[number];
export type SessionStatus = (typeof SESSION_STATUSES)[number];
export type ProviderRunState = (typeof PROVIDER_RUN_STATES)[number];
export type ClientCapabilityLevel = (typeof CLIENT_CAPABILITY_LEVELS)[number];

export interface User {
  id: EntityId;
}

export interface Machine {
  id: EntityId;
  userId: EntityId;
  hostname: string;
}

export interface DaemonInstance {
  id: EntityId;
  machineId: EntityId;
  osUser: string;
  version: string;
  status: DaemonStatus;
}

export interface Workspace {
  id: EntityId;
  userId: EntityId;
  name: string;
  rootPath: string;
}

export interface Worktree {
  id: EntityId;
  workspaceId: EntityId;
  gitBranch: string;
  absolutePath: string;
}

export interface Session {
  id: EntityId;
  workspaceId: EntityId;
  worktreeId: EntityId;
  hostMachineId: EntityId;
  hostDaemonId: EntityId;
  status: SessionStatus;
  activeProviderRunId: EntityId | null;
}

export interface ProviderRun {
  id: EntityId;
  sessionId: EntityId;
  provider: string;
  accountProfile: string;
  model: string;
  state: ProviderRunState;
}

export interface SessionAttachment {
  id: EntityId;
  sessionId: EntityId;
  clientId: EntityId;
  capabilityLevel: ClientCapabilityLevel;
  isObserver: boolean;
}

export interface ControllerLease {
  id: EntityId;
  sessionId: EntityId;
  attachmentId: EntityId;
  grantedAt: string;
  expiresAt: string | null;
}

export interface Schedule {
  id: EntityId;
  sessionId: EntityId;
  cron: string;
  enabled: boolean;
}
