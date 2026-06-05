import { readFile, stat, writeFile } from "node:fs/promises"
import { basename, dirname, resolve as resolvePath } from "node:path"

import type { WorkflowPublicationSnapshot } from "./kernel-types.js"

export type PublicationProviderModelProfile = {
  provider: string
  model?: string | null
  effort?: string | null
}

export type PublicationProviderModelOverride = {
  agent_id: string
  node_ids?: string[]
  captured: PublicationProviderModelProfile
  replacement?: PublicationProviderModelProfile | null
}

export type WorkflowPublicationBindings = {
  schema_version: number
  provider_model_overrides?: PublicationProviderModelOverride[]
}

export type WorkflowPublicationBindingsPackage = {
  root: string
  publicationJsonPath: string
  snapshotPath: string
  bindingsPath: string
  snapshot: WorkflowPublicationSnapshot
  bindings: WorkflowPublicationBindings
  bindingsExisted: boolean
}

export async function loadWorkflowPublicationBindingsPackage(
  packagePath: string,
): Promise<WorkflowPublicationBindingsPackage> {
  const target = resolvePath(packagePath)
  const targetStat = await stat(target)
  const publicationJsonPath = targetStat.isFile() ? target : resolvePath(target, "publication.json")
  const root = targetStat.isFile() ? dirname(target) : target
  const publicationPackage = parseJsonObject(await readFile(publicationJsonPath, "utf8"), publicationJsonPath)
  const bindingsName = typeof publicationPackage.default_bindings_path === "string" && publicationPackage.default_bindings_path.trim()
    ? publicationPackage.default_bindings_path
    : "bindings.local.json"
  const snapshotPath = resolvePath(root, "workflow.snapshot.json")
  const bindingsPath = resolvePath(root, bindingsName)
  const snapshot = parseJsonObject(await readFile(snapshotPath, "utf8"), snapshotPath) as WorkflowPublicationSnapshot
  const loaded = await readBindingsOrDefault(bindingsPath, snapshot)
  return {
    root,
    publicationJsonPath,
    snapshotPath,
    bindingsPath,
    snapshot,
    bindings: loaded.bindings,
    bindingsExisted: loaded.existed,
  }
}

export function formatWorkflowPublicationBindings(packageState: WorkflowPublicationBindingsPackage) {
  const overrides = packageState.bindings.provider_model_overrides ?? []
  const lines = [
    `publication bindings ${displayPath(packageState.bindingsPath)}`,
    `package ${displayPath(packageState.root)}`,
  ]
  if (overrides.length === 0) {
    lines.push("no provider/model bindings")
    return lines.join("\n")
  }
  for (const override of overrides) {
    const nodes = override.node_ids?.length ? override.node_ids.join(",") : "-"
    lines.push(`${override.agent_id} nodes=${nodes} captured=${profileLabel(override.captured)} replacement=${override.replacement ? profileLabel(override.replacement) : "default"}`)
  }
  if (!packageState.bindingsExisted) {
    lines.push("local bindings file has not been created yet")
  }
  return lines.join("\n")
}

export async function setWorkflowPublicationBinding(
  packagePath: string,
  agentId: string,
  replacement: PublicationProviderModelProfile,
) {
  const packageState = await loadWorkflowPublicationBindingsPackage(packagePath)
  const binding = bindingForAgent(packageState.bindings, packageState.snapshot, agentId)
  binding.replacement = normalizeProfile(replacement)
  await saveWorkflowPublicationBindings(packageState)
  return packageState
}

export async function clearWorkflowPublicationBinding(packagePath: string, agentId: string) {
  const packageState = await loadWorkflowPublicationBindingsPackage(packagePath)
  const binding = bindingForAgent(packageState.bindings, packageState.snapshot, agentId)
  binding.replacement = null
  await saveWorkflowPublicationBindings(packageState)
  return packageState
}

function parseJsonObject(content: string, filePath: string): Record<string, unknown> {
  const parsed = JSON.parse(content)
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(`${basename(filePath)} must contain a JSON object`)
  }
  return parsed as Record<string, unknown>
}

async function readBindingsOrDefault(
  bindingsPath: string,
  snapshot: WorkflowPublicationSnapshot,
): Promise<{ bindings: WorkflowPublicationBindings; existed: boolean }> {
  try {
    const bindings = parseJsonObject(await readFile(bindingsPath, "utf8"), bindingsPath) as WorkflowPublicationBindings
    if (bindings.schema_version !== 1) {
      throw new Error(`unsupported publication bindings schema_version ${bindings.schema_version}`)
    }
    return { bindings, existed: true }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error
  }
  return { bindings: defaultWorkflowPublicationBindings(snapshot), existed: false }
}

function defaultWorkflowPublicationBindings(snapshot: WorkflowPublicationSnapshot): WorkflowPublicationBindings {
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
  agentId: string,
) {
  bindings.provider_model_overrides ??= []
  let binding = bindings.provider_model_overrides.find((candidate) => candidate.agent_id === agentId)
  if (binding) return binding
  const agent = (snapshot.agents ?? []).find((candidate) => candidate.id === agentId)
  if (!agent) {
    throw new Error(`agent ${agentId} was not found in workflow publication snapshot`)
  }
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
  return binding
}

function normalizeProfile(profile: PublicationProviderModelProfile): PublicationProviderModelProfile {
  return {
    provider: profile.provider,
    model: optionalValue(profile.model),
    effort: optionalValue(profile.effort),
  }
}

function optionalValue(value: string | null | undefined) {
  if (value == null) return null
  const trimmed = value.trim()
  if (!trimmed || trimmed === "-") return null
  return trimmed
}

async function saveWorkflowPublicationBindings(packageState: WorkflowPublicationBindingsPackage) {
  await writeFile(packageState.bindingsPath, `${JSON.stringify(packageState.bindings, null, 2)}\n`)
  packageState.bindingsExisted = true
}

function profileLabel(profile: PublicationProviderModelProfile) {
  const model = profile.model ? `/${profile.model}` : ""
  const effort = profile.effort ? ` effort=${profile.effort}` : ""
  return `${profile.provider}${model}${effort}`
}

function displayPath(filePath: string) {
  return filePath
}
