export type DeploymentProjectKind = "workflow_endpoint" | "agent_app"
export type PublicationDeploymentMode = "local_runtime" | "hosted_container"
export type DeploymentOwnershipMode = "customer_owned" | "builder_managed" | "internal_team"
export type DeploymentControlRole = "owner" | "admin" | "deployer" | "operator" | "viewer" | "billing" | "maintainer"

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
