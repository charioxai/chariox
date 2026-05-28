function addWorkflowNodeRequest(sessionId, workflowRef, agentId, expectedRevision = null) {
  return {
    AddWorkflowNode: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      agent_id: agentId,
      expected_workflow_revision: expectedRevision,
    },
  }
}

function updateWorkflowNodeInstructionsRequest(sessionId, workflowRef, nodeId, instructions, expectedRevision = null) {
  return {
    UpdateWorkflowNodeInstructions: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      instructions,
      expected_workflow_revision: expectedRevision,
    },
  }
}

function createWorkflowEndpointRequest(sessionId, workflowRef, entryNodeId, alias, expectedRevision = null) {
  return {
    CreateWorkflowEndpoint: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      entry_node_id: entryNodeId,
      alias,
      expected_workflow_revision: expectedRevision,
    },
  }
}

function addWorkflowEdgeRequest(sessionId, workflowRef, fromNodeId, toNodeId, expectedRevision = null) {
  return {
    AddWorkflowEdge: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      from_node_id: fromNodeId,
      to_node_id: toNodeId,
      output_schema_ref: null,
      validation_policy: null,
      expected_workflow_revision: expectedRevision,
    },
  }
}

function removeWorkflowEdgeRequest(sessionId, workflowRef, edgeId, expectedRevision = null) {
  return {
    RemoveWorkflowEdge: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      edge_id: edgeId,
      expected_workflow_revision: expectedRevision,
    },
  }
}

