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

export function grantAgentExtensionRequest(
  workspaceId: string | null,
  agentRef: string,
  kind: "mcp" | "skill" | "script",
  name: string,
  environment?: string | null,
) {
  return {
    GrantAgentExtension: {
      workspace_id: workspaceId ?? null,
      agent_ref: agentRef,
      kind,
      name,
      ...(environment ? { environment } : {}),
    },
  }
}

export function revokeAgentExtensionRequest(agentRef: string, kind: "mcp" | "skill" | "script", name: string) {
  return {
    RevokeAgentExtension: {
      agent_ref: agentRef,
      kind,
      name,
    },
  }
}

export function registerEnvironmentRequest(workspaceId: string | null, config: Record<string, unknown>) {
  return {
    RegisterEnvironment: {
      workspace_id: workspaceId ?? null,
      config,
    },
  }
}

export function removeEnvironmentRequest(workspaceId: string | null, name: string) {
  return {
    RemoveEnvironment: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function getEnvironmentRequest(workspaceId: string | null, name: string) {
  return {
    GetEnvironment: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function listEnvironmentsRequest(workspaceId?: string | null) {
  return {
    ListEnvironments: {
      workspace_id: workspaceId ?? null,
    },
  }
}

export function validateScriptRequest(workspaceId: string | null, sourcePath: string, environment: string, name?: string | null) {
  return {
    ValidateScript: {
      workspace_id: workspaceId ?? null,
      source_path: sourcePath,
      environment,
      name: name ?? null,
    },
  }
}

export function registerScriptRequest(workspaceId: string | null, sourcePath: string, environment: string, name?: string | null) {
  return {
    RegisterScript: {
      workspace_id: workspaceId ?? null,
      source_path: sourcePath,
      environment,
      name: name ?? null,
    },
  }
}

export function removeScriptRequest(workspaceId: string | null, name: string) {
  return {
    RemoveScript: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function getScriptRequest(workspaceId: string | null, name: string) {
  return {
    GetScript: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function listScriptsRequest(workspaceId?: string | null) {
  return {
    ListScripts: {
      workspace_id: workspaceId ?? null,
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
