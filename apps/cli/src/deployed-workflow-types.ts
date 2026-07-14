export type DeploymentProjectKind = "workflow_endpoint" | "agent_app"
export type PublicationDeploymentMode = "local_runtime" | "hosted_container"
export type DeploymentOwnershipMode = "customer_owned" | "builder_managed" | "internal_team"
export type DeploymentControlRole = "owner" | "admin" | "deployer" | "operator" | "viewer" | "billing" | "maintainer"

export interface DeploymentRuntimeLimits {
  readonly concurrency?: number
  readonly queue?: number
  readonly invocations_per_minute?: number
  readonly body_bytes?: number
  readonly duration_ms?: number
  readonly daily_usage_units?: number
  readonly ephemeral_storage_bytes?: number
}

export interface DeploymentProjectSummary {
  readonly id: string
  readonly accountId: string
  readonly name: string
  readonly slug: string
  readonly kind: DeploymentProjectKind
  readonly ownershipMode?: DeploymentOwnershipMode
  readonly builderAccountId?: string | null
  readonly origin: string
  readonly defaultEnvironmentSlug: string
  readonly createdAt: string
  readonly updatedAt: string
}

export interface PublicationReleaseSummary {
  readonly id: string
  readonly projectId: string
  readonly sequence: number
  readonly status: string
  readonly packageId?: string | null
  readonly packageDigest: string
  readonly packageVersion: number
  readonly contractVersion?: number | null
  readonly verifiedAt?: string | null
  readonly rejectionReason?: string | null
  readonly createdAt: string
  readonly updatedAt: string
}

export interface DeploymentEnvironmentSummary {
  readonly id: string
  readonly projectId: string
  readonly name: string
  readonly slug: string
  readonly tier: string
  readonly runtimeMode: PublicationDeploymentMode
  readonly region?: string | null
  readonly desiredState: string
  readonly observedState: string
  readonly desiredReleaseId?: string | null
  readonly observedReleaseId?: string | null
  readonly desiredRevision: number
  readonly observedRevision: number
  readonly limits?: DeploymentRuntimeLimits | null
  readonly publicUrl?: string | null
  readonly lastHealthAt?: string | null
  readonly lastError?: string | null
  readonly createdAt: string
  readonly updatedAt: string
}

export interface ReleasePromotionSummary {
  readonly id: string
  readonly projectId: string
  readonly environmentId: string
  readonly fromReleaseId?: string | null
  readonly toReleaseId: string
  readonly rollbackOfId?: string | null
  readonly desiredRevision: number
  readonly status: string
  readonly errorMessage?: string | null
  readonly requestedAt: string
  readonly finishedAt?: string | null
}

export interface DeployedWorkflowProjectState {
  readonly project: DeploymentProjectSummary
  readonly releases: readonly PublicationReleaseSummary[]
  readonly environments: readonly DeploymentEnvironmentSummary[]
  readonly promotions: readonly ReleasePromotionSummary[]
}

export interface DeploymentProjectControlSummary {
  readonly role: DeploymentControlRole
  readonly source: "account" | "project_member"
  readonly canRead: boolean
  readonly canRelease: boolean
  readonly canOperate: boolean
  readonly canManage: boolean
}

export interface DeploymentPortfolioItem {
  readonly project: DeploymentProjectSummary
  readonly control?: DeploymentProjectControlSummary
  readonly defaultEnvironment?: DeploymentEnvironmentSummary | null
  readonly latestRelease?: PublicationReleaseSummary | null
  readonly latestPromotion?: ReleasePromotionSummary | null
  readonly needsAttention: boolean
}

export interface DeploymentProjectsResult {
  readonly projects: readonly DeploymentProjectSummary[]
  readonly portfolio: readonly DeploymentPortfolioItem[]
}

export interface DeploymentProjectResult {
  readonly state: DeployedWorkflowProjectState
}

export interface PublicationReleaseResult {
  readonly release: PublicationReleaseSummary
}

export interface DeploymentEnvironmentResult {
  readonly environment: DeploymentEnvironmentSummary
}

