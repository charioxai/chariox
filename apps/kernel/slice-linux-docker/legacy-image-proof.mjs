import {readProtectedLayoutReceipt, writeProtectedLayoutReceipt} from "./protected-layout-store.mjs"
import {verifyPrivateHostDirectory} from "./protected-host-root.mjs"

function key(image) {
  if (!/^sha256:[a-f0-9]{64}$/.test(image ?? "")) throw new Error("Legacy saved image identity is unavailable")
  return image.slice(7)
}
export function recordLegacyImageProof(root, layout, parent, container, captured) {
  verifyPrivateHostDirectory(root, process.getuid())
  if (container?.Image !== parent?.Id || captured?.Parent !== parent?.Id || captured.Config?.User !== "slice"
      || JSON.stringify(captured.Config?.Env) !== JSON.stringify(container.Config?.Env)
      || !Array.isArray(parent.RootFS?.Layers) || !Array.isArray(captured.RootFS?.Layers)
      || captured.RootFS.Layers.length !== parent.RootFS.Layers.length + 1
      || parent.RootFS.Layers.some((layer, i) => captured.RootFS.Layers[i] !== layer)
      || captured.RootFS.Layers.some(layer => !/^sha256:[a-f0-9]{64}$/.test(layer))) {
    throw new Error("Legacy captured image lineage is unverified")
  }
  const id = key(captured.Id)
  writeProtectedLayoutReceipt(root, id, {version: 1, sliceId: id, imageId: captured.Id,
    ownerContainer: layout.sliceId, homeVolume: layout.homeVolume, homeSource: layout.homeSource})
}
export function requireLegacyImageProof(root, layout, imageId) {
  const proof = readProtectedLayoutReceipt(root, key(imageId))
  if (proof.imageId !== imageId || proof.ownerContainer !== layout.sliceId
      || proof.homeVolume !== layout.homeVolume || proof.homeSource !== layout.homeSource) {
    throw new Error("Legacy saved image belongs to another home lineage")
  }
}
