import { randomUUID } from "node:crypto"
import { readFile } from "node:fs/promises"

import {
  adoptLegacyDeploymentProject,
  changeDeploymentEnvironmentLifecycle,
  createDeploymentProject,
  createDeploymentRelease,
  getDeploymentProject,
  listDeploymentProjects,
  promoteDeploymentRelease,
  rollbackDeploymentEnvironment,
} from "./deployed-workflow-api.js"
import { preparePublicationReleasePackage } from "./deployed-workflow-package.js"
import type {
  DeployedWorkflowProjectState,
  DeploymentPortfolioItem,
  DeploymentEnvironmentSummary,
  DeploymentProjectKind,
  PublicationDeploymentMode,
  PublicationReleaseSummary,
  ReleasePromotionResult,
} from "./deployed-workflow-types.js"
import { loadPreferences, relayCloudProfile, type RelayCloudProfile } from "./preferences.js"

export interface DeployedWorkflowCommandOutput {
  readonly notice: string
  readonly footer: string
}

export async function runDeployedWorkflowCommand(argv: readonly string[]): Promise<boolean> {
  if (argv[0] !== "deployments" && argv[0] !== "deployed") return false
  const profile = relayCloudProfile(await loadPreferences())
  if (!profile) throw new Error("cloud is not linked. Run /cloud link from the TUI first.")
  const result = await executeDeployedWorkflowCommand(profile, argv.slice(1))
  process.stdout.write(`${result.notice}\n`)
  return true
}

export async function handleDeployedWorkflowCloudCommand(
  deps: {
    readonly appendCloudNotice?: (message: string) => void
    readonly appendNotice: (message: string) => void
    readonly flashFooter: (message: string, tone: "info" | "error") => void
  },
  profile: RelayCloudProfile,
  area: string,
  action: string | undefined,
  args: readonly string[],
): Promise<boolean> {
  if (area !== "deployments" && area !== "deployed") return false
  const result = await executeDeployedWorkflowCommand(profile, [action ?? "list", ...args])
  ;(deps.appendCloudNotice ?? deps.appendNotice)(result.notice)
  deps.flashFooter(result.footer, "info")
  return true
}

export async function executeDeployedWorkflowCommand(
  profile: RelayCloudProfile,
  argv: readonly string[],
): Promise<DeployedWorkflowCommandOutput> {
  const action = argv[0] ?? "list"
  if (action === "list" || action === "ls") {
    const listed = await listDeploymentProjects(profile)
    return {
      notice: listed.portfolio.length > 0
        ? listed.portfolio.map(formatDeploymentPortfolioItem).join("\n")
        : "No deployed workflows.",
      footer: `${listed.portfolio.length} deployed workflow${listed.portfolio.length === 1 ? "" : "s"}`,
    }
  }
  if (action === "show" || action === "get") {
    const projectId = requiredArg(argv[1], "usage: deployments show <project-id>")
    const result = await getDeploymentProject(profile, projectId)
    return { notice: formatDeploymentProjectState(result.state), footer: `deployment ${result.state.project.slug}` }
  }
  if (action === "create") {
    const name = requiredArg(argv[1], createUsage)
    const options = parseCreateOptions(argv.slice(2))
    const result = await createDeploymentProject(profile, { name, ...options })
    return { notice: formatDeploymentProjectState(result.state), footer: `created deployment ${result.state.project.slug}` }
  }
  if (action === "adopt") {
    const deploymentId = requiredArg(argv[1], "usage: deployments adopt <legacy-deployment-id>")
    const result = await adoptLegacyDeploymentProject(profile, deploymentId)
    return { notice: formatDeploymentProjectState(result.state), footer: `adopted deployment ${result.state.project.slug}` }
  }
  if (action === "preflight") {
    const packagePath = requiredArg(argv[1], "usage: deployments preflight <package-dir|publication.json>")
    const prepared = await preparePublicationReleasePackage(packagePath)
    return {
      notice: [
        "release preflight passed",
        `package_id=${prepared.packageId}`,
        `package_digest=${prepared.packageDigest}`,
        `package_version=${prepared.packageVersion}`,
        `contract_version=${prepared.contractVersion}`,
        `credential_slots=${prepared.contract.credential_slots.length}`,
        `artifact_bytes=${prepared.artifact.byteSize}`,
        `artifact_sha256=${prepared.artifact.sha256}`,
      ].join("\n"),
      footer: "release preflight passed",
    }
  }
  if (action === "release") {
    const projectId = requiredArg(argv[1], "usage: deployments release <project-id> <package-dir|publication.json>")
    const packagePath = requiredArg(argv[2], "usage: deployments release <project-id> <package-dir|publication.json>")
    const result = await createDeploymentRelease(profile, projectId, packagePath)
    return { notice: formatRelease(result.release), footer: `release #${result.release.sequence} verified` }
  }
  if (action === "promote") {
    const projectId = requiredArg(argv[1], promoteUsage)
    const environmentId = requiredArg(argv[2], promoteUsage)
    const releaseId = requiredArg(argv[3], promoteUsage)
    const options = await parsePromotionOptions(argv.slice(4))
    const result = await promoteDeploymentRelease(profile, {
      projectId,
      environmentId,
      releaseId,
      idempotencyKey: options.idempotencyKey ?? randomUUID(),
      ...(options.configuration ? { configuration: options.configuration } : {}),
      ...(options.limits ? { limits: options.limits } : {}),
    })
    return formatPromotionOutput(result, "promotion")
  }
  if (action === "rollback") {
    const projectId = requiredArg(argv[1], rollbackUsage)
    const environmentId = requiredArg(argv[2], rollbackUsage)
    const promotionId = requiredArg(argv[3], rollbackUsage)
    const idempotencyKey = parseIdempotencyKey(argv.slice(4), rollbackUsage) ?? randomUUID()
    const result = await rollbackDeploymentEnvironment(profile, {
      projectId,
      environmentId,
      promotionId,
      idempotencyKey,
    })
    return formatPromotionOutput(result, "rollback")
  }
  if (action === "start" || action === "stop" || action === "restart") {
    const usage = lifecycleUsage(action)
    const projectId = requiredArg(argv[1], usage)
    const environmentId = requiredArg(argv[2], usage)
    const idempotencyKey = parseIdempotencyKey(argv.slice(3), usage) ?? randomUUID()
    const result = await changeDeploymentEnvironmentLifecycle(profile, {
      projectId,
      environmentId,
      action,
      idempotencyKey,
    })
    return formatLifecycleOutput(result.environment, action)
  }
  throw new Error(deploymentsUsage)
}

