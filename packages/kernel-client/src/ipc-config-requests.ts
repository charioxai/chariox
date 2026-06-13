export function updateSessionConfigRequest(
  sessionId: string,
  attachmentId: string,
  values: Record<string, string>,
  requiresIdle = false,
) {
  return {
    UpdateSessionConfig: {
      session_id: sessionId,
      attachment_id: attachmentId,
      values,
      requires_idle: requiresIdle,
    },
  }
}

export function getUserConfigRequest() {
  return { GetUserConfig: null }
}

export function getUserConfigSchemaRequest() {
  return { GetUserConfigSchema: null }
}

export function setUserConfigValueRequest(path: string, value: string) {
  return {
    SetUserConfigValue: {
      path,
      value,
    },
  }
}

export function unsetUserConfigValueRequest(path: string) {
  return {
    UnsetUserConfigValue: {
      path,
    },
  }
}

export type CredentialVaultRequestContext = {
  readonly sessionId?: string | null
  readonly agentId?: string | null
}

export function setCredentialSecretRequest(key: string, value: string, context: CredentialVaultRequestContext = {}) {
  return {
    SetCredentialSecret: {
      ...(context.sessionId ? { session_id: context.sessionId } : {}),
      ...(context.agentId ? { agent_id: context.agentId } : {}),
      key,
      value,
    },
  }
}

export function deleteCredentialSecretRequest(key: string, context: CredentialVaultRequestContext = {}) {
  return {
    DeleteCredentialSecret: {
      ...(context.sessionId ? { session_id: context.sessionId } : {}),
      ...(context.agentId ? { agent_id: context.agentId } : {}),
      key,
    },
  }
}

export function getCredentialVaultStatusRequest() {
  return { GetCredentialVaultStatus: null }
}

export function lockCredentialVaultRequest() {
  return { LockCredentialVault: null }
}

export function manageCredentialVaultRequest(sessionId: string, agentId?: string | null) {
  return {
    ManageCredentialVault: {
      session_id: sessionId,
      ...(agentId ? { agent_id: agentId } : {}),
    },
  }
}
