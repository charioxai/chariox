export type DeploymentProjectKind = "workflow_endpoint" | "agent_app"
export type PublicationDeploymentMode = "local_runtime" | "hosted_container"

export interface DeploymentProjectSummary {
  readonly id: string
  readonly accountId: string
  readonly name: string
  readonly slug: string
  readonly kind: DeploymentProjectKind
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

export interface DeploymentPortfolioItem {
  readonly project: DeploymentProjectSummary
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

export interface ReleasePromotionResult {
  readonly promotion: ReleasePromotionSummary
  readonly environment: DeploymentEnvironmentSummary
}