export function formatDeploymentPortfolioItem(item: DeploymentPortfolioItem): string {
  const environment = item.defaultEnvironment
  const release = item.latestRelease
  return [
    item.project.id,
    item.project.name,
    item.project.kind,
    environment?.slug ?? "no_environment",
    environment?.observedState ?? "setup",
    release ? `release=#${release.sequence}:${release.status}` : "release=none",
    environment ? `revision=${environment.observedRevision}/${environment.desiredRevision}` : "revision=none",
    environment?.publicUrl ?? "url=pending",
    item.needsAttention ? "attention=required" : "attention=none",
  ].join("\t")
}

export function formatDeploymentProjectState(state: DeployedWorkflowProjectState): string {
  return [
    `deployment ${state.project.id}`,
    `name ${state.project.name}`,
    `slug ${state.project.slug}`,
    `kind ${state.project.kind}`,
    `origin ${state.project.origin}`,
    ...state.environments.flatMap((environment) => [
      `environment ${environment.id} ${environment.slug} ${environment.tier}`,
      `  runtime ${environment.runtimeMode}${environment.region ? ` ${environment.region}` : ""}`,
      `  state desired=${environment.desiredState} observed=${environment.observedState}`,
      `  release desired=${environment.desiredReleaseId ?? "none"} observed=${environment.observedReleaseId ?? "none"}`,
      `  revision ${environment.observedRevision}/${environment.desiredRevision}`,
      `  url ${environment.publicUrl ?? "pending"}`,
      ...(environment.lastError ? [`  error ${environment.lastError}`] : []),
    ]),
    ...state.releases.map(formatRelease),
    ...state.promotions.map((promotion) => [
      `promotion ${promotion.id} ${promotion.status}`,
      `  release ${promotion.fromReleaseId ?? "none"} -> ${promotion.toReleaseId}`,
      `  revision ${promotion.desiredRevision}`,
      ...(promotion.errorMessage ? [`  error ${promotion.errorMessage}`] : []),
    ].join("\n")),
  ].join("\n")
}

function formatRelease(release: PublicationReleaseSummary): string {
  return [
    `release ${release.id} #${release.sequence} ${release.status}`,
    `  package_id ${release.packageId ?? "legacy"}`,
    `  package_digest ${release.packageDigest}`,
    `  package_version ${release.packageVersion}`,
    `  verified_at ${release.verifiedAt ?? "not_verified"}`,
    ...(release.rejectionReason ? [`  rejection ${release.rejectionReason}`] : []),
  ].join("\n")
}

function formatPromotionOutput(
  result: ReleasePromotionResult,
  action: "promotion" | "rollback",
): DeployedWorkflowCommandOutput {
  return {
    notice: [
      `${action} ${result.promotion.id} ${result.promotion.status}`,
      `environment ${result.environment.id} ${result.environment.slug}`,
      `state desired=${result.environment.desiredState} observed=${result.environment.observedState}`,
      `release desired=${result.environment.desiredReleaseId ?? "none"} observed=${result.environment.observedReleaseId ?? "none"}`,
      `revision ${result.environment.observedRevision}/${result.environment.desiredRevision}`,
      `url ${result.environment.publicUrl ?? "pending"}`,
    ].join("\n"),
    footer: `${action} requested for ${result.environment.slug}`,
  }
}

