const CLAUDE_MODEL_ALIASES = new Set(["haiku", "opus", "sonnet"])

export function providerModelSelectionMatches(provider, requestedModel, observedModel) {
  if (provider !== "claude" && provider !== "claude-headless") {
    return observedModel === requestedModel
  }
  const requested = claudeModelRef(requestedModel)
  const observed = claudeModelRef(observedModel)
  if (requested === null || observed === null) return requested === observed
  if (requested === observed) return true
  return CLAUDE_MODEL_ALIASES.has(requested)
    && observed.startsWith(`claude-${requested}-`)
}

function claudeModelRef(model) {
  if (typeof model !== "string") return null
  return model.trim().split("/").filter(Boolean).at(-1) ?? null
}
