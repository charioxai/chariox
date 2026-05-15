export * from "./ipc-workflow-requests.js"
export * from "./ipc-workspace-requests.js"
export * from "./ipc-remote-connection-requests.js"
export * from "./ipc-relay-control-requests.js"
export * from "./ipc-extension-requests.js"
export * from "./ipc-history-requests.js"
export * from "./ipc-session-requests.js"
export * from "./ipc-terminal-runtime-requests.js"
export * from "./ipc-provider-requests.js"
export * from "./ipc-agent-requests.js"
export * from "./ipc-slice-requests.js"

export function deleteKernelRequest() {
  return { DeleteKernel: null }
}

export function getDaemonHealthRequest() {
  return { GetDaemonHealth: null }
}

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

export function setCredentialSecretRequest(key: string, value: string) {
  return {
    SetCredentialSecret: {
      key,
      value,
    },
  }
}

export function deleteCredentialSecretRequest(key: string) {
  return {
    DeleteCredentialSecret: {
      key,
    },
  }
}
