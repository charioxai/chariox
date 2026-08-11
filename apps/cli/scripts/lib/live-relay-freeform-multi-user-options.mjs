import { providerModelSelectionMatches } from "./provider-model-selection.mjs"

export function collaborationProviderModel(provider, model) {
  if (provider === "opencode" && !model.includes("/")) return `opencode/${model}`
  if (provider === "codex" && !model.includes("/")) return codexCliModel(model)
  return model
}

export function collaborationSessionAgentDefaults(provider, model, effort) {
  return {
    provider,
    model: collaborationProviderModel(provider, model),
    effort,
  }
}

export function collaborationAgentSelectionEvidence(agents) {
  return agents.map((agent) => ({
    id: agent.id,
    ownerUserId: agent.owner_user_id,
    provider: agent.provider,
    model: agent.model ?? null,
    effort: agent.effort ?? null,
  }))
}

export function assertCollaborationAgentSelections(provider, model, effort, agents) {
  const expectedModel = collaborationProviderModel(provider, model)
  const selections = collaborationAgentSelectionEvidence(agents)
  const mismatch = selections.find((agent) => (
    agent.provider !== provider
      || !providerModelSelectionMatches(provider, expectedModel, agent.model)
      || agent.effort !== effort
  ))
  if (mismatch) {
    throw new Error(
      `collaboration agent selection mismatch: expected ${provider}/${expectedModel}/${effort}; received ${mismatch.provider}/${mismatch.model ?? "<none>"}/${mismatch.effort ?? "<none>"}`,
    )
  }
  return selections
}

function codexCliModel(model) {
  if (model.endsWith("-codex")) return model
  if (/^gpt-5\.[23]$/.test(model)) return `${model}-codex`
  return model
}
