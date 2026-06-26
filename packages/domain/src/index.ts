export type EntityId = string;

export const DAEMON_STATUSES = ['online', 'offline'] as const;
export const SESSION_STATUSES = ['created', 'active', 'parked', 'ended'] as const;
export const SESSION_EXECUTION_MODES = ['single_agent', 'multi_agent_workflow'] as const;
export const PROVIDER_RUN_STATES = ['starting', 'running', 'parked', 'ended'] as const;
export const PROMPT_STATUSES = ['queued', 'running', 'completed'] as const;
export const SCHEDULER_STATES = ['idle', 'runnable', 'running', 'waiting'] as const;
export const CLIENT_CAPABILITY_LEVELS = [
  'full_terminal',
  'interactive_structured',
  'message_transport',
  'automation_only',
] as const;
export const WORKFLOW_TOPOLOGIES = ['circular', 'hierarchical'] as const;
export const WORKFLOW_RUN_STATUSES = [
  'created',
  'running',
  'waiting',
  'completed',
  'failed',
  'stopped',
] as const;
export const NODE_RUN_STATUSES = [
  'created',
  'ready',
  'running',
  'waiting',
  'completed',
  'failed',
  'stopped',
] as const;
export const AGGREGATION_STATUSES = ['idle', 'waiting', 'ready', 'completed', 'failed'] as const;
export const NODE_MESSAGE_DELIVERY_STATUSES = ['pending', 'delivered', 'consumed', 'failed'] as const;
export const WORKTREE_ISOLATION_MODES = ['shared_session', 'isolated_branch', 'isolated_worktree'] as const;
export const WORKFLOW_EDGE_TYPES = ['handoff', 'loopback', 'fanout', 'fanin'] as const;
export const STOP_RECOMMENDATIONS = ['continue', 'stop', 'complete'] as const;
export const AGENT_STATES = ['idle', 'working', 'focused', 'error'] as const;

export type DaemonStatus = (typeof DAEMON_STATUSES)[number];
export type SessionStatus = (typeof SESSION_STATUSES)[number];
export type SessionExecutionMode = (typeof SESSION_EXECUTION_MODES)[number];
export type ProviderRunState = (typeof PROVIDER_RUN_STATES)[number];
export type PromptStatus = (typeof PROMPT_STATUSES)[number];
export type SchedulerState = (typeof SCHEDULER_STATES)[number];
export type ClientCapabilityLevel = (typeof CLIENT_CAPABILITY_LEVELS)[number];
export type WorkflowTopology = (typeof WORKFLOW_TOPOLOGIES)[number];
export type WorkflowRunStatus = (typeof WORKFLOW_RUN_STATUSES)[number];
export type NodeRunStatus = (typeof NODE_RUN_STATUSES)[number];
export type AggregationStatus = (typeof AGGREGATION_STATUSES)[number];
export type NodeMessageDeliveryStatus = (typeof NODE_MESSAGE_DELIVERY_STATUSES)[number];
export type WorktreeIsolationMode = (typeof WORKTREE_ISOLATION_MODES)[number];
export type WorkflowEdgeType = (typeof WORKFLOW_EDGE_TYPES)[number];
export type StopRecommendation = (typeof STOP_RECOMMENDATIONS)[number];
export type AgentState = (typeof AGENT_STATES)[number];

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

export interface AgentInstance {
  id: EntityId;
  agentRef: string;
  sessionId: EntityId;
  role?: "standard" | "meta" | string;
  metaMode?: {
    taskId?: string | null;
    activatedAtMs: number;
    baselineExecutionModeOverride?: "build" | "plan" | null;
    baselinePermissionLevelOverride?: "required" | "yolo" | null;
  } | null;
  alias: string | null;
  provider: string;
  model: string | null;
  worktreeId: EntityId | null;
  state: AgentState;
  isProcessing: boolean;
  gridRow: number;
  gridCol: number;
  gridRowSpan: number;
  gridColSpan: number;
  createdAt: string;
  lastActivityAt: string;
}

export interface Session {
  id: EntityId;
  workspaceId: EntityId;
  worktreeId: EntityId;
  hostMachineId: EntityId;
  hostDaemonId: EntityId;
  executionMode: SessionExecutionMode;
  status: SessionStatus;
  activeWorkflowRunId: EntityId | null;
  activePromptId: EntityId | null;
  focusedAgentId: EntityId | null;
  configVersion: number;
  schedulerState: SchedulerState;
  maxAgents: number;
}

export interface ProviderRun {
  id: EntityId;
  sessionId: EntityId;
  agentInstanceId: EntityId | null;
  nodeRunId: EntityId | null;
  worktreeAssignmentId: EntityId | null;
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
}

export interface PromptQueueItem {
  id: EntityId;
  sessionId: EntityId;
  sourceAttachmentId: EntityId;
  targetAgentId: EntityId;
  prompt: string;
  status: PromptStatus;
}

export interface SessionConfigState {
  sessionId: EntityId;
  version: number;
  values: Record<string, string>;
  updatedByAttachmentId: EntityId | null;
}

export interface Schedule {
  id: EntityId;
  sessionId: EntityId;
  cron: string;
  enabled: boolean;
}

export interface WorkflowDefinition {
  id: EntityId;
  sessionId: EntityId;
  topologyType: WorkflowTopology;
  coordinatorNodeId: EntityId;
  version: number;
  validationPolicy: string;
}

export interface WorkflowNode {
  id: EntityId;
  workflowDefinitionId: EntityId;
  nodeKey: string;
  label: string;
  role: string;
  provider: string | null;
  accountProfile: string | null;
  model: string | null;
  concurrencyGroup: string | null;
  writesCode: boolean;
}

export interface WorkflowEdge {
  id: EntityId;
  workflowDefinitionId: EntityId;
  fromNodeId: EntityId;
  toNodeId: EntityId;
  edgeType: WorkflowEdgeType;
  routingPolicy: string;
}

export interface WorkflowRun {
  id: EntityId;
  sessionId: EntityId;
  workflowDefinitionId: EntityId;
  coordinatorNodeId: EntityId;
  status: WorkflowRunStatus;
  iterationCount: number;
  maxIterations: number | null;
  startedAt: string;
  completedAt: string | null;
}

export interface NodeRun {
  id: EntityId;
  workflowRunId: EntityId;
  workflowNodeId: EntityId;
  providerRunId: EntityId | null;
  worktreeAssignmentId: EntityId | null;
  status: NodeRunStatus;
  summary: string | null;
  artifacts: string[];
  handoffPayload: string | null;
  stopRecommendation: StopRecommendation | null;
  startedAt: string;
  completedAt: string | null;
}

export interface NodeMessage {
  id: EntityId;
  workflowRunId: EntityId;
  sourceNodeRunId: EntityId;
  targetNodeId: EntityId | null;
  targetNodeRunId: EntityId | null;
  messageType: string;
  summary: string;
  artifacts: string[];
  handoffPayload: string;
  deliveryStatus: NodeMessageDeliveryStatus;
  createdAt: string;
}

export interface WorktreeAssignment {
  id: EntityId;
  sessionId: EntityId;
  workflowRunId: EntityId | null;
  workflowNodeId: EntityId | null;
  nodeRunId: EntityId | null;
  worktreeId: EntityId;
  branch: string;
  isolationMode: WorktreeIsolationMode;
}

export interface AggregationState {
  id: EntityId;
  workflowRunId: EntityId;
  workflowNodeId: EntityId;
  barrierKey: string;
  expectedInputs: number;
  receivedInputs: number;
  aggregationPolicy: string;
  status: AggregationStatus;
}
