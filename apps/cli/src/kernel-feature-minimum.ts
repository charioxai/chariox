import { OWNER_MANAGED_CONTEXT_MINIMUM_PROTOCOL_VERSION } from "@chariox/kernel-client/ipc-managed-context-requests"
import { isProjectEnvironmentRequest, projectEnvironmentMinimumProtocolVersion } from "@chariox/kernel-client"
import * as requests from "@chariox/kernel-client/ipc-requests"
import { projectEnvironmentReviewMinimumProtocolVersion } from "@chariox/kernel-client/project-environment-review"
import { projectEnvironmentAdjustmentMinimumProtocolVersion } from "@chariox/kernel-client/project-environment-panel"
import { KernelProtocolMinimumError } from "./protocol-minimum-diagnostic.js"

type FeatureMinimum = { feature: string; minimum: number }

const features: Record<string, FeatureMinimum> = {
  AddSshMachine: { feature: "SSH machine management", minimum: requests.sshMachineMinimumProtocolVersion },
  RemoveSshMachine: { feature: "SSH machine management", minimum: requests.sshMachineMinimumProtocolVersion },
  GetClientCatalogRevisions: { feature: "Catalog revisions", minimum: requests.clientCatalogRevisionsMinimumProtocolVersion },
  CloudRelayStatus: { feature: "Kernel Cloud ownership", minimum: requests.kernelOwnershipMinimumProtocolVersion },
  StartCloudRelayLogin: { feature: "Kernel Cloud ownership", minimum: requests.kernelOwnershipMinimumProtocolVersion },
  PollCloudRelayLogin: { feature: "Kernel Cloud ownership", minimum: requests.kernelOwnershipMinimumProtocolVersion },
  LogoutCloudRelay: { feature: "Kernel Cloud unlink", minimum: requests.kernelOwnershipMinimumProtocolVersion },
  ConnectCloudRelay: { feature: "Kernel Cloud ownership", minimum: requests.kernelOwnershipMinimumProtocolVersion },
  ResolveKernelClientConnection: { feature: "Key-bound kernel pivot", minimum: requests.kernelOwnershipMinimumProtocolVersion },
  IssueCloudRelayClientToken: { feature: "Kernel terminal delegation", minimum: requests.kernelOwnershipMinimumProtocolVersion },
  PreflightProviderAccountPortability: { feature: "Provider account portability", minimum: requests.providerAccountPortabilityPreflightMinimumProtocolVersion },
  RequestKernelAccess: { feature: "Local-kernel access", minimum: 470 },
  ControlRoomWorkflowRuns: { feature: "Room workflow run controls", minimum: 491 },
  SteerActivePrompt: { feature: "Active prompt steering", minimum: requests.activePromptSteeringMinimumProtocolVersion },
  GetProjectEnvironment: { feature: "Project Environment", minimum: requests.projectEnvironmentMinimumProtocolVersion },
  ...Object.fromEntries(["OpenUserAppView", "ListUserAppViews", "CloseUserAppView", "GetUserAppViewFrontend", "CallUserAppView", "SubscribeUserAppViews", "AnswerUserDomainInteraction"].map(name => [name, { feature: "User-domain App views", minimum: requests.userAppViewsMinimumProtocolVersion }])),
  CaptureVisibleRegion: { feature: "Visible region capture", minimum: requests.visibleRegionCaptureMinimumProtocolVersion },
  KernelBrowser: { feature: "Kernel browser", minimum: requests.kernelBrowserMinimumProtocolVersion },
  AcceptAppHostAction: { feature: "App host actions", minimum: requests.appHostActionMinimumProtocolVersion },
  RevokeAppFileGrants: { feature: "App file revocation", minimum: requests.appFileRevokeMinimumProtocolVersion },
  RestoreAppDataSnapshot: { feature: "App data snapshot restore", minimum: requests.appDataSnapshotRestoreMinimumProtocolVersion },
  GetRoomEnvironmentState: { feature: "Room environment status", minimum: requests.roomEnvironmentStateMinimumProtocolVersion },
  GetRoomEnvironmentResourceInventory: { feature: "Room resource inventory", minimum: requests.roomEnvironmentResourceInventoryMinimumProtocolVersion },
  GetRoomEnvironmentEvents: { feature: "Room event replay", minimum: requests.roomEnvironmentEventReplayMinimumProtocolVersion },
  GetRoomEnvironmentSlice: { feature: "Room slice binding", minimum: requests.roomEnvironmentSliceBindingMinimumProtocolVersion },
  BindRoomEnvironmentSlice: { feature: "Room slice binding", minimum: requests.roomEnvironmentSliceBindingMinimumProtocolVersion },
  GetRoomEnvironmentTabAccessibility: { feature: "Room Tab outline", minimum: requests.roomEnvironmentTabAccessibilityMinimumProtocolVersion },
  ListRoomEnvironmentActionHistory: { feature: "Room action history", minimum: requests.roomEnvironmentActionHistoryMinimumProtocolVersion },
  RequestRoomEnvironmentInputTakeover: { feature: "Room input takeover", minimum: requests.roomEnvironmentInputTakeoverMinimumProtocolVersion },
  ReleaseRoomEnvironmentInput: { feature: "Room input release", minimum: requests.roomEnvironmentInputReleaseMinimumProtocolVersion },
  CancelRoomEnvironmentAction: { feature: "Room action cancellation", minimum: requests.roomEnvironmentActionCancellationMinimumProtocolVersion },
  SetRoomBrowserBar: { feature: "Room browser bar", minimum: requests.roomBrowserBarMinimumProtocolVersion },
  CaptureRoomEnvironmentScreenshot: { feature: "Room screenshot capture", minimum: requests.roomEnvironmentScreenshotMinimumProtocolVersion },
  ReadRoomEnvironmentScreenshotChunk: { feature: "Room screenshot transfer", minimum: requests.roomEnvironmentScreenshotMinimumProtocolVersion },
  StartRoomEnvironment: { feature: "Room environment lifecycle", minimum: requests.roomEnvironmentLifecycleMinimumProtocolVersion },
  StopRoomEnvironment: { feature: "Room environment lifecycle", minimum: requests.roomEnvironmentLifecycleMinimumProtocolVersion },
  RetryRoomEnvironment: { feature: "Room environment lifecycle", minimum: requests.roomEnvironmentLifecycleMinimumProtocolVersion },
  GetManagedEnvironmentReimagePreflight: { feature: "Managed environment reimage preflight", minimum: requests.managedEnvironmentReimagePreflightMinimumProtocolVersion },
  AdjustProjectEnvironment: { feature: "Project environment adjustment", minimum: projectEnvironmentAdjustmentMinimumProtocolVersion },
  GetKernelResourceTelemetry: { feature: "Kernel resource telemetry", minimum: requests.kernelResourceTelemetryMinimumProtocolVersion },
}

