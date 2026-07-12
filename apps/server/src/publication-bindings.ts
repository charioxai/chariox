import { readFile, writeFile } from "node:fs/promises"
import { createInterface } from "node:readline/promises"
import process from "node:process"

import { getProviderCatalogRequest } from "@arroba/kernel-client/ipc-requests"

import type {
  KernelLookupClient,
  PublicationProviderModelProfile,
  WorkflowPublicationBindings,
  WorkflowPublicationSnapshot,
} from "./publication-types.js"

export type ProviderModelBindingPrompt = (request: {
  agent_id: string
  captured: PublicationProviderModelProfile
  available: ProviderCatalogIndex
}) => Promise<PublicationProviderModelProfile>

export async function resolvePublicationProviderModelBindings(
  snapshot: WorkflowPublicationSnapshot,
  bindingsPath: string,
  client: KernelLookupClient,
  options: {
    promptReplacement?: ProviderModelBindingPrompt | false
  } = {},
) {
  const catalog = await providerCatalogIndex(client)
  const bindings = await loadPublicationBindings(bindingsPath, snapshot)
  let changed = false
  for (const agent of snapshot.agents ?? []) {
    const binding = bindingForAgent(bindings, snapshot, agent)
    const selected = binding.replacement ?? binding.captured
    const selectedProfile = availableProviderProfile(catalog, selected)
    if (selectedProfile) {
      applyAgentProfile(agent, selectedProfile)
      continue
    }
    const promptReplacement = options.promptReplacement ?? promptProviderModelReplacement
    if (promptReplacement === false) {
      throw new Error(`publication provider/model is unavailable for agent ${agent.id}: ${profileLabel(selected)}`)
    }
    const replacement = await promptReplacement({
      agent_id: agent.id,
      captured: binding.captured,
      available: catalog,
    })
    const replacementProfile = availableProviderProfile(catalog, replacement)
    if (!replacementProfile) {
      throw new Error(`publication provider/model replacement is unavailable for agent ${agent.id}: ${profileLabel(replacement)}`)
    }
    binding.replacement = replacementProfile
    applyAgentProfile(agent, replacementProfile)
    changed = true
  }
  if (changed) {
    await writeFile(bindingsPath, `${JSON.stringify(bindings, null, 2)}\n`)
  }
  return { snapshot, bindings, changed }
}

export type ProviderCatalogIndex = {
  providers: Map<string, Set<string>>
}

async function providerCatalogIndex(client: KernelLookupClient): Promise<ProviderCatalogIndex> {
  const response = await client.send(getProviderCatalogRequest())
  const catalog = (response.ProviderCatalog as { catalog?: { all?: unknown[] } } | undefined)?.catalog
  const providers = new Map<string, Set<string>>()
  for (const provider of catalog?.all ?? []) {
    if (!provider || typeof provider !== "object" || Array.isArray(provider)) continue
    const record = provider as Record<string, unknown>
    if (typeof record.id !== "string" || !record.id.trim()) continue
    const models = new Set<string>()
    if (record.models && typeof record.models === "object" && !Array.isArray(record.models)) {
      for (const modelId of Object.keys(record.models)) {
        if (modelId.trim()) models.add(modelId)
      }
    }
    providers.set(record.id, models)
  }
  return { providers }
}

async function loadPublicationBindings(
  bindingsPath: string,
  snapshot: WorkflowPublicationSnapshot,
): Promise<WorkflowPublicationBindings> {
  try {
    const bindings = JSON.parse(await readFile(bindingsPath, "utf8")) as WorkflowPublicationBindings
    if (bindings.schema_version !== 1) {
      throw new Error(`unsupported publication bindings schema_version ${bindings.schema_version}`)
    }
    return bindings
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error
  }
  return {
    schema_version: 1,
    provider_model_overrides: (snapshot.agents ?? []).map((agent) => ({
      agent_id: agent.id,
      node_ids: (snapshot.workflow.nodes ?? [])
        .filter((node) => node.agent_id === agent.id)
        .map((node) => node.id),
      captured: {
        provider: agent.provider,
        model: agent.model ?? null,
        effort: agent.effort ?? null,
      },
      replacement: null,
    })),
  }
}

function bindingForAgent(
  bindings: WorkflowPublicationBindings,
  snapshot: WorkflowPublicationSnapshot,
  agent: NonNullable<WorkflowPublicationSnapshot["agents"]>[number],
) {
  bindings.provider_model_overrides ??= []
  let binding = bindings.provider_model_overrides.find((candidate) => candidate.agent_id === agent.id)
  if (!binding) {
    binding = {
      agent_id: agent.id,
      node_ids: (snapshot.workflow.nodes ?? [])
        .filter((node) => node.agent_id === agent.id)
        .map((node) => node.id),
      captured: {
        provider: agent.provider,
        model: agent.model ?? null,
        effort: agent.effort ?? null,
      },
      replacement: null,
    }
    bindings.provider_model_overrides.push(binding)
  }
  return binding
}

function availableProviderProfile(catalog: ProviderCatalogIndex, profile: PublicationProviderModelProfile): PublicationProviderModelProfile | null {
  const models = catalog.providers.get(profile.provider)
  if (!models) return null
  if (profile.model === "default" || profile.model === `${profile.provider}/default`) {
    return { ...profile, model: null }
  }
  if (!profile.model || models.size === 0 || models.has(profile.model)) return profile
  const providerPrefixedModel = `${profile.provider}/`
  if (profile.model.startsWith(providerPrefixedModel)) {
    const unprefixedModel = profile.model.slice(providerPrefixedModel.length)
    if (models.has(unprefixedModel)) {
      return { ...profile, model: unprefixedModel }
    }
  }
  return null
}

function applyAgentProfile(agent: NonNullable<WorkflowPublicationSnapshot["agents"]>[number], profile: PublicationProviderModelProfile) {
  agent.provider = profile.provider
  agent.model = profile.model ?? null
  agent.effort = profile.effort ?? null
}

async function promptProviderModelReplacement({
  agent_id,
  captured,
  available,
}: {
  agent_id: string
  captured: PublicationProviderModelProfile
  available: ProviderCatalogIndex
}) {
  if (!process.stdin.isTTY) {
    throw new Error(`publication provider/model is unavailable for agent ${agent_id}: ${profileLabel(captured)}`)
  }
  const choices = [...available.providers.entries()]
    .flatMap(([provider, models]) => {
      if (models.size === 0) return [provider]
      return [...models].map((model) => `${provider}/${model}`)
    })
    .join(", ")
  const readline = createInterface({ input: process.stdin, output: process.stderr })
  try {
    process.stderr.write(`Captured provider/model for published workflow agent ${agent_id} is unavailable: ${profileLabel(captured)}\n`)
    process.stderr.write(`Available provider/model choices: ${choices || "(none)"}\n`)
    const provider = (await readline.question("Replacement provider: ")).trim()
    const model = (await readline.question("Replacement model (blank for provider default): ")).trim()
    const effort = (await readline.question("Replacement effort (blank to keep unset): ")).trim()
    return {
      provider,
      model: model || null,
      effort: effort || null,
    }
  } finally {
    readline.close()
  }
}

function profileLabel(profile: PublicationProviderModelProfile) {
  const model = profile.model ? `/${profile.model}` : ""
  const effort = profile.effort ? ` effort=${profile.effort}` : ""
  return `${profile.provider}${model}${effort}`
}
