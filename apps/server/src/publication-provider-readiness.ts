import { access, readFile } from "node:fs/promises"
import { join } from "node:path"
import { execFile } from "node:child_process"
import { promisify } from "node:util"
import process from "node:process"

import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import { getProviderAuthStatusRequest } from "@arroba/kernel-client/ipc-requests"

import { defaultKernelEndpoint } from "./kernel-publication-client.js"
import type {
  GatewayDeps,
  PublicationPackageMaterializationStatus,
  PublicationProviderReadiness,
  WorkflowPublicationConfig,
  WorkflowPublicationSnapshot,
} from "./publication-types.js"

const execFileAsync = promisify(execFile)

export async function publicationHealthDetails(
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
): Promise<{
  package: PublicationPackageMaterializationStatus
  provider_readiness: readonly PublicationProviderReadiness[]
}> {
  return {
    package: await packageMaterializationStatus(publication),
    provider_readiness: await publicationProviderReadiness(publication, deps),
  }
}

export async function packageMaterializationStatus(
  publication: WorkflowPublicationConfig,
): Promise<PublicationPackageMaterializationStatus> {
  const packageRoot = publication.package_root ?? null
  if (!packageRoot) {
    return { materialized: true, package_root: null, missing_files: [] }
  }
  const required = ["publication.json", "workflow.snapshot.json", "requirements.json"]
  const missing: string[] = []
  for (const file of required) {
    if (!await fileExists(join(packageRoot, file))) {
      missing.push(file)
    }
  }
  return { materialized: missing.length === 0, package_root: packageRoot, missing_files: missing }
}

export async function publicationProviderReadiness(
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
): Promise<readonly PublicationProviderReadiness[]> {
  if (deps.getProviderReadiness) {
    return deps.getProviderReadiness(publication)
  }
  const providers = await requiredPublicationProviders(publication)
  if (providers.length === 0) {
    return []
  }
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const readiness: PublicationProviderReadiness[] = []
    for (const provider of providers) {
      readiness.push(await providerReadiness(provider, client))
    }
    return readiness
  } finally {
    await client.close().catch(() => {})
  }
}

async function requiredPublicationProviders(publication: WorkflowPublicationConfig): Promise<string[]> {
  if (!publication.package_root) {
    return []
  }
  const snapshot = JSON.parse(await readFile(join(publication.package_root, "workflow.snapshot.json"), "utf8")) as WorkflowPublicationSnapshot
  const providers = new Set<string>()
  for (const agent of snapshot.agents ?? []) {
    const provider = normalizeProvider(agent.provider)
    if (provider) providers.add(provider)
  }
  return [...providers].sort()
}

async function providerReadiness(provider: string, client: LocalIpcClient): Promise<PublicationProviderReadiness> {
  const command = providerCommand(provider)
  const cli = await providerCliStatus(command)
  if (!cli.available) {
    return {
      provider,
      status: "provider_cli_missing",
      ready: false,
      cli,
      auth: { status: "provider_auth_unknown" },
      error: `${provider} CLI was not found`,
    }
  }
  const auth = await providerAuthStatus(provider, client)
  const status = auth.status === "provider_auth_expired"
    ? "provider_auth_expired"
    : "provider_ready"
  return {
    provider,
    status,
    ready: status === "provider_ready",
    cli,
    auth,
    ...(auth.status === "provider_auth_expired" ? { error: `${provider} authentication is expired or missing` } : {}),
  }
}

async function providerCliStatus(command: string): Promise<PublicationProviderReadiness["cli"]> {
  try {
    const result = await execFileAsync(command, ["--version"], { timeout: 5_000, maxBuffer: 64 * 1024 })
    const version = `${result.stdout}${result.stderr}`.trim().split(/\r?\n/)[0] ?? null
    return { available: true, command, version: version || null }
  } catch {
    return { available: false, command, version: null }
  }
}

async function providerAuthStatus(
  provider: string,
  client: LocalIpcClient,
): Promise<PublicationProviderReadiness["auth"]> {
  try {
    const response = await client.send<Record<string, unknown>>(getProviderAuthStatusRequest(provider))
    const status = (response.ProviderAuthStatus as { status?: Record<string, unknown> } | undefined)?.status
    const authState = typeof status?.auth_state === "string" ? status.auth_state : "unknown"
    if (authState === "authenticated") {
      return {
        status: "provider_ready",
        account_profile: typeof status?.account_profile === "string" ? status.account_profile : null,
      }
    }
    if (authState === "not_logged_in" || authState === "expired") {
      return { status: "provider_auth_expired" }
    }
    return { status: "provider_auth_unknown" }
  } catch {
    return { status: "provider_auth_unknown" }
  }
}

function providerCommand(provider: string): string {
  if (provider === "codex") return process.env.ARROBA_CODEX_BIN || "codex"
  if (provider === "claude") return process.env.ARROBA_CLAUDE_BIN || "claude"
  if (provider === "opencode") return process.env.ARROBA_OPENCODE_BIN || "opencode"
  return provider
}

function normalizeProvider(provider: unknown): string | null {
  if (typeof provider !== "string") return null
  const trimmed = provider.trim().toLowerCase()
  if (!trimmed) return null
  return trimmed.split(":")[0] ?? trimmed
}

async function fileExists(path: string): Promise<boolean> {
  try {
    await access(path)
    return true
  } catch {
    return false
  }
}