/** Identify the feature used by this request, including new optional fields. */
export function kernelFeatureMinimum(request: unknown): FeatureMinimum | undefined {
  const envelope = record(request)
  if (!envelope || Object.keys(envelope).length !== 1) return
  if (isProjectEnvironmentRequest(request)) return { feature: "Project Environment", minimum: projectEnvironmentMinimumProtocolVersion }
  const name = Object.keys(envelope)[0]!
  const payload = record(envelope[name])
  let feature = Object.hasOwn(features, name) ? features[name] : undefined
  if (name === "GetSliceDisplayEndpoint" && payload?.shared_display) {
    feature = { feature: "Room shared display", minimum: requests.kernelDisplayResumeMinimumProtocolVersion }
  } else if (name === "KernelBrowser" && ["display_suspend", "display_resume"].includes(String(record(payload?.command)?.op))) {
    feature = { feature: "Display resumption", minimum: requests.kernelDisplayResumeMinimumProtocolVersion }
  } else if (name === "KernelBrowser" && record(payload?.command)?.op === "computer") {
    feature = { feature: "MP-08 owned Computer desktop", minimum: requests.kernelComputerMinimumProtocolVersion }
  } else if (name === "KernelBrowser" && ["grant_leased_computer", "revoke_leased_computer"].includes(String(record(payload?.command)?.op))) {
    feature = { feature: "MP-08 leased Computer grant", minimum: requests.leasedComputerMinimumProtocolVersion }
  } else if (requests.isGuardedKernelControl(request)) {
    feature = name === "KeepManagedEnvironmentRunning"
      ? { feature: "Managed environment keep running", minimum: requests.managedEnvironmentKeepRunningMinimumProtocolVersion }
      : { feature: "Disposable worker control", minimum: requests.disposableWorkerControlMinimumProtocolVersion }
  } else if (name === "ConfigureAppAutomation" && payload?.delivery_mode === "inject") {
    feature = { feature: "App automation injection", minimum: 491 }
  } else if (name === "CancelActivePrompt" && payload?.expected_prompt_id != null) {
    feature = { feature: "Exact prompt cancellation", minimum: requests.exactPromptCancellationMinimumProtocolVersion }
  } else if (name === "MoveAgentToLocal" && payload?.abort_pending === true) {
    // An older kernel ignores the flag and would Return the agent instead of cancelling its Move.
    feature = { feature: "Move cancellation", minimum: 638 }
  } else if (name === "SubmitRoomEnvironmentBrowserAction") {
    const tab = record(payload?.action)?.kind === "tab"
    feature = { feature: tab ? "Room browser tab lifecycle" : "Room browser history", minimum: tab
      ? requests.roomEnvironmentBrowserTabActionsMinimumProtocolVersion : requests.roomEnvironmentBrowserHistoryMinimumProtocolVersion }
  } else if (name === "CreateManagedEnvironment" && payload?.managedRepositoryRoot != null) {
    feature = { feature: "Custom managed repository root", minimum: requests.managedEnvironmentCreateMinimumProtocolVersion }
  } else if (name === "StartManagedContextTransfer" && payload?.ownerManaged != null) {
    feature = { feature: "Owner context copy", minimum: OWNER_MANAGED_CONTEXT_MINIMUM_PROTOCOL_VERSION }
  } else if ((name === "StartSlice" || name === "StartManagedContextTransfer") && payload?.interactive === true) {
    feature = { feature: "Project export review", minimum: projectEnvironmentReviewMinimumProtocolVersion }
  } else if (name === "RespondToInteraction" && payload?.session_id === "kernel-access") {
    feature = { feature: "Local-kernel access", minimum: payload?.choice_id === "refuse" ? 451 : 470 }
  } else if (name === "RequestNativeProviderTurnInteraction" && payload?.origin != null) {
    feature = { feature: "Native provider approval origin", minimum: requests.nativeProviderInteractionMinimumProtocolVersion }
  } else if ((name === "JoinTerminalPairingLink") && payload?.public_key_thumbprint != null) {
    feature = { feature: "Key-bound terminal connection", minimum: requests.relayClientKeyBindingMinimumProtocolVersion }
  }
  return feature
}

/** Unknown versions retain existing transport compatibility diagnostics. */
export function requireKernelFeatureProtocol(request: unknown, protocolVersion: number | undefined): void {
  const feature = kernelFeatureMinimum(request)
  if (protocolVersion === undefined || !Number.isInteger(protocolVersion) || protocolVersion < 1) {
    if (feature?.feature === "Exact prompt cancellation" || feature?.feature === "App automation injection" || feature?.feature === "Display resumption" || feature?.feature === "Move cancellation") {
      throw new KernelProtocolMinimumError(`kernel support unknown: ${feature.feature} needs protocol ≥${feature.minimum}; reconnect to an updated kernel`)
    }
    return
  }
  if (feature && protocolVersion < feature.minimum) {
    throw new KernelProtocolMinimumError(`kernel too old: ${feature.feature} needs protocol ≥${feature.minimum}; this kernel is ${protocolVersion}; update the kernel`)
  }
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined
}
