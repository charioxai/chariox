import assert from "node:assert/strict"
import test from "node:test"
import { issueOwnMachineEnrollmentRequest, ownMachineEnrollmentMinimumProtocolVersion, acceptAppHostActionRequest, appHostActionMinimumProtocolVersion, restoreAppDataSnapshotRequest, startSliceRequest, submitRoomEnvironmentBrowserActionRequest } from "@chariox/kernel-client/ipc-requests"
import { requireKernelFeatureProtocol } from "./kernel-feature-minimum.js"

test("MP-08 MP-10 native active steering requires allocated protocol 491", () => {
  const request = { SteerActivePrompt: {} }
  assert.throws(() => requireKernelFeatureProtocol(request, 482), /Active prompt steering needs protocol ≥491/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol(request, 491))
})

test("protocol-388 kernel gets actionable App host refusal before a request is sent", () => {
  assert.throws(() => requireKernelFeatureProtocol(acceptAppHostActionRequest("s", "o"), 388), {
    message: "kernel too old: App host actions needs protocol ≥409; this kernel is 388; update the kernel",
  })
})

test("minimum and current kernels accept the unchanged host and snapshot request shapes", () => {
  for (const version of [appHostActionMinimumProtocolVersion, 410]) {
    assert.doesNotThrow(() => requireKernelFeatureProtocol(acceptAppHostActionRequest("s", "o"), version))
  }
  assert.throws(() => requireKernelFeatureProtocol(restoreAppDataSnapshotRequest("app", "1", "saved"), 409), /App data snapshot restore needs protocol ≥410; this kernel is 409/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol(restoreAppDataSnapshotRequest("app", "1", "saved"), 410))
})

test("gates browser history and tab lifecycle by their separate existing minima", () => {
  const history = submitRoomEnvironmentBrowserActionRequest("s", 1, "id", { kind: "history", tab_id: "tab", action: "reload" })
  const tab = submitRoomEnvironmentBrowserActionRequest("s", 1, "id", { kind: "tab", tab_id: "tab", action: "close" })
  assert.doesNotThrow(() => requireKernelFeatureProtocol(history, 305))
  assert.throws(() => requireKernelFeatureProtocol(tab, 305), /tab lifecycle needs protocol ≥306; this kernel is 305; update the kernel/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol(tab, 306))
})

test("new optional fields are gated without refusing compatible older shapes", () => {
  assert.doesNotThrow(() => requireKernelFeatureProtocol(startSliceRequest("slice", false), 370))
  assert.throws(() => requireKernelFeatureProtocol({ StartSlice: { interactive: true } }, 370), /needs protocol ≥371; this kernel is 370/)
  assert.throws(() => requireKernelFeatureProtocol({ RequestNativeProviderTurnInteraction: { origin: {} } }, 388), /needs protocol ≥405; this kernel is 388/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol({ JoinTerminalPairingLink: { pairing_link: "pair" } }, 348))
  assert.throws(() => requireKernelFeatureProtocol({ JoinTerminalPairingLink: { pairing_link: "pair", public_key_thumbprint: "thumbprint" } }, 348), /needs protocol ≥349; this kernel is 348/)
})

test("unknown versions and unrelated requests retain existing transport behavior", () => {
  for (const version of [undefined, 0, NaN, -1, 388.5]) {
    assert.doesNotThrow(() => requireKernelFeatureProtocol(acceptAppHostActionRequest("s", "o"), version))
  }
  assert.doesNotThrow(() => requireKernelFeatureProtocol({ ListSessions: null }, 388))
})

test("Cloud owner control and terminal pivots require allocated protocol 491", () => {
  for (const name of ["CloudRelayStatus", "StartCloudRelayLogin", "PollCloudRelayLogin", "LogoutCloudRelay", "ConnectCloudRelay", "IssueCloudRelayClientToken", "ResolveKernelClientConnection", "PreflightProviderAccountPortability"]) {
    assert.throws(() => requireKernelFeatureProtocol({[name]: {}}, 472), /491/)
    assert.throws(() => requireKernelFeatureProtocol({[name]: {}}, 490), /491/)
    assert.doesNotThrow(() => requireKernelFeatureProtocol({[name]: {}}, 491))
  }
})

test("MP-08 / MP-10 / MP-11 kernel-wide access decisions require allocated protocol 470", () => {
  for (const request of [{ RequestKernelAccess: { holder_pid: 42 } },
    { RespondToInteraction: { session_id: "kernel-access", interaction_id: "grant", choice_id: "approve" } }]) {
    assert.throws(() => requireKernelFeatureProtocol(request, 435), /Local-kernel access needs protocol ≥470/)
    assert.throws(() => requireKernelFeatureProtocol(request, 451), /protocol ≥470/)
    assert.doesNotThrow(() => requireKernelFeatureProtocol(request, 470))
  }
  assert.doesNotThrow(() => requireKernelFeatureProtocol({ RespondToInteraction: { session_id: "ordinary" } }, 435))
  assert.doesNotThrow(() => requireKernelFeatureProtocol({ RespondToInteraction: { session_id: "kernel-access", choice_id: "refuse" } }, 451))
})

test("MP-08/MP-11 owner context copy requires its allocated C5 protocol 520", () => {
  const request = { StartManagedContextTransfer: { interactive: true, ownerManaged: {} } }
  assert.throws(() => requireKernelFeatureProtocol(request, 519), /Owner context copy needs protocol ≥520/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol(request, 520))
})

