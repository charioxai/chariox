export function browserStateCleanupFailure(result) {
  const leaks = []
  if (!result.dockerAvailable) leaks.push("Docker verification unavailable")
  if (!result.containerGone) leaks.push("container")
  if (!result.volumeGone) leaks.push("volume")
  if (!result.savedImageGone) leaks.push("saved image")
  if (result.backupImagesGone === false) leaks.push("backup images")
  if (!result.tempRootRemoved) leaks.push("runtime root")
  if (!result.listenersReleased) {
    leaks.push(`ports ${(result.occupiedPorts ?? []).join(", ") || "unknown"}`)
  }
  return leaks.length === 0
    ? null
    : new Error(`browser state drill cleanup leaked: ${leaks.join("; ")}`)
}