function formatLifecycleOutput(
  environment: DeploymentEnvironmentSummary,
  action: "start" | "stop" | "restart",
): DeployedWorkflowCommandOutput {
  return {
    notice: [
      `${action} requested for ${environment.slug}`,
      `state desired=${environment.desiredState} observed=${environment.observedState}`,
      `release desired=${environment.desiredReleaseId ?? "none"} observed=${environment.observedReleaseId ?? "none"}`,
      `revision ${environment.observedRevision}/${environment.desiredRevision}`,
      `url ${environment.publicUrl ?? "pending"}`,
    ].join("\n"),
    footer: `${action} requested for ${environment.slug}`,
  }
}

const createUsage = "usage: deployments create <name> [--kind workflow-endpoint|agent-app] [--mode local-runtime|hosted-container] [--slug value] [--region value]"
const promoteUsage = "usage: deployments promote <project-id> <environment-id> <release-id> [--configuration json-file] [--limits json-file] [--idempotency-key value]"
const rollbackUsage = "usage: deployments rollback <project-id> <environment-id> <promotion-id> [--idempotency-key value]"
const deploymentsUsage = "usage: deployments list | show <project-id> | create <name> | adopt <legacy-id> | preflight <package> | release <project-id> <package> | promote <project-id> <environment-id> <release-id> | rollback <project-id> <environment-id> <promotion-id> | start|stop|restart <project-id> <environment-id>"

function lifecycleUsage(action: "start" | "stop" | "restart"): string {
  return `usage: deployments ${action} <project-id> <environment-id> [--idempotency-key value]`
}

function parseCreateOptions(argv: readonly string[]): {
  readonly kind: DeploymentProjectKind
  readonly defaultRuntimeMode: PublicationDeploymentMode
  readonly slug?: string
  readonly defaultRegion?: string
} {
  let kind: DeploymentProjectKind = "workflow_endpoint"
  let defaultRuntimeMode: PublicationDeploymentMode = "hosted_container"
  let slug: string | undefined
  let defaultRegion: string | undefined
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    const value = argv[index + 1]
    if (option === "--kind") {
      if (value === "workflow-endpoint" || value === "workflow_endpoint") kind = "workflow_endpoint"
      else if (value === "agent-app" || value === "agent_app") kind = "agent_app"
      else throw new Error("--kind must be workflow-endpoint or agent-app")
      index += 1
    } else if (option === "--mode") {
      if (value === "local-runtime" || value === "local_runtime") defaultRuntimeMode = "local_runtime"
      else if (value === "hosted-container" || value === "hosted_container") defaultRuntimeMode = "hosted_container"
      else throw new Error("--mode must be local-runtime or hosted-container")
      index += 1
    } else if (option === "--slug") {
      slug = requiredArg(value, createUsage)
      index += 1
    } else if (option === "--region") {
      defaultRegion = requiredArg(value, createUsage)
      index += 1
    } else {
      throw new Error(`unknown deployments create option ${option}`)
    }
  }
  return {
    kind,
    defaultRuntimeMode,
    ...(slug ? { slug } : {}),
    ...(defaultRegion ? { defaultRegion } : {}),
  }
}

async function parsePromotionOptions(argv: readonly string[]): Promise<{
  readonly configuration?: Record<string, unknown>
  readonly limits?: Record<string, unknown>
  readonly idempotencyKey?: string
}> {
  let configuration: Record<string, unknown> | undefined
  let limits: Record<string, unknown> | undefined
  let idempotencyKey: string | undefined
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    const value = requiredArg(argv[index + 1], promoteUsage)
    if (option === "--configuration") configuration = await readJsonObject(value, "configuration")
    else if (option === "--limits") limits = await readJsonObject(value, "limits")
    else if (option === "--idempotency-key") idempotencyKey = value
    else throw new Error(`unknown deployments promote option ${option}`)
    index += 1
  }
  return {
    ...(configuration ? { configuration } : {}),
    ...(limits ? { limits } : {}),
    ...(idempotencyKey ? { idempotencyKey } : {}),
  }
}

function parseIdempotencyKey(argv: readonly string[], usage: string): string | undefined {
  if (argv.length === 0) return undefined
  if (argv.length === 2 && argv[0] === "--idempotency-key") {
    return requiredArg(argv[1], usage)
  }
  throw new Error(usage)
}

async function readJsonObject(path: string, label: string): Promise<Record<string, unknown>> {
  const value = JSON.parse(await readFile(path, "utf8")) as unknown
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} file must contain a JSON object`)
  }
  return value as Record<string, unknown>
}

function requiredArg(value: string | undefined, usage: string): string {
  if (!value?.trim()) throw new Error(usage)
  return value.trim()
}