test("integration received-event listing and replay require protocol 911", () => {
  for (const request of [{ ListEventConnectionEvents: { connection_id: "c", limit: 50 } },
    { ReplayEventConnectionEvent: { connection_id: "c", delivery_id: "d" } }]) {
    assert.throws(() => requireKernelFeatureProtocol(request, 910), /Integration received events needs protocol ≥911/)
    assert.doesNotThrow(() => requireKernelFeatureProtocol(request, 911))
  }
})

test("the Environment requests require protocol 982", () => {
  for (const kind of ["GetProjectEnvironment", "SaveProjectEnvironment", "SetProjectEnvironmentSecret", "RunProjectEnvironmentSetup", "ImportProjectEnvironmentEntries", "DraftProjectEnvironment", "RunRemoteProjectEnvironmentSetup", "RemoteEnvironmentOperation", "CheckProjectEnvironment", "ResolveEnvironmentProposal", "GetEnvironmentOperation", "CancelEnvironmentOperation"]) {
    assert.throws(() => requireKernelFeatureProtocol({ [kind]: {} }, 981), /Project Environment needs protocol ≥982/)
    assert.doesNotThrow(() => requireKernelFeatureProtocol({ [kind]: {} }, 982))
  }
})

test("MD-stack user App and browser surfaces require union protocol 443", () => {
  for (const name of ["OpenUserAppView", "ListUserAppViews", "CloseUserAppView", "GetUserAppViewFrontend", "CallUserAppView", "SubscribeUserAppViews", "AnswerUserDomainInteraction"]) {
    assert.throws(() => requireKernelFeatureProtocol({[name]: {}}, 417), /443/)
    assert.doesNotThrow(() => requireKernelFeatureProtocol({[name]: {}}, 443))
  }
  assert.throws(() => requireKernelFeatureProtocol({KernelBrowser: {command:{op:"input"}}},416), /443/)
})

// MP-08 / MP-10 / MP-11: old decoders may ignore an optional cancellation fence.
test("MP-08 MP-10 exact prompt cancellation rejects old and unknown kernels", () => {
  const request = { CancelActivePrompt: { expected_prompt_id: "home-prompt" } }
  for (const version of [undefined, 0, NaN, -1, 490.5, 490, 489]) {
    assert.throws(() => requireKernelFeatureProtocol(request, version), /Exact prompt cancellation needs protocol ≥491/)
  }
  assert.doesNotThrow(() => requireKernelFeatureProtocol(request, 491))
  assert.doesNotThrow(() => requireKernelFeatureProtocol({ CancelActivePrompt: {} }, undefined))
  assert.doesNotThrow(() => requireKernelFeatureProtocol({ CancelActivePrompt: {} }, 490))
})

test("MP-08/MP-10/MP-11 display resumption refuses unknown and pre-525 kernels", () => {
  for (const op of ["display_suspend", "display_resume"]) {
    const request = { KernelBrowser: { command: { op, subscription_id: "old", generation: 7 } } }
    for (const version of [undefined, 0, NaN, 524.5, 517, 519, 524]) {
      assert.throws(() => requireKernelFeatureProtocol(request, version), /Display resumption needs protocol ≥525/)
    }
    assert.doesNotThrow(() => requireKernelFeatureProtocol(request, 525))
  }
})

test("MP-08/MP-10/MP-11 shared Room offers require525 before an older kernel silently opens Selkies",()=>{
 const request={GetSliceDisplayEndpoint:{slice_ref:"s",shared_display:{codecs:["avc1.420033"],bitrate:8000000,device_scale_factor:2}}}
 assert.throws(()=>requireKernelFeatureProtocol(request,524),/Room shared display needs protocol ≥525/)
 assert.doesNotThrow(()=>requireKernelFeatureProtocol(request,525))
})

test("own machine enrollment refuses older kernels before transport", () => {
  const request = issueOwnMachineEnrollmentRequest()
  for (const version of [491, ownMachineEnrollmentMinimumProtocolVersion - 1]) {
    assert.throws(() => requireKernelFeatureProtocol(request, version), /own machine enrollment needs protocol ≥971.*update the kernel/)
  }
  assert.doesNotThrow(() => requireKernelFeatureProtocol(request, ownMachineEnrollmentMinimumProtocolVersion))
})

// An older kernel ignores `abort_pending` and would Return the agent instead of cancelling its Move.
test("MP-08 MP-10 cancelling a Move is refused by kernels older than protocol 638, while a Return is not", () => {
  const cancel = { MoveAgentToLocal: { session_id: "room", agent_ref: "agent", abort_pending: true } }
  for (const version of [undefined, 0, 637]) {
    assert.throws(() => requireKernelFeatureProtocol(cancel, version), /Move cancellation needs protocol ≥638/)
  }
  assert.doesNotThrow(() => requireKernelFeatureProtocol(cancel, 638))
  assert.doesNotThrow(() => requireKernelFeatureProtocol({ MoveAgentToLocal: { session_id: "room", agent_ref: "agent" } }, 600))
})

test("machine acquisition requires the kernel's owner connection support", () => {
  for (const request of [
    { ControlPlane: { action: "capabilities" } },
    { ControlPlane: { action: "acquire", capability_revision: "offer", request_id: "request" } },
    { ControlPlane: { action: "get", allocation_ref: "machine" } },
  ]) {
    assert.throws(() => requireKernelFeatureProtocol(request, 1005), /Machine acquisition needs protocol ≥1006/)
    assert.doesNotThrow(() => requireKernelFeatureProtocol(request, 1006))
  }
})