export interface DeploymentInvocationUsageItem {
  readonly invocationId: string
  readonly transport: "http" | "websocket"
  readonly state: "active" | "completed"
  readonly outcome?: "succeeded" | "failed" | "timed_out" | "client_closed" | "upstream_closed" | "interrupted" | null
  readonly statusCode?: number | null
  readonly errorCode?: string | null
  readonly queuedMs: number
  readonly durationMs?: number | null
  readonly requestBytes?: number | null
  readonly responseBytes?: number | null
  readonly usageUnits: number
  readonly startedAt: string
  readonly finishedAt?: string | null
}

export interface DeploymentEnvironmentUsageSummary {
  readonly accountId: string
  readonly projectId: string
  readonly environmentId: string
  readonly deploymentId?: string | null
  readonly generatedAt: string
  readonly dayStartedAt: string
  readonly limits: DeploymentRuntimeLimits
  readonly activeInvocations: number
  readonly invocationsLastMinute: number
  readonly invocationsToday: number
  readonly usageUnitsToday: number
  readonly succeededToday: number
  readonly failedToday: number
  readonly timedOutToday: number
  readonly interruptedToday: number
  readonly averageDurationMs?: number | null
  readonly maximumDurationMs?: number | null
  readonly averageQueuedMs?: number | null
  readonly requestBytesToday: number
  readonly responseBytesToday: number
  readonly recentInvocations: readonly DeploymentInvocationUsageItem[]
}

export interface DeploymentEnvironmentUsageResult {
  readonly usage: DeploymentEnvironmentUsageSummary
}

export interface DeploymentEnvironmentLimitsResult {
  readonly environment: DeploymentEnvironmentSummary
  readonly changed: boolean
  readonly restartRequested: boolean
}

export interface ReleasePromotionResult {
  readonly promotion: ReleasePromotionSummary
  readonly environment: DeploymentEnvironmentSummary
}

export interface DeploymentClaimSummary {
  readonly id: string
  readonly sourceAccountId: string
  readonly sourceProjectId: string
  readonly sourceReleaseId: string
  readonly sourceProjectName: string
  readonly sourceProjectSlug: string
  readonly sourceReleaseSequence: number
  readonly sourcePackageDigest: string
  readonly createdByUserId: string
  readonly targetAccountId?: string | null
  readonly targetEmail?: string | null
  readonly ownershipMode: DeploymentOwnershipMode
  readonly builderRole?: DeploymentControlRole | null
  readonly tokenPrefix: string
  readonly status: "pending" | "accepted" | "revoked" | "expired"
  readonly expiresAt: string
  readonly acceptedByAccountId?: string | null
  readonly acceptedByUserId?: string | null
  readonly acceptedAt?: string | null
  readonly revokedAt?: string | null
  readonly claimedProjectId?: string | null
  readonly createdAt: string
  readonly updatedAt: string
}

export interface DeploymentProjectMemberSummary {
  readonly id: string
  readonly projectId: string
  readonly granteeAccountId: string
  readonly userId: string
  readonly userEmail: string
  readonly userDisplayName?: string | null
  readonly role: DeploymentControlRole
  readonly status: "active" | "revoked"
  readonly grantedByUserId: string
  readonly originClaimId?: string | null
  readonly revokedAt?: string | null
  readonly createdAt: string
  readonly updatedAt: string
}

export interface DeploymentAccessState {
  readonly projectId: string
  readonly projectAccountId: string
  readonly ownershipMode: DeploymentOwnershipMode
  readonly builderAccountId?: string | null
  readonly claims: readonly DeploymentClaimSummary[]
  readonly members: readonly DeploymentProjectMemberSummary[]
}

export interface DeploymentClaimResult {
  readonly claim: DeploymentClaimSummary
}

export interface CreateDeploymentClaimResult extends DeploymentClaimResult {
  readonly claimToken: string
}

export interface AcceptDeploymentClaimResult extends DeploymentClaimResult {
  readonly state: DeployedWorkflowProjectState
}

export interface DeploymentAccessResult {
  readonly access: DeploymentAccessState
}

