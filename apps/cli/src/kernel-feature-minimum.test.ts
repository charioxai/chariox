import assert from "node:assert/strict"
import test from "node:test"
import { acceptAppHostActionRequest, appHostActionMinimumProtocolVersion, restoreAppDataSnapshotRequest, startSliceRequest, submitRoomEnvironmentBrowserActionRequest } from "@chariox/kernel-client/ipc-requests"
import { requireKernelFeatureProtocol } from "./kernel-feature-minimum.js"

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
  const oldRequest = { CreateManagedEnvironment: {} }
  assert.doesNotThrow(() => requireKernelFeatureProtocol(oldRequest, 341))
  assert.throws(() => requireKernelFeatureProtocol({ CreateManagedEnvironment: { managedRepositoryRoot: "/project" } }, 341), /needs protocol ≥342; this kernel is 341/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol(startSliceRequest("slice", false), 370))
  assert.throws(() => requireKernelFeatureProtocol({ StartSlice: { interactive: true } }, 370), /needs protocol ≥371; this kernel is 370/)
  assert.throws(() => requireKernelFeatureProtocol({ RequestNativeProviderTurnInteraction: { origin: {} } }, 388), /needs protocol ≥405; this kernel is 388/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol({ JoinTerminalPairingLink: { pairing_link: "pair" } }, 348))
  assert.throws(() => requireKernelFeatureProtocol({ JoinTerminalPairingLink: { pairing_link: "pair", public_key_thumbprint: "thumbprint" } }, 348), /needs protocol ≥349; this kernel is 348/)
  assert.throws(() => requireKernelFeatureProtocol({ GetDisposableWorker: { homeKernelId: "k", allocationId: "a" } }, 366), /Disposable worker control needs protocol ≥367; this kernel is 366/)
  assert.throws(() => requireKernelFeatureProtocol({ KeepManagedEnvironmentRunning: { environmentId: "e" } }, 366), /Managed environment keep running needs protocol ≥367; this kernel is 366/)
})

test("unknown versions and unrelated requests retain existing transport behavior", () => {
  for (const version of [undefined, 0, NaN, -1, 388.5]) {
    assert.doesNotThrow(() => requireKernelFeatureProtocol(acceptAppHostActionRequest("s", "o"), version))
  }
  assert.doesNotThrow(() => requireKernelFeatureProtocol({ ListSessions: null }, 388))
})


test("MD-stack MP-08 / MP-10 / MP-11 user App views require protocol 494", () => {
  for (const name of ["OpenUserAppView", "ListUserAppViews", "CloseUserAppView", "GetUserAppViewFrontend", "CallUserAppView", "SubscribeUserAppViews", "AnswerUserDomainInteraction"]) {
    for (const predecessor of [417, 472, 487, 493]) assert.throws(() => requireKernelFeatureProtocol({[name]: {}}, predecessor), /494/)
    assert.doesNotThrow(() => requireKernelFeatureProtocol({[name]: {}}, 494))
  }
  assert.throws(() => requireKernelFeatureProtocol({KernelBrowser: {command:{op:"input"}}},416), /494/)
})

// MP-08 / MP-10 / MP-11: main protocol 487 has no KernelBrowser union.
test("MP-08 / MP-10 / MP-11 Browser and Computer refuse the actual main predecessors", () => {
  for (const command of [{op:"state"}, {op:"computer",command:{op:"state"}}, {op:"revoke_grants",agent_id:null}]) {
    const request = {KernelBrowser:{command}}
    for (const predecessor of [472, 487, 493]) {
      assert.throws(() => requireKernelFeatureProtocol(request, predecessor), /needs protocol ≥494/)
    }
    assert.doesNotThrow(() => requireKernelFeatureProtocol(request,494))
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

test("MP-08 / MP-10 / MP-11 typed leased desktop grants require allocated protocol 500", () => {
  for (const command of [{op:"grant_leased_computer",agent_id:"a",target:{surface_id:"s",generation:"g"}}, {op:"revoke_leased_computer",agent_id:"a"}]) {
    for (const predecessor of [487, 494, 499]) assert.throws(() => requireKernelFeatureProtocol({KernelBrowser:{command}}, predecessor), /protocol ≥500/)
    assert.doesNotThrow(() => requireKernelFeatureProtocol({KernelBrowser:{command}}, 500))
  }
})

// MP-08 / MP-10 / MP-11: actual predecessors lack this request variant.
test("MP-08 visible-region capture refuses protocol 487 and 493 before transport", () => {
  const request = {CaptureVisibleRegion: {}}
  for (const version of [487, 493]) assert.throws(() => requireKernelFeatureProtocol(request, version), /Visible region capture needs protocol ≥494/)
  assert.doesNotThrow(() => requireKernelFeatureProtocol(request, 494))
})
