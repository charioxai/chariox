import { createHash, randomUUID } from "node:crypto"
import { readFile, writeFile } from "node:fs/promises"

import {
  acceptDeploymentClaim,
  adoptLegacyDeploymentProject,
  armDeploymentCredentialCallbackChannel,
  bindDeploymentEnvironmentCredential,
  changeDeploymentEnvironmentLifecycle,
  createDeploymentClaim,
  createDeploymentCredentialProfile,
  createDeploymentEnvironmentDomain,
  createDeploymentProject,
  createDeploymentRelease,
  deleteDeploymentEnvironmentTelemetry,
  getDeploymentAccess,
  getDeploymentEnvironmentCredentials,
  getDeploymentEnvironmentDomains,
  getDeploymentEnvironmentUsage,
  exportDeploymentEnvironmentTelemetry,
  getDeploymentCredentialEnrollment,
  getDeploymentProject,
  listDeploymentProjects,
  listDeploymentCredentialProfiles,
  operateDeploymentEnvironmentDomain,
  promoteDeploymentRelease,
  reviewDeploymentClaim,
  requestDeploymentCredentialOperation,
  revokeDeploymentClaim,
  revokeDeploymentProjectMember,
  revokeDeploymentEnvironmentCredentialBinding,
  rollbackDeploymentEnvironment,
  upsertDeploymentProjectMember,
  updateDeploymentEnvironmentLimits,
  updateDeploymentEnvironmentOperations,
  waitForDeploymentCredentialEnrollment,
} from "./deployed-workflow-api.js"
import {
  armClaudeCredentialEnrollment,
  isClaudeCredentialProfile,
  requiresClaudeCredentialCallbackChannel,
} from "./deployed-workflow-credential-enrollment.js"
import {
  executeDeploymentAudienceCommand,
} from "./deployed-workflow-audience-command.js"
import { preparePublicationReleasePackage } from "./deployed-workflow-package.js"
import {
  deploymentSetupUsage,
  executeDeploymentSetupCommand,
} from "./deployed-workflow-setup-command.js"
import type {
  DeployedWorkflowProjectState,
  DeploymentAccessState,
  DeploymentClaimSummary,
  DeploymentControlRole,
  DeploymentCredentialProfileSummary,
  DeploymentCredentialEnrollmentSummary,
  DeploymentCredentialProfileResult,
  DeploymentCredentialProfilesResult,
  DeploymentCredentialVerification,
  DeploymentEnvironmentDomainState,
  DeploymentEnvironmentCredentialState,
  DeploymentEnvironmentUsageSummary,
  DeploymentEnvironmentOperationsPolicy,
  DeploymentPortfolioItem,
  DeploymentEnvironmentSummary,
  DeploymentOwnershipMode,
  DeploymentProjectKind,
  DeploymentRuntimeLimits,
  PublicationDeploymentMode,
  PublicationReleaseSummary,
  ReleasePromotionResult,
} from "./deployed-workflow-types.js"
import type { RuntimeAttachment, RuntimeSession } from "./cli-types.js"
import { loadPreferences, relayCloudProfile, type RelayCloudProfile } from "./preferences.js"

export interface DeployedWorkflowCommandOutput {
  readonly notice: string
  readonly footer: string
}

