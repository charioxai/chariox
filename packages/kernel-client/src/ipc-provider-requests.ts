export function listProviderProcessesRequest(provider?: string | null) {
  return {
    ListProviderProcesses: {
      provider: provider ?? null,
    },
  }
}

export function teardownProviderProcessesRequest(provider?: string | null, force = false) {
  return {
    TeardownProviderProcesses: {
      provider: provider ?? null,
      force,
    },
  }
}

export function getProviderRunRequest(providerRunId: string) {
  return {
    GetProviderRun: {
      provider_run_id: providerRunId,
    },
  }
}

export function updateProviderRunSelectionRequest(
  sessionId: string,
  providerRunId: string,
  options: { model?: string | null; variant?: string | null; clearVariant?: boolean } = {},
) {
  return {
    UpdateProviderRunSelection: {
      session_id: sessionId,
      provider_run_id: providerRunId,
      model: options.model ?? null,
      variant: options.variant ?? null,
      clear_variant: options.clearVariant ?? false,
    },
  }
}

export function getProviderCatalogRequest() {
  return { GetProviderCatalog: null }
}

export function getProviderCommandCatalogsRequest() {
  return { GetProviderCommandCatalogs: null }
}

export function getProviderAuthStatusRequest(provider: string) {
  return {
    GetProviderAuthStatus: {
      provider,
    },
  }
}

export function startProviderLoginRequest(provider: string) {
  return {
    StartProviderLogin: {
      provider,
    },
  }
}

export function logoutProviderRequest(provider: string) {
  return {
    LogoutProvider: {
      provider,
    },
  }
}

export function launchProviderRunRequest(
  sessionId: string,
  provider: string,
  accountProfile: string,
  model: string,
  effort: string,
  agentId?: string | null,
  native?: {
    structuredEndpoint?: string | null
    providerSessionId?: string | null
    nativeTui?: boolean | null
  } | null,
) {
  const normalizedModel = provider === "codex" && model.startsWith("codex/")
    ? model.slice("codex/".length)
    : model
  return {
    LaunchProviderRun: {
      session_id: sessionId,
      agent_id: agentId ?? null,
      adapter_key: provider,
      provider,
      account_profile: accountProfile,
      model: normalizedModel,
      variant: effort.trim() || null,
      structured_endpoint: native?.structuredEndpoint ?? null,
      provider_session_id: native?.providerSessionId ?? null,
      native_tui: native?.nativeTui ?? false,
    },
  }
}
