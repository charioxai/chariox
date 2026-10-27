import assert from 'node:assert/strict'
import test from 'node:test'

import {
  AGGREGATION_STATUSES,
  CLIENT_CAPABILITY_LEVELS,
  DAEMON_STATUSES,
  NODE_MESSAGE_DELIVERY_STATUSES,
  NODE_RUN_STATUSES,
  PROMPT_STATUSES,
  PROVIDER_RUN_STATES,
  SCHEDULER_STATES,
  SESSION_EXECUTION_MODES,
  SESSION_STATUSES,
  STOP_RECOMMENDATIONS,
  WORKFLOW_EDGE_TYPES,
  WORKFLOW_RUN_STATUSES,
  WORKFLOW_TOPOLOGIES,
  WORKTREE_ISOLATION_MODES,
  type AggregationState,
  type DaemonInstance,
  type Machine,
  type NodeMessage,
  type NodeRun,
  type PromptQueueItem,
  type ProviderRun,
  type Schedule,
  type Session,
  type SessionAttachment,
  type SessionConfigState,
  type User,
  type WorkflowDefinition,
  type WorkflowEdge,
  type WorkflowNode,
  type WorkflowRun,
  type WorktreeAssignment,
  type Workspace,
  type Worktree,
} from './index.js'

test('runtime enum constants stay aligned with the workflow-oriented domain model', () => {
  assert.deepEqual(DAEMON_STATUSES, ['online', 'offline'])
  assert.deepEqual(SESSION_STATUSES, ['created', 'active', 'parked', 'ended'])
  assert.deepEqual(SESSION_EXECUTION_MODES, ['single_agent', 'multi_agent_workflow'])
  assert.deepEqual(PROVIDER_RUN_STATES, ['starting', 'running', 'parked', 'ended'])
  assert.deepEqual(PROMPT_STATUSES, ['queued', 'running', 'completed'])
  assert.deepEqual(SCHEDULER_STATES, ['idle', 'runnable', 'running', 'waiting'])
  assert.deepEqual(CLIENT_CAPABILITY_LEVELS, [
    'full_terminal',
    'interactive_structured',
    'message_transport',
    'automation_only',
  ])
  assert.deepEqual(WORKFLOW_TOPOLOGIES, ['circular', 'hierarchical'])
  assert.deepEqual(WORKFLOW_RUN_STATUSES, ['created', 'running', 'waiting', 'paused', 'completed', 'failed', 'stopped'])
  assert.deepEqual(NODE_RUN_STATUSES, ['created', 'ready', 'running', 'waiting', 'completed', 'failed', 'stopped'])
  assert.deepEqual(AGGREGATION_STATUSES, ['idle', 'waiting', 'ready', 'completed', 'failed'])
  assert.deepEqual(NODE_MESSAGE_DELIVERY_STATUSES, ['pending', 'delivered', 'consumed', 'failed'])
  assert.deepEqual(WORKTREE_ISOLATION_MODES, ['shared_session', 'isolated_branch', 'isolated_worktree'])
  assert.deepEqual(WORKFLOW_EDGE_TYPES, ['handoff', 'loopback', 'fanout', 'fanin'])
  assert.deepEqual(STOP_RECOMMENDATIONS, ['continue', 'stop', 'complete'])
})