export interface DeployedWorkflowCommandRuntime {
  readonly isAttached?: () => boolean
  readonly sessionState?: () => RuntimeSession
  readonly attachmentState?: () => RuntimeAttachment | null
  readonly getRelayStatus?: () => Promise<{
    readonly configured: boolean
    readonly connected: boolean
    readonly daemon_id: string
  }>
  readonly sendCredentialEnrollmentKernelRequest?: (
    request: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>
  readonly sendDeploymentSetupKernelRequest?: (
    request: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>
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
  deps: DeployedWorkflowCommandRuntime & {
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
  const result = await executeDeployedWorkflowCommand(profile, [action ?? "list", ...args], deps)
  ;(deps.appendCloudNotice ?? deps.appendNotice)(result.notice)
  deps.flashFooter(result.footer, "info")
  return true
}

export async function executeDeployedWorkflowCommand(
  profile: RelayCloudProfile,
  argv: readonly string[],
  runtime?: DeployedWorkflowCommandRuntime,
): Promise<DeployedWorkflowCommandOutput> {
  const action = argv[0] ?? "list"
  if (action === "setup" || action === "wizard") {
    return executeDeploymentSetupCommand(profile, argv.slice(1), runtime)
  }
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
    return {
      notice: `${formatRelease(result.release)}\nrequest_id=${result.requestId}`,
      footer: `release #${result.release.sequence} verified`,
    }
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
  if (action === "usage" || action === "runtime") {
    const projectId = requiredArg(argv[1], runtimeUsage)
    const environmentId = requiredArg(argv[2], runtimeUsage)
    const result = await getDeploymentEnvironmentUsage(profile, projectId, environmentId)
    return {
      notice: formatDeploymentEnvironmentUsage(result.usage),
      footer: `${result.usage.invocationsToday} invocation${result.usage.invocationsToday === 1 ? "" : "s"} today`,
    }
  }
  if (action === "limit" || action === "limits") {
    const limitsAction = argv[1] ?? "show"
    if (limitsAction === "show" || limitsAction === "status") {
      const projectId = requiredArg(argv[2], limitsShowUsage)
      const environmentId = requiredArg(argv[3], limitsShowUsage)
      const result = await getDeploymentEnvironmentUsage(profile, projectId, environmentId)
      return {
        notice: formatDeploymentEnvironmentUsage(result.usage),
        footer: "deployment runtime limits",
      }
    }
    if (limitsAction === "set") {
      const projectId = requiredArg(argv[2], limitsSetUsage)
      const environmentId = requiredArg(argv[3], limitsSetUsage)
      const limitsPath = requiredArg(argv[4], limitsSetUsage)
      const limits = parseRuntimeLimits(await readJsonObject(limitsPath, "limits"))
      const idempotencyKey = parseIdempotencyKey(argv.slice(5), limitsSetUsage) ?? randomUUID()
      const updated = await updateDeploymentEnvironmentLimits(profile, {
        projectId,
        environmentId,
        limits,
        idempotencyKey,
      })
      const result = await getDeploymentEnvironmentUsage(profile, projectId, environmentId)
      return {
        notice: formatDeploymentEnvironmentUsage(result.usage),
        footer: updated.changed
          ? updated.restartRequested ? "runtime limits saved; restart requested" : "runtime limits saved"
          : "runtime limits already current",
      }
    }
    throw new Error(limitsUsage)
  }
  if (action === "operation" || action === "operations" || action === "ops" || action === "admission") {
    const operationsAction = argv[1] ?? "show"
    if (operationsAction === "show" || operationsAction === "status") {
      const projectId = requiredArg(argv[2], operationsShowUsage)
      const environmentId = requiredArg(argv[3], operationsShowUsage)
      const result = await getDeploymentEnvironmentUsage(profile, projectId, environmentId)
      return {
        notice: formatDeploymentEnvironmentUsage(result.usage),
        footer: `deployment operations ${effectiveOperationsPolicy(result.usage).admissionMode}`,
      }
    }
    if (operationsAction === "deny" || operationsAction === "resume") {
      const usage = operationsAction === "deny" ? operationsDenyUsage : operationsResumeUsage
      const projectId = requiredArg(argv[2], usage)
      const environmentId = requiredArg(argv[3], usage)
      const options = parseOperationsMutationOptions(argv.slice(4), usage)
      if (operationsAction === "deny" && !options.reason) throw new Error(operationsDenyUsage)
      const current = await getDeploymentEnvironmentUsage(profile, projectId, environmentId)
      await updateDeploymentEnvironmentOperations(profile, {
        projectId,
        environmentId,
        idempotencyKey: options.idempotencyKey ?? randomUUID(),
        policy: {
          ...effectiveOperationsPolicy(current.usage),
          admissionMode: operationsAction === "deny" ? "denied" : "accepting",
          admissionReason: operationsAction === "deny" ? options.reason! : null,
        },
      })
      const result = await getDeploymentEnvironmentUsage(profile, projectId, environmentId)
      return {
        notice: formatDeploymentEnvironmentUsage(result.usage),
        footer: operationsAction === "deny"
          ? "new deployment calls denied; in-flight calls continue"
          : "deployment calls resumed",
      }
    }
    if (operationsAction === "set") {
      const projectId = requiredArg(argv[2], operationsSetUsage)
      const environmentId = requiredArg(argv[3], operationsSetUsage)
      const policyPath = requiredArg(argv[4], operationsSetUsage)
      const policy = parseOperationsPolicy(await readJsonObject(policyPath, "operations policy"))
      const idempotencyKey = parseIdempotencyKey(argv.slice(5), operationsSetUsage) ?? randomUUID()
      const updated = await updateDeploymentEnvironmentOperations(profile, {
        projectId,
        environmentId,
        idempotencyKey,
        policy,
      })
      const result = await getDeploymentEnvironmentUsage(profile, projectId, environmentId)
      const pruned = updated.prunedInvocationCount + updated.prunedLogCount
      return {
        notice: formatDeploymentEnvironmentUsage(result.usage),
        footer: updated.changed
          ? pruned > 0 ? `operations policy saved; ${pruned} expired records removed` : "operations policy saved"
          : "operations policy already current",
      }
    }
    throw new Error(operationsUsage)
  }
  if (action === "telemetry") {
    const telemetryAction = argv[1]
    if (telemetryAction === "export") {
      const projectId = requiredArg(argv[2], telemetryExportUsage)
      const environmentId = requiredArg(argv[3], telemetryExportUsage)
      const outputPath = requiredArg(argv[4], telemetryExportUsage)
      const result = await exportDeploymentEnvironmentTelemetry(profile, projectId, environmentId)
      await writeVerifiedTelemetryExport(result, outputPath)
      return {
        notice: [
          `telemetry_export ${result.exportId}`,
          `  file ${outputPath}`,
          `  bytes ${result.byteSize}`,
          `  sha256 ${result.sha256}`,
          `  invocation_metadata ${result.counts.invocationMetadata}`,
          `  deployment_logs ${result.counts.deploymentLogs}`,
          `  audit_events ${result.counts.auditEvents}`,
          `  generated_at ${result.generatedAt}`,
        ].join("\n"),
        footer: `deployment telemetry exported to ${outputPath}`,
      }
    }
    if (telemetryAction === "delete") {
      const projectId = requiredArg(argv[2], telemetryDeleteUsage)
      const environmentId = requiredArg(argv[3], telemetryDeleteUsage)
      const idempotencyKey = parseIdempotencyKey(argv.slice(4), telemetryDeleteUsage) ?? randomUUID()
      const result = await deleteDeploymentEnvironmentTelemetry(profile, {
        projectId,
        environmentId,
        idempotencyKey,
      })
      return {
        notice: [
          `telemetry_deleted project=${projectId} environment=${environmentId}`,
          `  invocation_metadata ${result.deletedInvocationCount}`,
          `  deployment_logs ${result.deletedLogCount}`,
          `  active_invocations_protected ${result.protectedActiveInvocationCount}`,
          `  deleted_at ${result.deletedAt}`,
        ].join("\n"),
        footer: `${result.deletedInvocationCount + result.deletedLogCount} retained metadata records deleted`,
      }
    }
    throw new Error(telemetryUsage)
  }
  if (action === "credential" || action === "credentials") {
    const credentialAction = argv[1] ?? "list"
    if (credentialAction === "list" || credentialAction === "ls") {
      const result = await listDeploymentCredentialProfiles(profile)
      return {
        notice: result.profiles.length > 0
          ? result.profiles.map((credential) => formatDeploymentCredentialProfile(
              credential,
              result.setupAccess,
            )).join("\n")
          : "No deployment credentials.",
        footer: `${result.profiles.length} deployment credential${result.profiles.length === 1 ? "" : "s"}`,
      }
    }
    if (credentialAction === "show" || credentialAction === "status") {
      const projectId = requiredArg(argv[2], credentialShowUsage)
      const environmentId = requiredArg(argv[3], credentialShowUsage)
      const releaseId = argv[4]?.trim() || undefined
      const result = await getDeploymentEnvironmentCredentials(profile, {
        projectId,
        environmentId,
        ...(releaseId ? { releaseId } : {}),
      })
      return {
        notice: formatDeploymentEnvironmentCredentials(result.credentials),
        footer: result.credentials.ready ? "deployment credentials ready" : "deployment credentials require attention",
      }
    }
    const setupTarget = credentialAction === "setup" ? argv[2]?.trim() : undefined
    if (
      credentialAction === "retry"
      || (setupTarget && setupTarget !== "provider" && setupTarget !== "integration" && !argv[3])
    ) {
      const profileId = requiredArg(credentialAction === "retry" ? argv[2] : setupTarget, credentialRetryUsage)
      const credential = await requireCredentialOperationAvailable(profile, profileId, "retry")
      const result = await requestDeploymentCredentialOperation(profile, profileId, "retry", {
        waitForEnrollmentDetails: !isClaudeCredentialProfile(credential),
      })
      await armClaudeCredentialMutation(profile, result, runtime)
      return formatDeploymentCredentialOperation(
        result.profile,
        result.job,
        credentialSetupFooter(result.profile),
        result.setupDetailsStatus,
      )
    }
    if (credentialAction === "setup" || credentialAction === "connect") {
      const kind = requiredArg(argv[2], credentialConnectUsage)
      const identity = requiredArg(argv[3], credentialConnectUsage).toLowerCase()
      const label = requiredArg(argv[4], credentialConnectUsage)
      if (kind !== "provider" && kind !== "integration") throw new Error(credentialConnectUsage)
      if (kind === "provider" && !["codex", "claude", "opencode", "dev-stub"].includes(identity)) {
        throw new Error("deployment credential provider must be codex, claude, opencode, or dev-stub")
      }
      await requireCredentialManagementAvailable(profile)
      const result = await createDeploymentCredentialProfile(profile, {
        kind,
        ...(kind === "provider" ? { provider: identity } : { integration: identity }),
        label,
      }, {
        waitForEnrollmentDetails: kind !== "provider" || identity !== "claude",
      })
      await armClaudeCredentialMutation(profile, result, runtime)
      return formatDeploymentCredentialOperation(
        result.profile,
        result.job,
        credentialSetupFooter(result.profile),
        result.setupDetailsStatus,
      )
    }
    if (credentialAction === "enrollment") {
      const profileId = requiredArg(argv[2], credentialEnrollmentUsage)
      const listed = await requireCredentialManagementAvailable(profile)
      const credential = listed.profiles.find((candidate) => candidate.id === profileId)
      if (!credential) throw new Error(`credential ${profileId} was not found`)
      const result = isClaudeCredentialProfile(credential)
        ? await getDeploymentCredentialEnrollment(profile, profileId)
        : await waitForDeploymentCredentialEnrollment(profile, profileId)
      if (!result.enrollment) throw new Error(`credential ${profileId} has no enrollment`)
      await armClaudeCredentialMutation(profile, {
        profile: { ...credential, enrollment: result.enrollment },
      }, runtime)
      return {
        notice: formatDeploymentCredentialEnrollment(result.enrollment, credential),
        footer: `credential ${enrollmentStatusLabel(result.enrollment.status)}`,
      }
    }
    if (credentialAction === "test" || credentialAction === "rotate"
      || credentialAction === "revoke" || credentialAction === "purge") {
      const profileId = requiredArg(argv[2], credentialOperationUsage)
      const credential = await requireCredentialOperationAvailable(profile, profileId, credentialAction)
      const result = await requestDeploymentCredentialOperation(profile, profileId, credentialAction, {
        waitForEnrollmentDetails: credentialAction !== "rotate" || !isClaudeCredentialProfile(credential),
      })
      if (credentialAction === "rotate") {
        await armClaudeCredentialMutation(profile, result, runtime)
      }
      return formatDeploymentCredentialOperation(
        result.profile,
        result.job,
        credentialAction === "rotate" ? credentialSetupFooter(result.profile) : `credential ${credentialAction} requested`,
        result.setupDetailsStatus,
      )
    }
    if (credentialAction === "bind") {
      const projectId = requiredArg(argv[2], credentialBindUsage)
      const environmentId = requiredArg(argv[3], credentialBindUsage)
      const releaseId = requiredArg(argv[4], credentialBindUsage)
      const slotId = requiredArg(argv[5], credentialBindUsage)
      const profileId = requiredArg(argv[6], credentialBindUsage)
      await requireCredentialManagementAvailable(profile)
      const result = await bindDeploymentEnvironmentCredential(profile, {
        projectId,
        environmentId,
        releaseId,
        slotId,
        profileId,
      })
      return {
        notice: `${formatDeploymentEnvironmentCredentials(result.credentials)}\nrequest_id=${result.requestId}`,
        footer: `credential bound to ${slotId}`,
      }
    }
    if (credentialAction === "unbind") {
      const projectId = requiredArg(argv[2], credentialUnbindUsage)
      const environmentId = requiredArg(argv[3], credentialUnbindUsage)
      const slotId = requiredArg(argv[4], credentialUnbindUsage)
      await requireCredentialManagementAvailable(profile)
      const result = await revokeDeploymentEnvironmentCredentialBinding(profile, {
        projectId,
        environmentId,
        slotId,
      })
      return {
        notice: formatDeploymentEnvironmentCredentials(result.credentials),
        footer: `credential removed from ${slotId}`,
      }
    }
    throw new Error(credentialUsage)
  }
  if (action === "domain" || action === "domains") {
    const domainAction = argv[1] ?? "show"
    if (domainAction === "show" || domainAction === "status" || domainAction === "list") {
      const projectId = requiredArg(argv[2], domainShowUsage)
      const environmentId = requiredArg(argv[3], domainShowUsage)
      const result = await getDeploymentEnvironmentDomains(profile, projectId, environmentId)
      return {
        notice: formatDeploymentEnvironmentDomains(result.domains),
        footer: `${result.domains.domains.length} deployment domain${result.domains.domains.length === 1 ? "" : "s"}`,
      }
    }
    if (domainAction === "add") {
      const projectId = requiredArg(argv[2], domainAddUsage)
      const environmentId = requiredArg(argv[3], domainAddUsage)
      const hostname = requiredArg(argv[4], domainAddUsage)
      const result = await createDeploymentEnvironmentDomain(profile, { projectId, environmentId, hostname })
      return {
        notice: formatDeploymentEnvironmentDomains(result.domains),
        footer: `deployment domain ${hostname} added`,
      }
    }
    if (domainAction === "verify" || domainAction === "canonical" || domainAction === "remove") {
      const projectId = requiredArg(argv[2], domainOperationUsage)
      const environmentId = requiredArg(argv[3], domainOperationUsage)
      const domainId = requiredArg(argv[4], domainOperationUsage)
      const result = await operateDeploymentEnvironmentDomain(profile, {
        projectId,
        environmentId,
        domainId,
        operation: domainAction,
      })
      return {
        notice: formatDeploymentEnvironmentDomains(result.domains),
        footer: `deployment domain ${domainAction} complete`,
      }
    }
    throw new Error(domainUsage)
  }
  if (action === "audience") {
    return executeDeploymentAudienceCommand(profile, argv.slice(1))
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
  const capabilities = item.control
    ? [
        item.control.canRead ? "read" : null,
        item.control.canRelease ? "release" : null,
        item.control.canOperate ? "operate" : null,
        item.control.canManage ? "manage" : null,
      ].filter((capability): capability is string => capability !== null).join(",") || "none"
    : "unknown"
  return [
    item.project.id,
    item.project.name,
    item.project.kind,
    `ownership=${item.project.ownershipMode ?? "internal_team"}`,
    `role=${item.control?.role ?? "unknown"}`,
    `capabilities=${capabilities}`,
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

export function formatDeploymentCredentialProfile(
  profile: DeploymentCredentialProfileSummary,
  setupAccess: DeploymentCredentialProfilesResult["setupAccess"] = "available",
): string {
  const verification = effectiveCredentialVerification(profile)
  return [
    `credential ${profile.id} ${profile.status}`,
    `  kind ${profile.kind} ${profile.kind === "provider" ? profile.provider ?? "unknown" : profile.integration ?? "unknown"}`,
    `  label ${profile.label}`,
    `  account ${verification === "verified" ? profile.accountLabel ?? "verified" : "unverified"}`,
    `  verification ${verification}`,
    `  version ${profile.version}`,
    `  runner ${profile.runnerConnected ? "online" : "offline"}`,
    `  expires_at ${profile.expiresAt ?? "none"}`,
    `  checked_at ${profile.lastCheckedAt ?? "never"}`,
    ...(profile.enrollment ? [
      `  setup_mode ${profile.enrollment.mode}`,
      `  setup_status ${profile.enrollment.status}`,
      `  setup_expires_at ${profile.enrollment.expiresAt}`,
      ...(profile.enrollment.instructions ? [`  setup_instructions ${profile.enrollment.instructions}`] : []),
    ] : []),
    ...(profile.statusCode ? [`  status_code ${profile.statusCode}`] : []),
    `  actions ${credentialProfileActions(profile, setupAccess)}`,
  ].join("\n")
}

export function formatDeploymentCredentialEnrollment(
  enrollment: DeploymentCredentialEnrollmentSummary,
  profile: DeploymentCredentialProfileSummary,
): string {
  const verification = enrollmentVerification(enrollment)
  const verificationUrl = safeCredentialVerificationUrl(enrollment.verificationUrl, profile)
  const userCode = safeCredentialUserCode(enrollment.userCode)
  return [
    `setup ${enrollment.profileId} version=${enrollment.targetVersion}`,
    `  mode ${enrollment.mode}`,
    `  status ${enrollment.status}`,
    `  verification ${verification}`,
    `  expires_at ${enrollment.expiresAt}`,
    ...(enrollment.statusCode ? [`  status_code ${enrollment.statusCode}`] : []),
    ...(enrollment.instructions ? [`  instructions ${enrollment.instructions}`] : []),
    ...(verificationUrl ? [`  verification_url ${verificationUrl}`] : []),
    ...(userCode ? [`  user_code ${userCode}`] : []),
  ].join("\n")
}

export function formatDeploymentEnvironmentCredentials(credentials: DeploymentEnvironmentCredentialState): string {
  return [
    `credentials project=${credentials.projectId} environment=${credentials.environmentId}`,
    `release ${credentials.releaseId}`,
    `ready ${credentials.ready ? "yes" : "no"}`,
    ...credentials.slots.map((state) => [
      `slot ${state.slot.slotId} ${state.readiness}`,
      `  kind ${state.slot.kind} ${state.slot.kind === "provider" ? state.slot.provider ?? "unknown" : state.slot.integration ?? "unknown"}`,
      `  required ${state.slot.required ? "yes" : "no"}`,
      `  uses ${state.slot.uses.join(",") || "none"}`,
      `  profile ${state.binding?.profileId ?? "none"}`,
      ...(state.binding ? [
        `  binding_id ${state.binding.id}`,
        `  binding ${state.binding.status} version=${state.binding.version}`,
        `  label ${state.binding.profile.label}`,
      ] : []),
    ].join("\n")),
  ].join("\n")
}

async function requireCredentialOperationAvailable(
  cloudProfile: RelayCloudProfile,
  profileId: string,
  operation: "retry" | "test" | "rotate" | "revoke" | "purge",
): Promise<DeploymentCredentialProfileSummary> {
  const listed = await listDeploymentCredentialProfiles(cloudProfile)
  if (operation !== "test") requireCredentialManagementAccess(listed)
  const credential = listed.profiles.find((candidate) => candidate.id === profileId)
  if (!credential) throw new Error(`credential ${profileId} was not found`)
  if (credentialSetupActive(credential)) {
    throw new Error(
      `credential ${profileId} ${enrollmentStatusLabel(credential.enrollment?.status)}; ${operation} is unavailable until setup finishes`,
    )
  }
  if (operation === "retry" && !credentialSetupRetryable(credential)) {
    throw new Error(`credential ${profileId} does not require setup retry`)
  }
  if (operation === "purge" && credential.status !== "revoked") {
    throw new Error(`credential ${profileId} must be revoked before purge`)
  }
  if (operation !== "purge" && credential.status === "revoked") {
    throw new Error(`credential ${profileId} is revoked; only purge is available`)
  }
  return credential
}

async function armClaudeCredentialMutation(
  cloudProfile: RelayCloudProfile,
  result: DeploymentCredentialProfileResult,
  runtime: DeployedWorkflowCommandRuntime | undefined,
): Promise<void> {
  const enrollment = result.profile.enrollment
  if (!isClaudeCredentialProfile(result.profile)) return
  if (!enrollment) {
    throw new Error("Claude credential setup did not return an enrollment")
  }
  if (enrollment.profileId !== result.profile.id || result.profile.accountId !== cloudProfile.accountId) {
    throw new Error("Claude credential setup returned a mismatched enrollment")
  }
  if (!requiresClaudeCredentialCallbackChannel(result.profile, enrollment)) return

  const attachment = runtime?.attachmentState?.() ?? null
  const session = runtime?.sessionState?.()
  if (!runtime?.isAttached?.() || !attachment || !session) {
    throw new Error("Claude provider-native credential setup requires an attached Arroba TUI")
  }
  if (attachment.session_id !== session.id) {
    throw new Error("Claude credential setup attachment is stale")
  }
  const agentId = session.focused_agent_id
  if (!agentId || !session.agents.some((agent) => agent.id === agentId)) {
    throw new Error("Claude credential setup requires a focused session agent")
  }
  if (!runtime.sendCredentialEnrollmentKernelRequest) {
    throw new Error("Claude credential setup requires kernel protocol 241 support")
  }
  if (!runtime.getRelayStatus) {
    throw new Error("Claude credential setup cannot resolve its kernel relay target")
  }
  const relayStatus = await runtime.getRelayStatus()
  if (!relayStatus.configured || !relayStatus.connected || !relayStatus.daemon_id.trim()) {
    throw new Error("Claude credential setup requires the attached kernel to be online in Cloud")
  }

  await armClaudeCredentialEnrollment({
    sendKernelRequest: runtime.sendCredentialEnrollmentKernelRequest,
    armCloudCallbackChannel: (binding) => armDeploymentCredentialCallbackChannel(cloudProfile, binding),
  }, {
    accountId: cloudProfile.accountId,
    enrollmentId: enrollment.id,
    profileId: result.profile.id,
    targetVersion: enrollment.targetVersion,
    enrollmentExpiresAt: enrollment.expiresAt,
    realmId: cloudProfile.realmId,
    kernelTarget: relayStatus.daemon_id,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentId,
  })
}

async function requireCredentialManagementAvailable(
  cloudProfile: RelayCloudProfile,
): Promise<DeploymentCredentialProfilesResult> {
  const listed = await listDeploymentCredentialProfiles(cloudProfile)
  requireCredentialManagementAccess(listed)
  return listed
}

function requireCredentialManagementAccess(listed: DeploymentCredentialProfilesResult): void {
  if (listed.setupAccess === "restricted") {
    throw new Error("Credential management requires account owner or admin access")
  }
}

function credentialSetupActive(profile: DeploymentCredentialProfileSummary): boolean {
  return profile.enrollment?.status === "pending"
    || profile.enrollment?.status === "claimed"
    || profile.verification === "setup_required"
    || profile.verification === "setup_in_progress"
}

function effectiveCredentialVerification(profile: DeploymentCredentialProfileSummary): DeploymentCredentialVerification {
  if (profile.status === "revoked") return "revoked"
  if (profile.enrollment) return enrollmentVerification(profile.enrollment)
  if (profile.verification) return profile.verification
  if (profile.status === "ready") return "unverified"
  if (profile.status === "expired") return "expired"
  if (profile.status === "error") return "failed"
  return "setup_in_progress"
}

function enrollmentVerification(enrollment: DeploymentCredentialEnrollmentSummary): DeploymentCredentialVerification {
  if (enrollment.mode === "runner_seeded" && enrollment.status === "consumed") return "unverified"
  if (enrollment.status === "pending") return "setup_required"
  if (enrollment.status === "claimed") return "setup_in_progress"
  if (enrollment.status === "consumed") return "verified"
  if (enrollment.status === "expired") return "expired"
  return "failed"
}

function credentialProfileActions(
  profile: DeploymentCredentialProfileSummary,
  setupAccess: DeploymentCredentialProfilesResult["setupAccess"],
): string {
  if (credentialSetupActive(profile)) return "disabled_while_setup_active"
  if (setupAccess === "restricted") return profile.status === "revoked" ? "none" : "test"
  if (credentialSetupRetryable(profile)) return "retry_setup"
  return profile.status === "revoked" ? "purge" : "test rotate revoke"
}

function credentialSetupRetryable(profile: DeploymentCredentialProfileSummary): boolean {
  if (profile.enrollment?.status === "consumed") return false
  if (profile.status !== "connecting" && profile.status !== "error" && profile.status !== "expired") return false
  const verification = effectiveCredentialVerification(profile)
  return verification === "failed" || verification === "expired"
}

function credentialSetupFooter(profile: DeploymentCredentialProfileSummary): string {
  const verification = effectiveCredentialVerification(profile)
  if (verification === "setup_required") return "credential setup required"
  if (verification === "setup_in_progress") return "credential setup in progress"
  if (verification === "unverified") return "credential setup complete; identity unverified"
  if (verification === "verified") return "credential setup verified"
  return `credential setup ${verification}`
}

function enrollmentStatusLabel(
  status: DeploymentCredentialEnrollmentSummary["status"] | undefined,
): string {
  if (status === "pending") return "setup required"
  if (status === "claimed") return "setup in progress"
  return status ? `setup ${status}` : "setup in progress"
}

function safeCredentialVerificationUrl(
  value: string | null | undefined,
  profile: DeploymentCredentialProfileSummary,
): string | null {
  if (!value || value.length > 2_048) return null
  try {
    const url = new URL(value)
    if (url.protocol !== "https:" || url.username || url.password || url.hash || url.port) return null
    if ([...url.searchParams].length > 24 || url.search.length > 1_536) return null
    if ([...url.searchParams.keys()].some(secretBearingCredentialVerificationParameter)) return null
    if ([...url.searchParams.keys()].some(redirectBearingCredentialVerificationParameter)) return null
    const policy = providerVerificationUrlPolicies[credentialAdapter(profile)]
    const matchedPolicy = policy?.find((entry) => entry.origin === url.origin && entry.pathname === url.pathname)
    if (!matchedPolicy || !validProviderCredentialVerificationParameters(url, matchedPolicy)) return null
    return url.toString()
  } catch {
    return null
  }
}

type ProviderVerificationUrlPolicy = {
  readonly origin: string
  readonly pathname: string
  readonly allowedParameters: readonly string[]
  readonly requiredParameters?: readonly string[]
  readonly redirectUri?: string
  readonly responseType?: string
  readonly requireClientId?: boolean
  readonly codeChallengeMethod?: string
}

const providerVerificationUrlPolicies: Readonly<Record<string, readonly ProviderVerificationUrlPolicy[]>> = {
  codex: [{
    origin: "https://auth.openai.com",
    pathname: "/codex/device",
    allowedParameters: ["user_code", "verification_code"],
  }],
  claude: [{
    origin: "https://claude.com",
    pathname: "/cai/oauth/authorize",
    allowedParameters: [
      "client_id",
      "response_type",
      "redirect_uri",
      "scope",
      "code_challenge",
      "code_challenge_method",
      "state",
    ],
    requiredParameters: [
      "client_id",
      "response_type",
      "redirect_uri",
      "code_challenge",
      "code_challenge_method",
      "state",
    ],
    redirectUri: "https://claude.com/cai/oauth/code/callback",
    responseType: "code",
    requireClientId: true,
    codeChallengeMethod: "S256",
  }],
  opencode: [{
    origin: "https://auth.openai.com",
    pathname: "/codex/device",
    allowedParameters: ["user_code", "verification_code"],
  }],
}

function validProviderCredentialVerificationParameters(
  url: URL,
  policy: ProviderVerificationUrlPolicy,
): boolean {
  const allowedParameters = new Set(policy.allowedParameters)
  if ([...url.searchParams.keys()].some((name) => !allowedParameters.has(name))) return false
  if ([...allowedParameters].some((name) => url.searchParams.getAll(name).length > 1)) return false
  if ((policy.requiredParameters ?? []).some((name) => {
    const values = url.searchParams.getAll(name)
    return values.length !== 1 || !values[0]?.trim()
  })) return false
  const redirectUris = url.searchParams.getAll("redirect_uri")
  if (policy.redirectUri) {
    if (redirectUris.length !== 1 || redirectUris[0] !== policy.redirectUri) return false
    let redirect: URL
    try {
      redirect = new URL(redirectUris[0])
    } catch {
      return false
    }
    if (
      redirect.protocol !== "https:"
      || redirect.username
      || redirect.password
      || redirect.port
      || redirect.hash
      || redirect.search
      || redirect.toString() !== policy.redirectUri
    ) return false
  } else if (redirectUris.length > 0) {
    return false
  }
  if (policy.responseType) {
    const responseTypes = url.searchParams.getAll("response_type")
    if (responseTypes.length !== 1 || responseTypes[0] !== policy.responseType) return false
  }
  if (policy.requireClientId) {
    const clientIds = url.searchParams.getAll("client_id")
    if (clientIds.length !== 1 || !clientIds[0]?.trim() || clientIds[0].length > 256) return false
  }
  if (policy.codeChallengeMethod) {
    const methods = url.searchParams.getAll("code_challenge_method")
    if (methods.length !== 1 || methods[0] !== policy.codeChallengeMethod) return false
  }
  return true
}

function secretBearingCredentialVerificationParameter(name: string): boolean {
  const normalized = name.trim().toLowerCase().replaceAll("-", "_").replaceAll(".", "_")
  const compact = normalized.replaceAll("_", "")
  return normalized === "api_key"
    || normalized === "code"
    || normalized === "authorization_code"
    || normalized === "password"
    || normalized === "credential"
    || normalized === "code_verifier"
    || normalized === "device_code"
    || normalized.endsWith("_token")
    || normalized.endsWith("_secret")
    || normalized.includes("password")
    || normalized.includes("credential")
    || compact === "apikey"
    || compact === "accesstoken"
    || compact === "refreshtoken"
    || compact === "clientsecret"
    || compact === "devicesecret"
}

function redirectBearingCredentialVerificationParameter(name: string): boolean {
  const normalized = name.trim().toLowerCase().replaceAll("-", "_").replaceAll(".", "_")
  return normalized === "redirect"
    || normalized === "redirect_url"
    || normalized === "return_to"
    || normalized === "return_url"
    || normalized === "continue"
    || normalized === "next"
    || normalized === "url"
}

function credentialAdapter(profile: DeploymentCredentialProfileSummary): string {
  return (profile.kind === "provider" ? profile.provider : profile.integration)?.trim().toLowerCase() ?? ""
}

function safeCredentialUserCode(value: string | null | undefined): string | null {
  const code = value?.trim()
  return code && /^[A-Za-z0-9-]{4,64}$/.test(code) ? code : null
}

export function formatDeploymentEnvironmentDomains(domains: DeploymentEnvironmentDomainState): string {
  return [
    `domains project=${domains.projectId} environment=${domains.environmentId}`,
    `canonical ${domains.canonicalHostname}`,
    ...domains.domains.map((domain) => [
      `domain ${domain.id} ${domain.kind} ${domain.status} canonical=${domain.isCanonical ? "yes" : "no"}`,
      `  hostname ${domain.hostname}`,
      `  url ${domain.publicUrl}`,
      `  dns ${domain.dnsStatus}`,
      `  tls ${domain.tlsStatus}`,
      `  redirect_to_canonical ${domain.redirectToCanonical ? "yes" : "no"}`,
      ...(domain.verificationName ? [`  txt_name ${domain.verificationName}`] : []),
      ...(domain.verificationValue ? [`  txt_value ${domain.verificationValue}`] : []),
      ...(domain.cnameTarget ? [`  cname ${domain.cnameTarget}`] : []),
      `  checked_at ${domain.lastCheckedAt ?? "never"}`,
      `  verified_at ${domain.verifiedAt ?? "never"}`,
      `  activated_at ${domain.activatedAt ?? "never"}`,
      ...(domain.lastError ? [`  error ${domain.lastError}`] : []),
    ].join("\n")),
  ].join("\n")
}

export function formatDeploymentEnvironmentUsage(usage: DeploymentEnvironmentUsageSummary): string {
  const operations = effectiveOperationsPolicy(usage)
  return [
    `runtime project=${usage.projectId} environment=${usage.environmentId}`,
    `deployment ${usage.deploymentId ?? "none"}`,
    `limits ${formatDeploymentRuntimeLimits(usage.limits)}`,
    `operations admission=${operations.admissionMode} policy_version=${usage.operationsPolicyVersion ?? 1} reason=${operations.admissionReason ?? "none"}`,
    `retention invocation_metadata_days=${operations.invocationMetadataRetentionDays} deployment_log_days=${operations.deploymentLogRetentionDays} content_capture=disabled`,
    `alerts error_rate_percent=${operations.alertThresholds.errorRatePercent} average_duration_ms=${operations.alertThresholds.averageDurationMs} queue_depth_percent=${operations.alertThresholds.queueDepthPercent} daily_usage_percent=${operations.alertThresholds.dailyUsagePercent} health_stale_seconds=${operations.alertThresholds.healthStaleSeconds}`,
    `usage active=${usage.activeInvocations} minute=${usage.invocationsLastMinute} today=${usage.invocationsToday} units=${usage.usageUnitsToday}`,
    `outcomes succeeded=${usage.succeededToday} failed=${usage.failedToday} timed_out=${usage.timedOutToday} interrupted=${usage.interruptedToday}`,
    `latency average_ms=${formatMetric(usage.averageDurationMs)} maximum_ms=${formatMetric(usage.maximumDurationMs)} queue_average_ms=${formatMetric(usage.averageQueuedMs)}`,
    `traffic request_bytes=${usage.requestBytesToday} response_bytes=${usage.responseBytesToday}`,
    `active_alerts ${usage.alerts?.length ?? 0}`,
    ...(usage.alerts ?? []).map((alert) => (
      `alert ${alert.code} severity=${alert.severity} current=${formatMetric(alert.currentValue)} threshold=${formatMetric(alert.threshold)} unit=${alert.unit ?? "none"}\n`
      + `  ${alert.message}\n`
      + `  observed_at ${alert.observedAt}`
    )),
    `diagnostics ${usage.diagnostics?.length ?? 0}`,
    ...(usage.diagnostics ?? []).map((diagnostic) => (
      `diagnostic ${diagnostic.code} ${diagnostic.status}\n`
      + `  ${diagnostic.message}\n`
      + `  details ${formatOperationalDetails(diagnostic.details)}\n`
      + `  observed_at ${diagnostic.observedAt ?? "unknown"}`
    )),
    `audit_events ${usage.auditEvents?.length ?? 0}`,
    ...(usage.auditEvents ?? []).map((event) => (
      `audit ${event.eventType} actor=${event.actorUserId ?? event.actorKind.toLowerCase()} subject=${event.subjectType ?? "none"}:${event.subjectId ?? "none"}\n`
      + `  occurred_at ${event.occurredAt}`
    )),
    `privacy capture=${usage.privacy?.captureMode ?? "metadata_only"} content_capture=disabled active_invocations_protected=yes state_contract=${usage.privacy?.stateContract ?? "stateless_external_storage"} persistent_ephemeral_storage=no`,
    `generated_at ${usage.generatedAt}`,
    ...usage.recentInvocations.map((invocation) => [
      `invocation ${invocation.invocationId} ${invocation.state} ${invocation.outcome ?? "pending"}`,
      `  transport ${invocation.transport} status=${invocation.statusCode ?? "none"}`
      + ` error=${invocation.errorCode ?? "none"}`
      + ` admission=${invocation.admissionDeferred ? "deferred_replay" : "live"}`,
      `  timing queued_ms=${invocation.queuedMs} duration_ms=${formatMetric(invocation.durationMs)}`,
      `  bytes request=${invocation.requestBytes ?? "none"} response=${invocation.responseBytes ?? "none"} units=${invocation.usageUnits}`,
      `  started_at ${invocation.startedAt}`,
      `  finished_at ${invocation.finishedAt ?? "active"}`,
    ].join("\n")),
  ].join("\n")
}

async function writeVerifiedTelemetryExport(
  result: import("./deployed-workflow-types.js").DeploymentTelemetryExportResult,
  outputPath: string,
): Promise<void> {
  const normalized = result.contentBase64.replace(/\s+/g, "")
  const content = Buffer.from(normalized, "base64")
  if (content.toString("base64").replace(/=+$/, "") !== normalized.replace(/=+$/, "")) {
    throw new Error("deployment telemetry export content is not valid base64")
  }
  if (content.byteLength !== result.byteSize) {
    throw new Error("deployment telemetry export byte size does not match")
  }
  const sha256 = `sha256:${createHash("sha256").update(content).digest("hex")}`
  if (sha256 !== result.sha256) {
    throw new Error("deployment telemetry export digest does not match")
  }
  await writeFile(outputPath, content, { flag: "wx", mode: 0o600 })
}

function formatDeploymentRuntimeLimits(limits: DeploymentRuntimeLimits): string {
  const values = runtimeLimitKeys.flatMap((key) => limits[key] === undefined ? [] : [`${key}=${limits[key]}`])
  return values.join(" ") || "platform_defaults"
}

function formatMetric(value: number | null | undefined): string {
  return value === null || value === undefined ? "none" : String(value)
}

function formatOperationalDetails(
  details: Readonly<Record<string, string | number | boolean | null>> | undefined,
): string {
  if (!details) return "none"
  const values = Object.entries(details)
    .filter(([, value]) => value !== null)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `${key}=${String(value)}`)
  return values.join(" ") || "none"
}

function formatDeploymentCredentialOperation(
  profile: DeploymentCredentialProfileSummary,
  job: { readonly id: string; readonly type: string; readonly status: string } | null | undefined,
  footer: string,
  setupDetailsStatus?: "available" | "unavailable",
): DeployedWorkflowCommandOutput {
  return {
    notice: [
      formatDeploymentCredentialProfile(profile),
      ...(profile.enrollment ? [formatDeploymentCredentialEnrollment(profile.enrollment, profile)] : []),
      ...(setupDetailsStatus === "unavailable" ? ["setup_details unavailable"] : []),
      ...(job ? [`job ${job.id} ${job.type} ${job.status}`] : []),
    ].join("\n"),
    footer: setupDetailsStatus === "unavailable" ? `${footer}; setup details unavailable` : footer,
  }
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
  result: ReleasePromotionResult & { readonly requestId: string },
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
      `request_id=${result.requestId}`,
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
const credentialShowUsage = "usage: deployments credentials show <project-id> <environment-id> [release-id]"
const credentialConnectUsage = "usage: deployments credentials setup provider <codex|claude|opencode|dev-stub> <label> | integration <identity> <label>"
const credentialRetryUsage = "usage: deployments credentials setup <profile-id> | retry <profile-id>"
const credentialEnrollmentUsage = "usage: deployments credentials enrollment <profile-id>"
const credentialOperationUsage = "usage: deployments credentials test|rotate|revoke|purge <profile-id>"
const credentialBindUsage = "usage: deployments credentials bind <project-id> <environment-id> <release-id> <slot-id> <profile-id>"
const credentialUnbindUsage = "usage: deployments credentials unbind <project-id> <environment-id> <slot-id>"
const credentialUsage = "usage: deployments credentials list|show|setup|retry|enrollment|test|rotate|revoke|purge|bind|unbind ..."
const domainShowUsage = "usage: deployments domains show <project-id> <environment-id>"
const domainAddUsage = "usage: deployments domains add <project-id> <environment-id> <hostname>"
const domainOperationUsage = "usage: deployments domains verify|canonical|remove <project-id> <environment-id> <domain-id>"
const domainUsage = "usage: deployments domains show|add|verify|canonical|remove ..."
const runtimeUsage = "usage: deployments usage <project-id> <environment-id>"
const limitsShowUsage = "usage: deployments limits show <project-id> <environment-id>"
const limitsSetUsage = "usage: deployments limits set <project-id> <environment-id> <json-file> [--idempotency-key value]"
const limitsUsage = "usage: deployments limits show|set ..."
const operationsShowUsage = "usage: deployments operations show <project-id> <environment-id>"
const operationsDenyUsage = "usage: deployments operations deny <project-id> <environment-id> --reason <text> [--idempotency-key value]"
const operationsResumeUsage = "usage: deployments operations resume <project-id> <environment-id> [--idempotency-key value]"
const operationsSetUsage = "usage: deployments operations set <project-id> <environment-id> <json-file> [--idempotency-key value]"
const operationsUsage = "usage: deployments operations show|deny|resume|set ..."
const telemetryExportUsage = "usage: deployments telemetry export <project-id> <environment-id> <output-path>"
const telemetryDeleteUsage = "usage: deployments telemetry delete <project-id> <environment-id> [--idempotency-key value]"
const telemetryUsage = "usage: deployments telemetry export|delete ..."
const deploymentsUsage = `usage: deployments list | show <project-id> | setup|wizard ... | create <name> | adopt <legacy-id> | preflight <package> | release <project-id> <package> | promote <project-id> <environment-id> <release-id> | rollback <project-id> <environment-id> <promotion-id> | start|stop|restart <project-id> <environment-id> | usage <project-id> <environment-id> | limits show|set ... | operations show|deny|resume|set ... | telemetry export|delete ... | credentials list|show|setup|retry|enrollment|test|rotate|revoke|purge|bind|unbind ... | domains show|add|verify|canonical|remove ... | audience show|policy|grant|invite|key|jwt|webhook ... | claim create|review|accept|revoke ... | access <project-id> | member add|revoke ...\n${deploymentSetupUsage}`

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
  readonly limits?: DeploymentRuntimeLimits
  readonly idempotencyKey?: string
}> {
  let configuration: Record<string, unknown> | undefined
  let limits: DeploymentRuntimeLimits | undefined
  let idempotencyKey: string | undefined
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    const value = requiredArg(argv[index + 1], promoteUsage)
    if (option === "--configuration") configuration = await readJsonObject(value, "configuration")
    else if (option === "--limits") limits = parseRuntimeLimits(await readJsonObject(value, "limits"))
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

function parseOperationsMutationOptions(argv: readonly string[], usage: string): {
  readonly reason?: string
  readonly idempotencyKey?: string
} {
  let reason: string | undefined
  let idempotencyKey: string | undefined
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    if (option === "--reason") {
      reason = requiredArg(argv[index + 1], usage)
      if (reason.length > 240) throw new Error("deployment admission reason must be at most 240 characters")
      index += 1
    } else if (option === "--idempotency-key") {
      idempotencyKey = requiredArg(argv[index + 1], usage)
      index += 1
    } else {
      throw new Error(usage)
    }
  }
  return { ...(reason ? { reason } : {}), ...(idempotencyKey ? { idempotencyKey } : {}) }
}

async function readJsonObject(path: string, label: string): Promise<Record<string, unknown>> {
  const value = JSON.parse(await readFile(path, "utf8")) as unknown
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} file must contain a JSON object`)
  }
  return value as Record<string, unknown>
}

const runtimeLimitRanges = {
  concurrency: [1, 64],
  queue: [0, 1_000],
  invocations_per_minute: [1, 100_000],
  body_bytes: [1, 16 * 1024 * 1024],
  duration_ms: [100, 30 * 60 * 1_000],
  daily_usage_units: [1, 1_000_000_000],
  ephemeral_storage_bytes: [1024 * 1024, 10 * 1024 * 1024 * 1024],
} as const satisfies Record<keyof DeploymentRuntimeLimits, readonly [number, number]>

const runtimeLimitKeys = Object.keys(runtimeLimitRanges) as readonly (keyof DeploymentRuntimeLimits)[]

function parseRuntimeLimits(value: Record<string, unknown>): DeploymentRuntimeLimits {
  const unknown = Object.keys(value).find((key) => !(key in runtimeLimitRanges))
  if (unknown) throw new Error(`deployment limits contain unsupported field ${unknown}`)
  const limits: Partial<Record<keyof DeploymentRuntimeLimits, number>> = {}
  for (const key of runtimeLimitKeys) {
    const candidate = value[key]
    if (candidate === undefined) continue
    const [minimum, maximum] = runtimeLimitRanges[key]
    if (!Number.isSafeInteger(candidate) || (candidate as number) < minimum || (candidate as number) > maximum) {
      throw new Error(`deployment limit ${key} must be an integer between ${minimum} and ${maximum}`)
    }
    limits[key] = candidate as number
  }
  return limits
}

const defaultDeploymentOperationsPolicy: DeploymentEnvironmentOperationsPolicy = {
  admissionMode: "accepting",
  admissionReason: null,
  invocationMetadataRetentionDays: 30,
  deploymentLogRetentionDays: 30,
  alertThresholds: {
    errorRatePercent: 20,
    averageDurationMs: 30_000,
    queueDepthPercent: 80,
    dailyUsagePercent: 80,
    healthStaleSeconds: 120,
  },
}

function effectiveOperationsPolicy(usage: DeploymentEnvironmentUsageSummary): DeploymentEnvironmentOperationsPolicy {
  return usage.operationsPolicy ?? defaultDeploymentOperationsPolicy
}

function parseOperationsPolicy(value: Record<string, unknown>): DeploymentEnvironmentOperationsPolicy {
  assertOnlyFields(value, [
    "admissionMode",
    "admissionReason",
    "invocationMetadataRetentionDays",
    "deploymentLogRetentionDays",
    "alertThresholds",
  ], "deployment operations policy")
  const admissionMode = value.admissionMode
  if (admissionMode !== "accepting" && admissionMode !== "denied") {
    throw new Error("deployment admissionMode must be accepting or denied")
  }
  const admissionReason = typeof value.admissionReason === "string" && value.admissionReason.trim()
    ? value.admissionReason.trim()
    : null
  if (admissionMode === "denied" && !admissionReason) {
    throw new Error("denied deployment operations policy requires admissionReason")
  }
  const thresholds = objectRecord(value.alertThresholds, "deployment alertThresholds")
  assertOnlyFields(thresholds, [
    "errorRatePercent",
    "averageDurationMs",
    "queueDepthPercent",
    "dailyUsagePercent",
    "healthStaleSeconds",
  ], "deployment alert thresholds")
  return {
    admissionMode,
    admissionReason: admissionMode === "denied" ? admissionReason : null,
    invocationMetadataRetentionDays: requiredPolicyInteger(value.invocationMetadataRetentionDays, "invocationMetadataRetentionDays", 1, 365),
    deploymentLogRetentionDays: requiredPolicyInteger(value.deploymentLogRetentionDays, "deploymentLogRetentionDays", 1, 365),
    alertThresholds: {
      errorRatePercent: requiredPolicyNumber(thresholds.errorRatePercent, "errorRatePercent", 0, 100),
      averageDurationMs: requiredPolicyInteger(thresholds.averageDurationMs, "averageDurationMs", 100, 1_800_000),
      queueDepthPercent: requiredPolicyNumber(thresholds.queueDepthPercent, "queueDepthPercent", 0, 100),
      dailyUsagePercent: requiredPolicyNumber(thresholds.dailyUsagePercent, "dailyUsagePercent", 0, 100),
      healthStaleSeconds: requiredPolicyInteger(thresholds.healthStaleSeconds, "healthStaleSeconds", 10, 86_400),
    },
  }
}

function objectRecord(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object`)
  return value as Record<string, unknown>
}

function assertOnlyFields(value: Record<string, unknown>, allowed: readonly string[], label: string): void {
  const unknown = Object.keys(value).find((key) => !allowed.includes(key))
  if (unknown) throw new Error(`${label} contains unsupported field ${unknown}`)
}

function requiredPolicyInteger(value: unknown, label: string, minimum: number, maximum: number): number {
  if (!Number.isSafeInteger(value)) throw new Error(`deployment ${label} must be an integer`)
  return requiredPolicyNumber(value, label, minimum, maximum)
}

function requiredPolicyNumber(value: unknown, label: string, minimum: number, maximum: number): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < minimum || value > maximum) {
    throw new Error(`deployment ${label} must be between ${minimum} and ${maximum}`)
  }
  return value
}

function requiredArg(value: string | undefined, usage: string): string {
  if (!value?.trim()) throw new Error(usage)
  return value.trim()
}
