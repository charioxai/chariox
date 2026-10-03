export function browserStateCleanupFailure(result) {
  const leaks = []
  if (!result.dockerAvailable) leaks.push("Docker verification unavailable")
  if (!result.containerGone) leaks.push("container")
  if (result.fixtureSidecarGone === false) leaks.push("fixture sidecar")
  if (!result.volumeGone) leaks.push("volume")
  if (!result.savedImageGone) leaks.push("saved image")
  if (result.backupImagesGone === false) leaks.push("backup images")
  if (result.rollbackImagesUnresolved?.length) leaks.push(`unresolved rollback images ${result.rollbackImagesUnresolved.join(", ")}`)
  if (result.rollbackImageInventoryFailed) leaks.push("rollback image inventory unavailable")
  if (result.stateImageInventoryFailed) leaks.push("state image inventory unavailable")
  if (result.fixtureWorkspaceRemoved === false) leaks.push("fixture workspace")
  if (!result.tempRootRemoved) leaks.push("runtime root")
  if (!result.listenersReleased) {
    leaks.push(`ports ${(result.occupiedPorts ?? []).join(", ") || "unknown"}`)
  }
  return leaks.length === 0
    ? null
    : new Error(`browser state drill cleanup leaked: ${leaks.join("; ")}`)
}

export async function cleanupBrowserStateImages({ imageRefs, imagesBefore, slice, inspectImage, removeImage }) {
  const candidates = [...new Set(imageRefs)].filter(imageRef => !imagesBefore.has(imageRef))
  const result = { removedImageRefs: [], preservedImageRefs: [], unresolvedImageRefs: [] }
  const ownership = {
    "io.chariox.slice.id": slice?.id,
    "io.chariox.slice.owner-kernel-id": slice?.owner_kernel_id,
    "io.chariox.slice.owner-machine-id": slice?.owner_machine_id,
  }
  if (!Object.values(ownership).every(value => typeof value === "string" && value.trim())) {
    result.unresolvedImageRefs.push(...candidates)
    return result
  }
  for (const imageRef of candidates) {
    try {
      const image = await inspectImage(imageRef)
      if (!image) continue
      if (!Object.entries(ownership).every(([key, value]) => image.Config?.Labels?.[key] === value)) {
        result.preservedImageRefs.push(imageRef)
        continue
      }
      if (!/^sha256:[a-f0-9]{64}$/.test(image.Id ?? "")) {
        result.unresolvedImageRefs.push(imageRef)
        continue
      }
      // The tag may move after inspection. Remove only the owned immutable ID.
      await removeImage(image.Id)
      if (await inspectImage(image.Id)) result.unresolvedImageRefs.push(imageRef)
      else result.removedImageRefs.push(imageRef)
    } catch {
      result.unresolvedImageRefs.push(imageRef)
    }
  }
  return result
}