export type DeploymentCredentialKind = "provider" | "integration"
export type DeploymentCredentialStatus = "connecting" | "ready" | "expiring" | "expired" | "revoked" | "error"
export type DeploymentCredentialReadiness =
  | "ready"
  | "missing"
  | "connecting"
  | "expiring"
  | "expired"
  | "revoked"
  | "error"
  | "incompatible"

export interface DeploymentCredentialProfileSummary {
  readonly id: string
  readonly accountId: string
  readonly kind: DeploymentCredentialKind
  readonly provider?: string | null
  readonly integration?: string | null
  readonly label: string
  readonly accountLabel?: string | null
  readonly version: number
  readonly status: DeploymentCredentialStatus
  readonly statusCode?: string | null
  readonly runnerConnected: boolean
  readonly expiresAt?: string | null
  readonly lastCheckedAt?: string | null
  readonly rotatedAt?: string | null
  readonly revokedAt?: string | null
  readonly purgedAt?: string | null
  readonly createdAt: string
  readonly updatedAt: string
}

export interface DeploymentCredentialJobSummary {
  readonly id: string
  readonly accountId: string
  readonly profileId: string
  readonly type: "connect" | "test" | "rotate" | "revoke" | "purge"
  readonly status: string
  readonly lastError?: string | null
  readonly createdAt: string
  readonly updatedAt: string
}

export interface DeploymentCredentialSlotSummary {
  readonly slotId: string
  readonly kind: DeploymentCredentialKind
  readonly label: string
  readonly provider?: string | null
  readonly integration?: string | null
  readonly required: boolean
  readonly scope: "environment"
  readonly uses: readonly string[]
  readonly testMethod: string
}

export interface DeploymentCredentialBindingSummary {
  readonly id: string
  readonly profileId: string
  readonly version: number
  readonly status: "active" | "revoked"
  readonly profile: DeploymentCredentialProfileSummary
}

export interface DeploymentCredentialSlotState {
  readonly slot: DeploymentCredentialSlotSummary
  readonly readiness: DeploymentCredentialReadiness
  readonly binding?: DeploymentCredentialBindingSummary | null
}

export interface DeploymentEnvironmentCredentialState {
  readonly projectId: string
  readonly environmentId: string
  readonly releaseId: string
  readonly ready: boolean
  readonly slots: readonly DeploymentCredentialSlotState[]
}

export interface DeploymentCredentialProfilesResult {
  readonly profiles: readonly DeploymentCredentialProfileSummary[]
}

export interface DeploymentCredentialProfileResult {
  readonly profile: DeploymentCredentialProfileSummary
  readonly job?: DeploymentCredentialJobSummary | null
}

export interface DeploymentEnvironmentCredentialsResult {
  readonly credentials: DeploymentEnvironmentCredentialState
}

export type DeploymentDomainKind = "default" | "custom"
export type DeploymentDomainStatus = "pending_dns" | "tls_pending" | "ready" | "failed" | "removed"
export type DeploymentDomainDnsStatus = "not_required" | "pending" | "verified" | "failed"
export type DeploymentDomainTlsStatus = "pending" | "ready" | "failed"

export interface DeploymentDomainSummary {
  readonly id: string
  readonly accountId: string
  readonly projectId: string
  readonly environmentId: string
  readonly kind: DeploymentDomainKind
  readonly hostname: string
  readonly publicUrl: string
  readonly status: DeploymentDomainStatus
  readonly dnsStatus: DeploymentDomainDnsStatus
  readonly tlsStatus: DeploymentDomainTlsStatus
  readonly isCanonical: boolean
  readonly redirectToCanonical: boolean
  readonly verificationName?: string | null
  readonly verificationValue?: string | null
  readonly cnameTarget?: string | null
  readonly lastCheckedAt?: string | null
  readonly verifiedAt?: string | null
  readonly activatedAt?: string | null
  readonly removedAt?: string | null
  readonly lastError?: string | null
  readonly createdAt: string
  readonly updatedAt: string
}

export interface DeploymentEnvironmentDomainState {
  readonly projectId: string
  readonly environmentId: string
  readonly canonicalHostname: string
  readonly domains: readonly DeploymentDomainSummary[]
}

export interface DeploymentEnvironmentDomainsResult {
  readonly domains: DeploymentEnvironmentDomainState
}
