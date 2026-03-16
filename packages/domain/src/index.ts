export type EntityId = string;

export type SessionStatus = 'created' | 'active' | 'parked' | 'ended';

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
  status: 'online' | 'offline';
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
  state: 'starting' | 'running' | 'parked' | 'ended';
}
