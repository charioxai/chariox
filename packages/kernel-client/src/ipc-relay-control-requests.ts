export function relayStatusRequest() {
  return { RelayStatus: null }
}

export function configureRelayRequest(relayUrl: string | null, relayToken: string | null) {
  return {
    ConfigureRelay: {
      relay_url: relayUrl,
      relay_token: relayToken,
    },
  }
}

export function cloudRelayStatusRequest() {
  return { CloudRelayStatus: null }
}

export function startCloudRelayLoginRequest(apiUrl: string, input: {
  clientId?: string
  clientAlias?: string
  machineId?: string
  machineAlias?: string
}) {
  return {
    StartCloudRelayLogin: {
      api_url: apiUrl,
      client_id: input.clientId,
      client_alias: input.clientAlias,
      machine_id: input.machineId,
      machine_alias: input.machineAlias,
    },
  }
}

export function pollCloudRelayLoginRequest(apiUrl: string, deviceCode: string) {
  return {
    PollCloudRelayLogin: {
      api_url: apiUrl,
      device_code: deviceCode,
    },
  }
}

export function logoutCloudRelayRequest(options: { revokeClient?: boolean; revokeMachine?: boolean } = {}) {
  return {
    LogoutCloudRelay: {
      revoke_client: options.revokeClient ?? false,
      revoke_machine: options.revokeMachine ?? false,
    },
  }
}

export function pairCloudRelayClientRequest(clientId: string, alias?: string) {
  return {
    PairCloudRelayClient: {
      client_id: clientId,
      alias,
    },
  }
}

export function pairCloudRelayMachineRequest(machineId: string, alias?: string) {
  return {
    PairCloudRelayMachine: {
      machine_id: machineId,
      alias,
    },
  }
}

export function connectCloudRelayRequest() {
  return { ConnectCloudRelay: null }
}

export function issueCloudRelayClientTokenRequest(targetDaemonAlias: string, clientId: string, sessionId?: string | null) {
  return {
    IssueCloudRelayClientToken: {
      target_daemon_alias: targetDaemonAlias,
      client_id: clientId,
      session_id: sessionId ?? null,
    },
  }
}

export function createCloudSessionInviteRequest(
  sessionId: string,
  options: { displayName?: string | null; expiresInMs?: number | null; maxUses?: number | null } = {},
) {
  return {
    CreateCloudSessionInvite: {
      session_id: sessionId,
      display_name: options.displayName ?? null,
      expires_in_ms: options.expiresInMs ?? null,
      max_uses: options.maxUses ?? null,
    },
  }
}

export function showCloudSessionInviteRequest(inviteToken: string) {
  return {
    ShowCloudSessionInvite: {
      invite_token: inviteToken,
    },
  }
}

export function acceptCloudSessionInviteRequest(inviteToken: string) {
  return {
    AcceptCloudSessionInvite: {
      invite_token: inviteToken,
    },
  }
}

export function revokeCloudSessionInviteRequest(sessionId: string, inviteId: string) {
  return {
    RevokeCloudSessionInvite: {
      session_id: sessionId,
      invite_id: inviteId,
    },
  }
}

export function listCloudSessionMembersRequest(sessionId: string) {
  return {
    ListCloudSessionMembers: {
      session_id: sessionId,
    },
  }
}

export function listCloudCollaboratorsRequest() {
  return { ListCloudCollaborators: null }
}
