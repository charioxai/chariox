export async function joinRemoteHomeExtensionCollaborator({
  LocalIpcClient,
  client,
  relayUrl,
  clientToken,
  createSessionInviteRequest,
  joinSessionInviteRequest,
  sessionId,
  targetDaemonAlias = "home",
  userId = "user-2",
}) {
  const invite = unwrap(
    await client.send(createSessionInviteRequest(sessionId, null, 1, "full")),
    "SessionInviteCreated",
  ).invite
  const userClient = new LocalIpcClient(relayUrl, {
    relayAuthToken: clientToken(userId),
    targetDaemonAlias,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  await userClient.send(joinSessionInviteRequest(invite.invite_token, userId))
  return userClient
}

function unwrap(response, key) {
  return response?.[key] ?? response
}
