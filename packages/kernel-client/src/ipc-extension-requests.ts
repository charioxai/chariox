export function installMcpServerRequest(workspaceId: string | null, config: Record<string, unknown>) {
  return {
    InstallMcpServer: {
      workspace_id: workspaceId ?? null,
      config,
    },
  }
}

export function updateMcpServerRequest(workspaceId: string | null, config: Record<string, unknown>) {
  return {
    UpdateMcpServer: {
      workspace_id: workspaceId ?? null,
      config,
    },
  }
}

export function uninstallMcpServerRequest(workspaceId: string | null, name: string) {
  return {
    UninstallMcpServer: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function getMcpServerRequest(workspaceId: string | null, name: string) {
  return {
    GetMcpServer: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function importMcpServersRequest(workspaceId: string | null, provider: string, name?: string | null) {
  return {
    ImportMcpServers: {
      workspace_id: workspaceId ?? null,
      provider,
      name: name ?? null,
    },
  }
}

export function getSkillRequest(workspaceId: string | null, name: string) {
  return {
    GetSkill: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function installSkillRequest(workspaceId: string | null, sourcePath: string) {
  return {
    InstallSkill: {
      workspace_id: workspaceId ?? null,
      source_path: sourcePath,
    },
  }
}

export function updateSkillRequest(workspaceId: string | null, sourcePath: string) {
  return {
    UpdateSkill: {
      workspace_id: workspaceId ?? null,
      source_path: sourcePath,
    },
  }
}

export function uninstallSkillRequest(workspaceId: string | null, name: string) {
  return {
    UninstallSkill: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function importSkillsRequest(workspaceId: string | null, provider: string, name?: string | null) {
  return {
    ImportSkills: {
      workspace_id: workspaceId ?? null,
      provider,
      name: name ?? null,
    },
  }
}

export function grantAgentCapabilityRequest(
  workspaceId: string | null,
  agentRef: string,
  kind: "mcp" | "skill",
  name: string,
) {
  return {
    GrantAgentCapability: {
      workspace_id: workspaceId ?? null,
      agent_ref: agentRef,
      kind,
      name,
    },
  }
}

export function revokeAgentCapabilityRequest(agentRef: string, kind: "mcp" | "skill", name: string) {
  return {
    RevokeAgentCapability: {
      agent_ref: agentRef,
      kind,
      name,
    },
  }
}

export function listMcpServersRequest(workspaceId?: string | null) {
  return {
    ListMcpServers: {
      workspace_id: workspaceId ?? null,
    },
  }
}

export function listSkillsRequest(workspaceId?: string | null) {
  return {
    ListSkills: {
      workspace_id: workspaceId ?? null,
    },
  }
}