export async function runHostedMultiUserAssertions({
  LocalIpcClient,
  requests,
  localClient,
  ownerRemoteClient,
  ownerProfile,
  ownerClientId,
  workspace,
  daemonAlias,
  session,
  apiUrl,
  log,
  assert,
  unwrap,
  postJson,
  issueSessionScopedClientToken,
  manualCloudDeviceLogin,
  installSendRetry,
  expectReject,
}) {
  log("multi-user-cloud-invites")
  const localInvite = unwrap(
    await ownerRemoteClient.send(requests.createSessionInviteRequest(session.id, null, 2)),
    "SessionInviteCreated",
  )
  const cloudInvite = unwrap(
    await ownerRemoteClient.send(requests.createCloudSessionInviteRequest(session.id, {
      displayName: "Hosted cloud relay multi-user drill",
      maxUses: 2,
    })),
    "CloudSessionInviteCreated",
  )
  const localInviteToken = localInvite.invite?.invite_token
  const cloudInviteToken = cloudInvite.invite?.invite_token
  assert(localInviteToken, "local session invite token should be returned", localInvite)
  assert(cloudInviteToken, "cloud session invite token should be returned", cloudInvite)

  const ownerScopedToken = await issueSessionScopedClientToken(apiUrl, {
    sessionToken: ownerProfile.cloudSessionToken,
    accountId: ownerProfile.accountId,
    realmId: ownerProfile.realmId,
    subject: ownerClientId,
    userId: ownerProfile.userId,
    clientId: ownerClientId,
    sessionId: session.id,
    targetDaemonAlias: daemonAlias,
  })
  const ownerScopedClient = installSendRetry(new LocalIpcClient(ownerProfile.relayUrl, {
    relayAuthToken: ownerScopedToken,
    targetDaemonAlias: daemonAlias,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  }), "owner-scoped-relay")

  const peerClientId = `${ownerClientId}-peer`
  const thirdClientId = `${ownerClientId}-third`
  let peerRemoteClient = null
  let thirdRemoteClient = null
  try {
    const peerLogin = await manualCloudDeviceLogin({
      role: "peer",
      clientId: peerClientId,
      clientAlias: "hosted-peer-cli",
      localClient,
      requests,
    })
    const thirdLogin = await manualCloudDeviceLogin({
      role: "third",
      clientId: thirdClientId,
      clientAlias: "hosted-third-cli",
      localClient,
      requests,
    })
    const peerProfile = peerLogin.profile
    const thirdProfile = thirdLogin.profile
    assert(peerProfile.userId !== ownerProfile.userId, "peer login must use a different Auth0 user from owner", {
      ownerUserId: ownerProfile.userId,
      peerUserId: peerProfile.userId,
    })
    assert(thirdProfile.userId !== ownerProfile.userId && thirdProfile.userId !== peerProfile.userId, "third login must use a distinct Auth0 user", {
      ownerUserId: ownerProfile.userId,
      peerUserId: peerProfile.userId,
      thirdUserId: thirdProfile.userId,
    })

    log("peer-accept-cloud-invite")
    const peerAcceptance = await postJson(`${apiUrl}/sessions/invites/${encodeURIComponent(cloudInviteToken)}/accept`, {
      sessionToken: peerLogin.cloudSessionToken,
    })
    assert(peerAcceptance.userId === peerProfile.userId, "peer should accept the cloud invite as itself", peerAcceptance)

    log("third-accept-cloud-invite")
    const thirdAcceptance = await postJson(`${apiUrl}/sessions/invites/${encodeURIComponent(cloudInviteToken)}/accept`, {
      sessionToken: thirdLogin.cloudSessionToken,
    })
    assert(thirdAcceptance.userId === thirdProfile.userId, "third user should accept the cloud invite as itself", thirdAcceptance)

    const peerRelayToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: peerLogin.cloudSessionToken,
      accountId: ownerProfile.accountId,
      realmId: ownerProfile.realmId,
      subject: peerClientId,
      userId: peerProfile.userId,
      clientId: peerClientId,
      sessionId: session.id,
      targetDaemonAlias: daemonAlias,
    })
    const thirdRelayToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: thirdLogin.cloudSessionToken,
      accountId: ownerProfile.accountId,
      realmId: ownerProfile.realmId,
      subject: thirdClientId,
      userId: thirdProfile.userId,
      clientId: thirdClientId,
      sessionId: session.id,
      targetDaemonAlias: daemonAlias,
    })

    peerRemoteClient = installSendRetry(new LocalIpcClient(ownerProfile.relayUrl, {
      relayAuthToken: peerRelayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    }), "peer-relay")
    thirdRemoteClient = installSendRetry(new LocalIpcClient(ownerProfile.relayUrl, {
      relayAuthToken: thirdRelayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    }), "third-relay")

    await peerRemoteClient.send(requests.joinSessionInviteRequest(localInviteToken, peerProfile.userId))
    await thirdRemoteClient.send(requests.joinSessionInviteRequest(localInviteToken, thirdProfile.userId))
    const peerAttached = unwrap(
      await peerRemoteClient.send(requests.attachToSessionRequest(session.id, `${peerClientId}-remote`)),
      "SessionAttached",
    )
    assert(peerAttached.attachment?.session_id === session.id, "peer should attach to joined session", peerAttached)
    const thirdAttached = unwrap(
      await thirdRemoteClient.send(requests.attachToSessionRequest(session.id, `${thirdClientId}-remote`)),
      "SessionAttached",
    )
    assert(thirdAttached.attachment?.session_id === session.id, "third user should attach to joined session", thirdAttached)
    const members = unwrap(
      await peerRemoteClient.send(requests.listSessionMembersRequest(session.id)),
      "SessionMembersListed",
    )
    assert(members.members?.some((member) => member.user_id === peerProfile.userId), "peer should appear in kernel session members", members)
    assert(members.members?.some((member) => member.user_id === thirdProfile.userId), "third should appear in kernel session members", members)

    const ownerAgent = unwrap(
      await ownerScopedClient.send(requests.spawnAgentRequest(session.id, "dev-stub", "owner-agent", "multi-user-drill", workspace, "low")),
      "AgentSpawned",
    ).agent
    const peerAgent = unwrap(
      await peerRemoteClient.send(requests.spawnAgentRequest(session.id, "dev-stub", "peer-agent", "multi-user-drill", workspace, "low")),
      "AgentSpawned",
    ).agent
    assert(ownerAgent.owner_user_id === ownerProfile.userId, "owner agent should use owner cloud user id", ownerAgent)
    assert(peerAgent.owner_user_id === peerProfile.userId, "peer agent should use peer cloud user id", peerAgent)

    const peerAgents = unwrap(
      await peerRemoteClient.send(requests.listAgentsRequest(session.id)),
      "AgentsListed",
    ).agents
    assert(
      peerAgents.some((agent) => agent.id === peerAgent.id && agent.visible_in_freeform !== false),
      "peer should list its own agent as freeform-visible",
      peerAgents,
    )
    const redactedPeerOwnerAgent = peerAgents.find((agent) => agent.id === ownerAgent.id)
    assert(
      redactedPeerOwnerAgent?.provider === "redacted"
        && redactedPeerOwnerAgent?.model == null
        && redactedPeerOwnerAgent?.visible_in_freeform === false,
      "peer should list owner agent only as a redacted workflow-selectable handle",
      peerAgents,
    )

    const workflow = unwrap(
      await ownerScopedClient.send(requests.createWorkflowRequest(session.id, "hosted-cloud-session-scoped-flow")),
      "WorkflowCreated",
    ).workflow
    const ownerNode = unwrap(
      await ownerScopedClient.send(addWorkflowNodeRequest(session.id, workflow.id, ownerAgent.id, workflow.revision)),
      "WorkflowNodeAdded",
    ).node
    await ownerScopedClient.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      ownerNode.id,
      "private hosted owner prompt",
    ))

    const beforePeerNode = unwrap(
      await peerRemoteClient.send(requests.resolveWorkflowRequest(session.id, workflow.id)),
      "WorkflowResolved",
    ).workflow
    const peerNode = unwrap(
      await peerRemoteClient.send(addWorkflowNodeRequest(session.id, workflow.id, peerAgent.id, beforePeerNode.revision)),
      "WorkflowNodeAdded",
    ).node
    await peerRemoteClient.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      peerNode.id,
      "private hosted peer prompt",
    ))
    const endpoint = unwrap(
      await ownerScopedClient.send(createWorkflowEndpointRequest(session.id, workflow.id, ownerNode.id, "owner-hosted-entry")),
      "WorkflowEndpointCreated",
    ).endpoint
    await expectReject(
      peerRemoteClient.send(requests.invokeWorkflowEndpointRequest(session.id, workflow.id, endpoint.id, "should be denied")),
      "peer invoking owner endpoint",
      "owned by",
    )

    const beforeEdge = unwrap(
      await peerRemoteClient.send(requests.resolveWorkflowRequest(session.id, workflow.id)),
      "WorkflowResolved",
    ).workflow
    const edge = unwrap(
      await peerRemoteClient.send(addWorkflowEdgeRequest(session.id, workflow.id, ownerNode.id, peerNode.id, beforeEdge.revision)),
      "WorkflowEdgeAdded",
    ).edge
    assert(edge.created_by_user_id === peerProfile.userId, "cross-owner edge should record peer cloud user id", edge)
    await expectReject(
      thirdRemoteClient.send(removeWorkflowEdgeRequest(session.id, workflow.id, edge.id)),
      "third user removing unrelated edge",
      "cannot perform",
    )

    const beforeStaleMutation = unwrap(
      await ownerScopedClient.send(requests.resolveWorkflowRequest(session.id, workflow.id)),
      "WorkflowResolved",
    ).workflow
    await peerRemoteClient.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      peerNode.id,
      "private hosted peer prompt after revision bump",
    ))
    await expectReject(
      ownerScopedClient.send(updateWorkflowNodeInstructionsRequest(
        session.id,
        workflow.id,
        ownerNode.id,
        "stale private hosted owner prompt",
        beforeStaleMutation.revision,
      )),
      "stale workflow revision mutation",
      "expected",
    )

    const freshWorkflow = unwrap(
      await ownerScopedClient.send(requests.resolveWorkflowRequest(session.id, workflow.id)),
      "WorkflowResolved",
    ).workflow
    const removedWorkflow = unwrap(
      await ownerScopedClient.send(removeWorkflowEdgeRequest(session.id, workflow.id, edge.id, freshWorkflow.revision)),
      "WorkflowEdgeRemoved",
    ).workflow
    assert(removedWorkflow.edges.length === 0, "owner should remove edge incident to its own node", removedWorkflow)

    const peerStatePayload = unwrap(
      await peerRemoteClient.send(requests.getSessionStateRequest(session.id)),
      "SessionState",
    )
    const peerState = peerStatePayload.session ?? peerStatePayload.state ?? peerStatePayload
    assert(
      peerState.agents.some((agent) => agent.id === peerAgent.id && agent.visible_in_freeform !== false),
      "peer state should keep own agent freeform-visible",
      peerState.agents,
    )
    const redactedStateOwnerAgent = peerState.agents.find((agent) => agent.id === ownerAgent.id)
    assert(
      redactedStateOwnerAgent?.provider === "redacted"
        && redactedStateOwnerAgent?.model == null
        && redactedStateOwnerAgent?.visible_in_freeform === false,
      "peer state should redact owner agent parameters while preserving the workflow handle",
      peerState.agents,
    )
    const redactedWorkflow = peerState.workflows.find((entry) => entry.id === workflow.id)
    assert(redactedWorkflow, "peer should see shared workflow graph", peerState.workflows)
    const redactedOwnerNode = redactedWorkflow.nodes.find((node) => node.id === ownerNode.id)
    const visiblePeerNode = redactedWorkflow.nodes.find((node) => node.id === peerNode.id)
    assert(redactedOwnerNode, "peer should see owner node shell", redactedWorkflow)
    assert(visiblePeerNode, "peer should see own node", redactedWorkflow)
    assert(redactedOwnerNode.instructions == null, "owner node instructions should be redacted from peer", redactedOwnerNode)
    assert(
      visiblePeerNode.instructions === "private hosted peer prompt after revision bump",
      "peer node instructions should remain visible to peer",
      visiblePeerNode,
    )
  } finally {
    await thirdRemoteClient?.close().catch(() => {})
    await peerRemoteClient?.close().catch(() => {})
    await ownerScopedClient.close().catch(() => {})
  }
}
