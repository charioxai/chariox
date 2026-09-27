export function roomCompanionClickFixture(realProvider) {
  return {
    path: "/click",
    readyMarker: "POINTER_CLICK_READY",
    // Provider click/form completion contributes one click before the human
    // takes over. Office tasks have a separate physical-effect contract.
    pointerClickExpectedCount: realProvider && realProvider.computerTask !== "office" ? 2 : 1,
  }
}