test('workflow-aware entity shapes serialize with queue and config state', () => {
  const user: User = { id: 'user_1' }
  const machine: Machine = { id: 'machine_1', userId: user.id, hostname: 'builder.local' }
  const workspace: Workspace = {
    id: 'workspace_1',
    userId: user.id,
    name: 'chariox',
    rootPath: '/workspace/chariox',
  }
  const worktree: Worktree = {
    id: 'worktree_1',
    workspaceId: workspace.id,
    gitBranch: 'feature/workflow',
    absolutePath: '/workspace/chariox-worktree',
  }
  const daemon: DaemonInstance = {
    id: 'daemon_1',
    machineId: machine.id,
    osUser: 'miguel',
    version: '0.1.0',
    status: 'online',
  }
  const session: Session = {
    id: 'session_1',
    workspaceId: workspace.id,
    worktreeId: worktree.id,
    hostMachineId: machine.id,
    hostDaemonId: daemon.id,
    executionMode: 'multi_agent_workflow',
    status: 'active',
    activeWorkflowRunId: 'workflow_run_1',
    activePromptId: 'prompt_1',
    focusedAgentId: 'agent_1',
    configVersion: 2,
    schedulerState: 'running',
    maxAgents: 4,
  }
  const attachment: SessionAttachment = {
    id: 'attachment_1',
    sessionId: session.id,
    clientId: 'client_1',
    capabilityLevel: 'full_terminal',
  }
  const queuedPrompt: PromptQueueItem = {
    id: 'prompt_1',
    sessionId: session.id,
    sourceAttachmentId: attachment.id,
    targetAgentId: 'agent_1',
    prompt: 'Implement the feature',
    status: 'running',
  }
  const configState: SessionConfigState = {
    sessionId: session.id,
    version: 2,
    values: { theme: 'compact' },
    updatedByAttachmentId: attachment.id,
  }
  const workflowDefinition: WorkflowDefinition = {
    id: 'workflow_definition_1',
    sessionId: session.id,
    topologyType: 'hierarchical',
    coordinatorNodeId: 'node_root',
    version: 1,
    validationPolicy: 'v1_hierarchical',
  }
  const workflowNode: WorkflowNode = {
    id: 'node_root',
    workflowDefinitionId: workflowDefinition.id,
    nodeKey: 'coordinator',
    label: 'Coordinator',
    role: 'coordinator',
    provider: 'claude-code',
    accountProfile: 'default',
    model: 'sonnet',
    concurrencyGroup: null,
    writesCode: true,
  }
  const workflowEdge: WorkflowEdge = {
    id: 'edge_1',
    workflowDefinitionId: workflowDefinition.id,
    fromNodeId: workflowNode.id,
    toNodeId: 'node_worker',
    edgeType: 'fanout',
    routingPolicy: 'all_children',
  }
  const workflowRun: WorkflowRun = {
    id: 'workflow_run_1',
    sessionId: session.id,
    workflowDefinitionId: workflowDefinition.id,
    coordinatorNodeId: workflowNode.id,
    status: 'running',
    iterationCount: 1,
    maxIterations: null,
    startedAt: '2026-03-17T12:00:00.000Z',
    completedAt: null,
  }
  const worktreeAssignment: WorktreeAssignment = {
    id: 'assignment_1',
    sessionId: session.id,
    workflowRunId: workflowRun.id,
    workflowNodeId: workflowNode.id,
    nodeRunId: 'node_run_1',
    worktreeId: worktree.id,
    branch: 'feature/workflow-node-root',
    isolationMode: 'isolated_worktree',
  }
  const providerRun: ProviderRun = {
    id: 'run_1',
    sessionId: session.id,
    agentInstanceId: 'agent_1',
    nodeRunId: 'node_run_1',
    worktreeAssignmentId: worktreeAssignment.id,
    provider: 'claude-code',
    accountProfile: 'default',
    model: 'sonnet',
    state: 'running',
  }
  const nodeRun: NodeRun = {
    id: 'node_run_1',
    workflowRunId: workflowRun.id,
    workflowNodeId: workflowNode.id,
    providerRunId: providerRun.id,
    worktreeAssignmentId: worktreeAssignment.id,
    status: 'running',
    summary: 'Coordinator is dispatching work',
    artifacts: ['docs/plan.md'],
    handoffPayload: '{"tasks":["implement"]}',
    stopRecommendation: 'continue',
    startedAt: '2026-03-17T12:00:01.000Z',
    completedAt: null,
  }
  const nodeMessage: NodeMessage = {
    id: 'message_1',
    workflowRunId: workflowRun.id,
    sourceNodeRunId: nodeRun.id,
    targetNodeId: 'node_worker',
    targetNodeRunId: null,
    messageType: 'handoff',
    summary: 'Execute child task',
    artifacts: ['docs/plan.md'],
    handoffPayload: '{"task":"child"}',
    deliveryStatus: 'delivered',
    createdAt: '2026-03-17T12:00:02.000Z',
  }
  const schedule: Schedule = {
    id: 'schedule_1',
    sessionId: session.id,
    cron: '0 * * * *',
    enabled: true,
  }
  const aggregationState: AggregationState = {
    id: 'aggregation_1',
    workflowRunId: workflowRun.id,
    workflowNodeId: workflowNode.id,
    barrierKey: 'children_complete',
    expectedInputs: 2,
    receivedInputs: 1,
    aggregationPolicy: 'wait_for_all',
    status: 'waiting',
  }

  const snapshot = JSON.parse(JSON.stringify({
    session,
    attachment,
    queuedPrompt,
    configState,
    workflowDefinition,
    workflowNode,
    workflowEdge,
    workflowRun,
    worktreeAssignment,
    providerRun,
    nodeRun,
    nodeMessage,
    schedule,
    aggregationState,
  })) as {
    session: Session
    attachment: SessionAttachment
    queuedPrompt: PromptQueueItem
    configState: SessionConfigState
    workflowDefinition: WorkflowDefinition
    workflowNode: WorkflowNode
    workflowEdge: WorkflowEdge
    workflowRun: WorkflowRun
    worktreeAssignment: WorktreeAssignment
    providerRun: ProviderRun
    nodeRun: NodeRun
    nodeMessage: NodeMessage
    schedule: Schedule
    aggregationState: AggregationState
  }

  assert.equal(snapshot.session.activePromptId, snapshot.queuedPrompt.id)
  assert.equal(snapshot.session.configVersion, snapshot.configState.version)
  assert.equal(snapshot.session.schedulerState, 'running')
  assert.equal(snapshot.queuedPrompt.sourceAttachmentId, snapshot.attachment.id)
  assert.equal(snapshot.configState.updatedByAttachmentId, snapshot.attachment.id)
  assert.equal(snapshot.providerRun.worktreeAssignmentId, snapshot.worktreeAssignment.id)
  assert.equal(snapshot.workflowDefinition.coordinatorNodeId, snapshot.workflowNode.id)
  assert.equal(snapshot.workflowEdge.edgeType, 'fanout')
  assert.equal(snapshot.nodeRun.providerRunId, snapshot.providerRun.id)
  assert.equal(snapshot.nodeMessage.workflowRunId, snapshot.workflowRun.id)
  assert.equal(snapshot.schedule.sessionId, snapshot.session.id)
  assert.equal(snapshot.aggregationState.workflowRunId, snapshot.workflowRun.id)
})
