import { isProjectEnvironmentRequest, projectEnvironmentMinimumProtocolVersion } from "@chariox/kernel-client"
import * as requests from "@chariox/kernel-client/ipc-requests"
import { projectEnvironmentReviewMinimumProtocolVersion } from "@chariox/kernel-client/project-environment-review"
import { projectEnvironmentAdjustmentMinimumProtocolVersion } from "@chariox/kernel-client/project-environment-panel"
import { KernelProtocolMinimumError } from "./protocol-minimum-diagnostic.js"

type FeatureMinimum = { feature: string; minimum: number }

const features: Record<string, FeatureMinimum> = {
  RequestKernelAccess: { feature: "Local-kernel access", minimum: 470 },
  GetProjectEnvironment: { feature: "Project Environment", minimum: requests.projectEnvironmentMinimumProtocolVersion },
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
  if (requests.isGuardedKernelControl(request)) {
    feature = name === "KeepManagedEnvironmentRunning"
      ? { feature: "Managed environment keep running", minimum: requests.managedEnvironmentKeepRunningMinimumProtocolVersion }
      : { feature: "Disposable worker control", minimum: requests.disposableWorkerControlMinimumProtocolVersion }
  } else if (name === "SubmitRoomEnvironmentBrowserAction") {
    const tab = record(payload?.action)?.kind === "tab"
    feature = { feature: tab ? "Room browser tab lifecycle" : "Room browser history", minimum: tab
      ? requests.roomEnvironmentBrowserTabActionsMinimumProtocolVersion : requests.roomEnvironmentBrowserHistoryMinimumProtocolVersion }
  } else if (name === "CreateManagedEnvironment" && payload?.managedRepositoryRoot != null) {
    feature = { feature: "Custom managed repository root", minimum: requests.managedEnvironmentCreateMinimumProtocolVersion }
  } else if ((name === "StartSlice" || name === "StartManagedContextTransfer") && payload?.interactive === true) {
    feature = { feature: "Project export review", minimum: projectEnvironmentReviewMinimumProtocolVersion }
  } else if (name === "RespondToInteraction" && payload?.session_id === "kernel-access") {
    feature = { feature: "Local-kernel access", minimum: payload?.choice_id === "refuse" ? 451 : 470 }
  } else if (name === "RequestNativeProviderTurnInteraction" && payload?.origin != null) {
    feature = { feature: "Native provider approval origin", minimum: requests.nativeProviderInteractionMinimumProtocolVersion }
  } else if ((name === "IssueCloudRelayClientToken" || name === "JoinTerminalPairingLink") && payload?.public_key_thumbprint != null) {
    feature = { feature: "Key-bound terminal connection", minimum: requests.relayClientKeyBindingMinimumProtocolVersion }
  }
  return feature
}

/** Unknown versions retain existing transport compatibility diagnostics. */
export function requireKernelFeatureProtocol(request: unknown, protocolVersion: number | undefined): void {
  if (protocolVersion === undefined || !Number.isInteger(protocolVersion) || protocolVersion < 1) return
  const feature = kernelFeatureMinimum(request)
  if (feature && protocolVersion < feature.minimum) {
    throw new KernelProtocolMinimumError(`kernel too old: ${feature.feature} needs protocol ≥${feature.minimum}; this kernel is ${protocolVersion}; update the kernel`)
  }
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined
}
