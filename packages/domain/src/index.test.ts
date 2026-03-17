import assert from 'node:assert/strict'
import test from 'node:test'

import {
  AGGREGATION_STATUSES,
  CLIENT_CAPABILITY_LEVELS,
  DAEMON_STATUSES,
  NODE_MESSAGE_DELIVERY_STATUSES,
  NODE_RUN_STATUSES,
  PROVIDER_RUN_STATES,
  SESSION_EXECUTION_MODES,
  SESSION_STATUSES,
  STOP_RECOMMENDATIONS,
  WORKFLOW_EDGE_TYPES,
  WORKFLOW_RUN_STATUSES,
  WORKFLOW_TOPOLOGIES,
  WORKTREE_ISOLATION_MODES,
  type AggregationState,
  type ControllerLease,
  type DaemonInstance,
  type Machine,
  type NodeMessage,
  type NodeRun,
  type ProviderRun,
  type Schedule,
  type Session,
  type SessionAttachment,
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
  assert.deepEqual(CLIENT_CAPABILITY_LEVELS, [
    'full_terminal',
    'interactive_structured',
    'message_transport',
    'automation_only',
  ])
  assert.deepEqual(WORKFLOW_TOPOLOGIES, ['circular', 'hierarchical'])
  assert.deepEqual(WORKFLOW_RUN_STATUSES, ['created', 'running', 'waiting', 'completed', 'failed', 'stopped'])
  assert.deepEqual(NODE_RUN_STATUSES, ['created', 'ready', 'running', 'waiting', 'completed', 'failed', 'stopped'])
  assert.deepEqual(AGGREGATION_STATUSES, ['idle', 'waiting', 'ready', 'completed', 'failed'])
  assert.deepEqual(NODE_MESSAGE_DELIVERY_STATUSES, ['pending', 'delivered', 'consumed', 'failed'])
  assert.deepEqual(WORKTREE_ISOLATION_MODES, ['shared_session', 'isolated_branch', 'isolated_worktree'])
  assert.deepEqual(WORKFLOW_EDGE_TYPES, ['handoff', 'loopback', 'fanout', 'fanin'])
  assert.deepEqual(STOP_RECOMMENDATIONS, ['continue', 'stop', 'complete'])
})

test('workflow-aware entity shapes serialize with the expected identifiers and relations', () => {
  const user: User = { id: 'user_1' }
  const machine: Machine = { id: 'machine_1', userId: user.id, hostname: 'builder.local' }
  const workspace: Workspace = {
    id: 'workspace_1',
    userId: user.id,
    name: 'arroba',
    rootPath: '/workspace/arroba',
  }
  const worktree: Worktree = {
    id: 'worktree_1',
    workspaceId: workspace.id,
    gitBranch: 'feature/workflow',
    absolutePath: '/workspace/arroba-worktree',
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
    activeProviderRunId: 'run_1',
    activeWorkflowRunId: 'workflow_run_1',
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
  const attachment: SessionAttachment = {
    id: 'attachment_1',
    sessionId: session.id,
    clientId: 'client_1',
    capabilityLevel: 'full_terminal',
    isObserver: false,
  }
  const lease: ControllerLease = {
    id: 'lease_1',
    sessionId: session.id,
    attachmentId: attachment.id,
    grantedAt: '2026-03-17T12:00:03.000Z',
    expiresAt: null,
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

  const snapshot = JSON.parse(
    JSON.stringify({
      user,
      machine,
      workspace,
      worktree,
      daemon,
      session,
      workflowDefinition,
      workflowNode,
      workflowEdge,
      workflowRun,
      worktreeAssignment,
      providerRun,
      nodeRun,
      nodeMessage,
      attachment,
      lease,
      schedule,
      aggregationState,
    }),
  ) as {
    session: Session
    workflowDefinition: WorkflowDefinition
    workflowNode: WorkflowNode
    workflowEdge: WorkflowEdge
    workflowRun: WorkflowRun
    worktreeAssignment: WorktreeAssignment
    providerRun: ProviderRun
    nodeRun: NodeRun
    nodeMessage: NodeMessage
    attachment: SessionAttachment
    lease: ControllerLease
    schedule: Schedule
    aggregationState: AggregationState
  }

  assert.equal(snapshot.session.executionMode, 'multi_agent_workflow')
  assert.equal(snapshot.session.activeWorkflowRunId, snapshot.workflowRun.id)
  assert.equal(snapshot.providerRun.worktreeAssignmentId, snapshot.worktreeAssignment.id)
  assert.equal(snapshot.workflowDefinition.coordinatorNodeId, snapshot.workflowNode.id)
  assert.equal(snapshot.workflowEdge.edgeType, 'fanout')
  assert.equal(snapshot.nodeRun.providerRunId, snapshot.providerRun.id)
  assert.equal(snapshot.nodeRun.worktreeAssignmentId, snapshot.worktreeAssignment.id)
  assert.equal(snapshot.nodeRun.stopRecommendation, 'continue')
  assert.equal(snapshot.nodeMessage.deliveryStatus, 'delivered')
  assert.equal(snapshot.nodeMessage.workflowRunId, snapshot.workflowRun.id)
  assert.equal(snapshot.attachment.sessionId, snapshot.session.id)
  assert.equal(snapshot.lease.attachmentId, snapshot.attachment.id)
  assert.equal(snapshot.schedule.sessionId, snapshot.session.id)
  assert.equal(snapshot.aggregationState.workflowRunId, snapshot.workflowRun.id)
})
