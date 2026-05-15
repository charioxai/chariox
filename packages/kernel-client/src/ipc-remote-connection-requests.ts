export function listRemoteMachinesRequest() {
  return { ListRemoteMachines: null }
}

export function listRemoteMachineKernelsRequest(machineRef: string) {
  return {
    ListRemoteMachineKernels: {
      machine_ref: machineRef,
    },
  }
}

export function getWaitingRoomInventoryRequest() {
  return { GetWaitingRoomInventory: null }
}

export function getWaitingRoomPublicSnapshotRequest() {
  return { GetWaitingRoomPublicSnapshot: null }
}

export function approveRemoteMachineRequest(machineRef: string) {
  return {
    ApproveRemoteMachine: {
      machine_ref: machineRef,
    },
  }
}

export function forgetRemoteMachineRequest(machineRef: string) {
  return {
    ForgetRemoteMachine: {
      machine_ref: machineRef,
    },
  }
}

export function renameRemoteMachineRequest(machineRef: string, alias: string) {
  return {
    RenameRemoteMachine: {
      machine_ref: machineRef,
      alias,
    },
  }
}

export function createPairingInviteRequest(
  intent: "client" | "machine",
  alias: string | null = null,
  expiresInMs: number | null = null,
  terminalType: "cli" | "web" | "ios" | "android" | null = null,
) {
  return {
    CreatePairingInvite: {
      intent,
      alias,
      expires_in_ms: expiresInMs,
      ...(terminalType ? { terminal_type: terminalType } : {}),
    },
  }
}

export function createTerminalPairingLinkRequest(
  terminalType: "cli" | "web" | "ios" | "android" | null = "cli",
  alias: string | null = null,
  expiresInMs: number | null = null,
) {
  return {
    CreateTerminalPairingLink: {
      terminal_type: terminalType,
      alias,
      expires_in_ms: expiresInMs,
    },
  }
}

export function joinTerminalPairingLinkRequest(
  pairingLink: string,
  terminalId: string | null = null,
  terminalType: "cli" | "web" | "ios" | "android" | null = null,
  alias: string | null = null,
) {
  return {
    JoinTerminalPairingLink: {
      pairing_link: pairingLink,
      terminal_id: terminalId,
      terminal_type: terminalType,
      alias,
    },
  }
}

export function listTerminalsRequest() {
  return { ListTerminals: null }
}

export function joinPairingInviteRequest(
  inviteToken: string,
  subjectId: string | null = null,
  publicKeyThumbprint: string | null = null,
  alias: string | null = null,
) {
  return {
    JoinPairingInvite: {
      invite_token: inviteToken,
      subject_id: subjectId,
      public_key_thumbprint: publicKeyThumbprint,
      alias,
    },
  }
}

export function listPairedClientsRequest() {
  return { ListPairedClients: null }
}

export function recordPairedClientRequest(
  clientId: string,
  publicKeyThumbprint: string,
  alias: string | null = null,
  pairedAtMs: number | null = null,
) {
  return {
    RecordPairedClient: {
      client_id: clientId,
      public_key_thumbprint: publicKeyThumbprint,
      alias,
      paired_at_ms: pairedAtMs,
    },
  }
}

export function revokePairedClientRequest(clientId: string) {
  return {
    RevokePairedClient: {
      client_id: clientId,
    },
  }
}
