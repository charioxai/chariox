import { randomUUID } from "node:crypto"
import { readFile } from "node:fs/promises"

import {
  acceptDeploymentClaim,
  adoptLegacyDeploymentProject,
  changeDeploymentEnvironmentLifecycle,
  createDeploymentClaim,
  createDeploymentProject,
  createDeploymentRelease,
  getDeploymentAccess,
  getDeploymentProject,
  listDeploymentProjects,
  promoteDeploymentRelease,
  reviewDeploymentClaim,
  revokeDeploymentClaim,
  revokeDeploymentProjectMember,
  rollbackDeploymentEnvironment,
  upsertDeploymentProjectMember,
} from "./deployed-workflow-api.js"
import { preparePublicationReleasePackage } from "./deployed-workflow-package.js"
import type {
  DeployedWorkflowProjectState,
  DeploymentAccessState,
  DeploymentClaimSummary,
  DeploymentControlRole,
  DeploymentPortfolioItem,
  DeploymentEnvironmentSummary,
  DeploymentOwnershipMode,
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
  if (action === "claim") {
    const claimAction = argv[1]
    if (claimAction === "create") {
      const projectId = requiredArg(argv[2], claimCreateUsage)
      const releaseId = requiredArg(argv[3], claimCreateUsage)
      const options = parseClaimCreateOptions(argv.slice(4))
      const result = await createDeploymentClaim(profile, { projectId, releaseId, ...options })
      return {
        notice: `${formatDeploymentClaim(result.claim)}\nclaim_token ${result.claimToken}`,
        footer: "deployment claim created; token shown once",
      }
    }
    if (claimAction === "review") {
      const claimToken = requiredArg(argv[2], claimReviewUsage)
      const result = await reviewDeploymentClaim(profile, claimToken)
      return { notice: formatDeploymentClaim(result.claim), footer: `claim ${result.claim.status}` }
    }
    if (claimAction === "accept") {
      const claimToken = requiredArg(argv[2], claimAcceptUsage)
      const result = await acceptDeploymentClaim(profile, {
        claimToken,
        ...parseClaimAcceptOptions(argv.slice(3)),
      })
      return {
        notice: `${formatDeploymentClaim(result.claim)}\n${formatDeploymentProjectState(result.state)}`,
        footer: `claimed deployment ${result.state.project.slug}`,
      }
    }
    if (claimAction === "revoke") {
      const projectId = requiredArg(argv[2], claimRevokeUsage)
      const claimId = requiredArg(argv[3], claimRevokeUsage)
      const result = await revokeDeploymentClaim(profile, projectId, claimId)
      return { notice: formatDeploymentClaim(result.claim), footer: "deployment claim revoked" }
    }
    throw new Error(claimUsage)
  }
  if (action === "access") {
    const projectId = requiredArg(argv[1] === "show" ? argv[2] : argv[1], accessUsage)
    const result = await getDeploymentAccess(profile, projectId)
    return { notice: formatDeploymentAccess(result.access), footer: `deployment access ${projectId}` }
  }
  if (action === "member" || action === "members") {
    const memberAction = argv[1]
    if (memberAction === "add" || memberAction === "set") {
      const projectId = requiredArg(argv[2], memberAddUsage)
      const granteeAccountId = requiredArg(argv[3], memberAddUsage)
      const userEmail = requiredArg(argv[4], memberAddUsage)
      const role = parseMemberRole(requiredArg(argv[5], memberAddUsage))
      const result = await upsertDeploymentProjectMember(profile, {
        projectId,
        granteeAccountId,
        userEmail,
        role,
      })
      return { notice: formatDeploymentAccess(result.access), footer: `deployment member ${role}` }
    }
    if (memberAction === "revoke") {
      const projectId = requiredArg(argv[2], memberRevokeUsage)
      const memberId = requiredArg(argv[3], memberRevokeUsage)
      const result = await revokeDeploymentProjectMember(profile, projectId, memberId)
      return { notice: formatDeploymentAccess(result.access), footer: "deployment member revoked" }
    }
    throw new Error(memberUsage)
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
    `ownership=${item.project.ownershipMode ?? "internal_team"}`,
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
    `ownership ${state.project.ownershipMode ?? "internal_team"}`,
    `builder_account ${state.project.builderAccountId ?? "none"}`,
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

export function formatDeploymentClaim(claim: DeploymentClaimSummary): string {
  return [
    `claim ${claim.id} ${claim.status}`,
    `source ${claim.sourceProjectId} release=${claim.sourceReleaseId} sequence=${claim.sourceReleaseSequence}`,
    `package_digest ${claim.sourcePackageDigest}`,
    `ownership ${claim.ownershipMode}`,
    `builder_role ${claim.builderRole ?? "none"}`,
    `target_account ${claim.targetAccountId ?? "any"}`,
    `target_email ${claim.targetEmail ?? "any"}`,
    `token_prefix ${claim.tokenPrefix}`,
    `expires_at ${claim.expiresAt}`,
    `claimed_project ${claim.claimedProjectId ?? "none"}`,
  ].join("\n")
}

export function formatDeploymentAccess(access: DeploymentAccessState): string {
  return [
    `access ${access.projectId}`,
    `owner_account ${access.projectAccountId}`,
    `ownership ${access.ownershipMode}`,
    `builder_account ${access.builderAccountId ?? "none"}`,
    ...access.claims.map((claim) => [
      `claim ${claim.id} ${claim.status}`,
      `  source ${claim.sourceProjectId} release=${claim.sourceReleaseId}`,
      `  target account=${claim.targetAccountId ?? "any"} email=${claim.targetEmail ?? "any"}`,
      `  expires_at ${claim.expiresAt}`,
    ].join("\n")),
    ...access.members.map((member) => [
      `member ${member.id} ${member.status}`,
      `  user ${member.userEmail} account=${member.granteeAccountId}`,
      `  role ${member.role}`,
      `  origin_claim ${member.originClaimId ?? "none"}`,
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
const claimCreateUsage = "usage: deployments claim create <project-id> <release-id> [--ownership customer-owned|builder-managed|internal-team] [--builder-role maintainer|deployer|operator|viewer|none] [--target-account id] [--target-email email] [--expires-seconds value]"
const claimReviewUsage = "usage: deployments claim review <claim-token>"
const claimAcceptUsage = "usage: deployments claim accept <claim-token> [--name value] [--slug value] [--mode local-runtime|hosted-container]"
const claimRevokeUsage = "usage: deployments claim revoke <project-id> <claim-id>"
const claimUsage = "usage: deployments claim create|review|accept|revoke ..."
const accessUsage = "usage: deployments access [show] <project-id>"
const memberAddUsage = "usage: deployments member add <project-id> <grantee-account-id> <user-email> <admin|deployer|operator|viewer|billing|maintainer>"
const memberRevokeUsage = "usage: deployments member revoke <project-id> <member-id>"
const memberUsage = "usage: deployments member add|revoke ..."
const deploymentsUsage = "usage: deployments list | show <project-id> | create <name> | adopt <legacy-id> | preflight <package> | release <project-id> <package> | promote <project-id> <environment-id> <release-id> | rollback <project-id> <environment-id> <promotion-id> | start|stop|restart <project-id> <environment-id> | claim create|review|accept|revoke ... | access <project-id> | member add|revoke ..."

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

function parseClaimCreateOptions(argv: readonly string[]): {
  readonly ownershipMode: DeploymentOwnershipMode
  readonly builderRole?: DeploymentControlRole | null
  readonly targetAccountId?: string
  readonly targetEmail?: string
  readonly expiresInSeconds?: number
} {
  let ownershipMode: DeploymentOwnershipMode = "customer_owned"
  let builderRole: DeploymentControlRole | null | undefined
  let targetAccountId: string | undefined
  let targetEmail: string | undefined
  let expiresInSeconds: number | undefined
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    const value = requiredArg(argv[index + 1], claimCreateUsage)
    if (option === "--ownership") ownershipMode = parseOwnershipMode(value)
    else if (option === "--builder-role") builderRole = parseBuilderRole(value)
    else if (option === "--target-account") targetAccountId = value
    else if (option === "--target-email") targetEmail = value
    else if (option === "--expires-seconds") {
      expiresInSeconds = Number(value)
      if (!Number.isInteger(expiresInSeconds) || expiresInSeconds < 300 || expiresInSeconds > 2_592_000) {
        throw new Error("--expires-seconds must be an integer between 300 and 2592000")
      }
    } else throw new Error(`unknown deployments claim create option ${option}`)
    index += 1
  }
  return {
    ownershipMode,
    ...(builderRole !== undefined ? { builderRole } : {}),
    ...(targetAccountId ? { targetAccountId } : {}),
    ...(targetEmail ? { targetEmail } : {}),
    ...(expiresInSeconds !== undefined ? { expiresInSeconds } : {}),
  }
}

function parseClaimAcceptOptions(argv: readonly string[]): {
  readonly projectName?: string
  readonly projectSlug?: string
  readonly runtimeMode?: PublicationDeploymentMode
} {
  let projectName: string | undefined
  let projectSlug: string | undefined
  let runtimeMode: PublicationDeploymentMode | undefined
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    const value = requiredArg(argv[index + 1], claimAcceptUsage)
    if (option === "--name") projectName = value
    else if (option === "--slug") projectSlug = value
    else if (option === "--mode") runtimeMode = parseRuntimeMode(value)
    else throw new Error(`unknown deployments claim accept option ${option}`)
    index += 1
  }
  return {
    ...(projectName ? { projectName } : {}),
    ...(projectSlug ? { projectSlug } : {}),
    ...(runtimeMode ? { runtimeMode } : {}),
  }
}

function parseOwnershipMode(value: string): DeploymentOwnershipMode {
  const normalized = value.replace(/-/g, "_")
  if (normalized === "customer_owned" || normalized === "builder_managed" || normalized === "internal_team") {
    return normalized
  }
  throw new Error("--ownership must be customer-owned, builder-managed, or internal-team")
}

function parseBuilderRole(value: string): DeploymentControlRole | null {
  if (value === "none") return null
  if (value === "maintainer" || value === "deployer" || value === "operator" || value === "viewer") return value
  throw new Error("--builder-role must be maintainer, deployer, operator, viewer, or none")
}

function parseMemberRole(value: string): DeploymentControlRole {
  if (value === "admin" || value === "deployer" || value === "operator" || value === "viewer"
    || value === "billing" || value === "maintainer") return value
  throw new Error("member role must be admin, deployer, operator, viewer, billing, or maintainer")
}

function parseRuntimeMode(value: string): PublicationDeploymentMode {
  if (value === "local-runtime" || value === "local_runtime") return "local_runtime"
  if (value === "hosted-container" || value === "hosted_container") return "hosted_container"
  throw new Error("--mode must be local-runtime or hosted-container")
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
