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
  requests,
  ownerRemoteClient,
  ownerProfile,
  ownerTerminal,
  modules,
  stateRoot,
  homeDaemonId,
  connectSessionScopedCloudClient,
  cleanupHostedCloudTerminal,
  workspace,
  session,
  log,
  assert,
  unwrap,
  manualCloudDeviceLogin,
  installSendRetry,
  expectReject,
}) {
  log("multi-user-cloud-invites")
  const localInvite = unwrap(
    await ownerRemoteClient.send(requests.createSessionInviteRequest(session.id, null, 2)),
    "SessionInviteCreated",
  )
  // MP-08 / MP-10 / MP-11: human collaboration uses each terminal's CLIENT
  // authority; only local session membership mutations go through the kernel.
  const cloudInvite = await ownerTerminal.client.collaboration.createSessionInvite(session.id, {
    displayName: "Hosted cloud relay multi-user drill", maxUses: 2,
  })
  const localInviteToken = localInvite.invite?.invite_token
  const cloudInviteToken = cloudInvite.invite?.invite_token
  assert(localInviteToken, "local session invite token should be returned")
  assert(cloudInviteToken, "cloud session invite token should be returned")
  const scope = {accountId: ownerProfile.accountId, realmId: ownerProfile.realmId, sessionId: session.id, targetDaemonId: homeDaemonId}
  let ownerScopedClient = null, peerRemoteClient = null, thirdRemoteClient = null
  let peerLogin = null, thirdLogin = null, peerClientId = null, thirdClientId = null
  let scenarioFailure = null
  try {
    ownerScopedClient = installSendRetry(await connectSessionScopedCloudClient(modules, ownerTerminal, scope), "owner-scoped-relay")
    peerLogin = await manualCloudDeviceLogin({role: "peer", stateRoot, modules})
    thirdLogin = await manualCloudDeviceLogin({role: "third", stateRoot, modules})
    const peerProfile = peerLogin.profile, thirdProfile = thirdLogin.profile
    peerClientId = peerProfile.clientId
    thirdClientId = thirdProfile.clientId
    assert(peerProfile.userId !== ownerProfile.userId, "peer login must use a different Auth0 user from owner")
    assert(thirdProfile.userId !== ownerProfile.userId && thirdProfile.userId !== peerProfile.userId, "third login must use a distinct Auth0 user")

    log("peer-accept-cloud-invite")
    const peerAcceptance = await peerLogin.client.collaboration.acceptSessionInvite(cloudInviteToken)
    assert(peerAcceptance.acceptance.user_id === peerProfile.userId, "peer should accept the cloud invite as itself")
    log("third-accept-cloud-invite")
    const thirdAcceptance = await thirdLogin.client.collaboration.acceptSessionInvite(cloudInviteToken)
    assert(thirdAcceptance.acceptance.user_id === thirdProfile.userId, "third user should accept the cloud invite as itself")

    peerRemoteClient = installSendRetry(await connectSessionScopedCloudClient(modules, peerLogin, scope), "peer-relay")
    thirdRemoteClient = installSendRetry(await connectSessionScopedCloudClient(modules, thirdLogin, scope), "third-relay")

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
  } catch (error) {
    scenarioFailure = error
    throw error
  } finally {
    await thirdRemoteClient?.close().catch(() => {})
    await peerRemoteClient?.close().catch(() => {})
    await ownerScopedClient?.close().catch(() => {})
    const cleanupErrors = []
    for (const identity of [
      { login: peerLogin, clientId: peerClientId, role: "peer" },
      { login: thirdLogin, clientId: thirdClientId, role: "third" },
    ]) {
      if (!identity.login) continue
      await cleanupHostedCloudTerminal(identity.login, {
        reason: `hosted Cloud ${identity.role} drill cleanup`,
      }).catch(error => cleanupErrors.push(error))
    }
    if (cleanupErrors.length > 0) {
      log("multi-user-cloud-cleanup-failed", {
        errors: cleanupErrors.map((error) => error instanceof Error ? error.message : String(error)),
      })
      if (!scenarioFailure) {
        throw new AggregateError(cleanupErrors, "hosted Cloud multi-user identity cleanup failed")
      }
    }
  }
}
