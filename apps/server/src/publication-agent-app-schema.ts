import { existsSync, statSync } from "node:fs"
import { join, normalize, sep } from "node:path"

import type {
  AgentAppConfig,
  AgentAppRouteConfig,
} from "./publication-types.js"

const supportedManipulationLevels = new Set([
  "none",
  "state",
  "overlay",
  "state_and_overlay",
  "full_ephemeral",
  "persistent_patch",
])
const supportedManipulationScopes = new Set(["invocation", "session", "persistent"])
const supportedActionTransports = new Set(["http"])
const supportedActionMethods = new Set(["GET", "POST"])

export function validateAgentAppConfig(
  agentApp: AgentAppConfig | undefined,
  options: { readonly packageRoot?: string | undefined } = {},
): void {
  if (agentApp?.enabled !== true) return
  validateAssets(agentApp, options.packageRoot)
  const actionIds = new Set(Object.keys(agentApp.actions ?? {}))
  for (const [actionId, action] of Object.entries(agentApp.actions ?? {})) {
    if (!isSafeIdentifier(actionId)) throw new Error(`Agent App action id ${actionId} is invalid`)
    const transportKind = action.transport?.kind ?? "http"
    if (!supportedActionTransports.has(transportKind)) {
      throw new Error(`Agent App action ${actionId} uses unsupported transport ${transportKind}`)
    }
    const method = action.transport?.method ?? "POST"
    if (!supportedActionMethods.has(method)) {
      throw new Error(`Agent App action ${actionId} uses unsupported method ${method}`)
    }
    if (action.transport?.url !== undefined) validateActionUrl(actionId, action.transport.url)
  }
  for (const route of agentApp.routes ?? []) validateRoute(route, actionIds)
  validateReplicas(agentApp.replicas?.count)
}

function validateAssets(agentApp: AgentAppConfig, packageRoot: string | undefined): void {
  const publicDir = agentApp.assets?.public_dir
  if (typeof publicDir !== "string" || !publicDir.trim()) {
    throw new Error("Agent App package is missing assets.public_dir")
  }
  if (publicDir.includes("\0") || normalize(publicDir).startsWith("..")) {
    throw new Error("Agent App assets.public_dir must stay inside the package")
  }
  if (!packageRoot) return
  const assetDir = join(packageRoot, publicDir)
  if (!existsSync(assetDir) || !statSync(assetDir).isDirectory()) {
    throw new Error(`Agent App assets.public_dir does not exist: ${publicDir}`)
  }
}

function validateRoute(route: AgentAppRouteConfig, actionIds: Set<string>): void {
  if (typeof route.path !== "string" || !route.path.startsWith("/")) {
    throw new Error("Agent App route path must start with /")
  }
  if (route.path.includes("\0") || route.path.includes("://") || route.path.includes("..")) {
    throw new Error(`Agent App route path ${route.path} is invalid`)
  }
  if (route.path.includes("*") && !route.path.endsWith("/*")) {
    throw new Error(`Agent App route path ${route.path} may only use a trailing /* wildcard`)
  }
  if (route.prompt_source !== undefined && route.prompt_source !== "path_tail") {
    throw new Error(`Agent App route ${route.path} uses unsupported prompt_source ${route.prompt_source}`)
  }
  if (route.response !== undefined && route.response !== "streaming_shell") {
    throw new Error(`Agent App route ${route.path} uses unsupported response ${route.response}`)
  }
  const manipulation = route.manipulation
  if (!manipulation) return
  const level = manipulation.level ?? "none"
  if (!supportedManipulationLevels.has(level)) {
    throw new Error(`Agent App route ${route.path} uses unsupported manipulation level ${level}`)
  }
  const scope = manipulation.scope ?? "session"
  if (!supportedManipulationScopes.has(scope)) {
    throw new Error(`Agent App route ${route.path} uses unsupported manipulation scope ${scope}`)
  }
  for (const path of [...(manipulation.allowed_paths ?? []), ...(manipulation.protected_paths ?? [])]) {
    validateAppPathPattern(route.path, path)
  }
  for (const actionId of manipulation.allowed_actions ?? []) {
    if (!actionIds.has(actionId)) {
      throw new Error(`Agent App route ${route.path} references unknown action ${actionId}`)
    }
  }
}

function validateReplicas(count: unknown): void {
  if (count === undefined) return
  if (typeof count !== "number" || !Number.isInteger(count) || count < 1 || count > 32) {
    throw new Error("Agent App replicas.count must be an integer from 1 to 32")
  }
}

function validateActionUrl(actionId: string, url: string): void {
  try {
    const parsed = new URL(url)
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      throw new Error("unsupported protocol")
    }
  } catch {
    throw new Error(`Agent App action ${actionId} has an invalid URL`)
  }
}

function validateAppPathPattern(routePath: string, path: string): void {
  const normalized = normalize(`/${path}`).replaceAll(sep, "/")
  if (!path.startsWith("/") || normalized.includes("\0") || normalized.includes("..")) {
    throw new Error(`Agent App route ${routePath} has invalid app path pattern ${path}`)
  }
}

function isSafeIdentifier(value: string): boolean {
  return /^[a-zA-Z0-9_.-]+$/.test(value)
}
