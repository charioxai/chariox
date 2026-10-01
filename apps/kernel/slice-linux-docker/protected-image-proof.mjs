import { spawnSync } from "node:child_process"
import { writeProtectedLayoutReceipt, readProtectedLayoutReceipt } from "./protected-layout-store.mjs"
import { verifyPrivateHostDirectory } from "./protected-host-root.mjs"

function refuse() { throw new Error("Slice capture requires a trusted managed runtime build; existing state is preserved") }
function identity(imageId) {
  if (!/^sha256:[a-f0-9]{64}$/.test(imageId)) refuse()
  return imageId.slice(7)
}

// Called by the signed provisioner immediately after its standard Dockerfile
// build. Existing tags, labels, custom extensions and user-supplied images are
// not enough to mint this proof. No image environment or layer data is retained.
export function recordManagedImageProof(root, sourceDigest, inspect) {
  if (!/^sha256:[a-f0-9]{64}$/.test(sourceDigest)) refuse()
  verifyPrivateHostDirectory(root, process.getuid())
  const id = identity(inspect?.Id)
  if (inspect.Config?.User !== "slice" || !Array.isArray(inspect.RootFS?.Layers)) refuse()
  if (inspect.RootFS.Layers.some(layer => !/^sha256:[a-f0-9]{64}$/.test(layer))) refuse()
  writeProtectedLayoutReceipt(root, id, {version: 1, sliceId: id, imageId: inspect.Id, sourceDigest})
}

export function requireManagedImageProof(root, sourceDigest, imageId) {
  const proof = readProtectedLayoutReceipt(root, identity(imageId))
  if (proof.sourceDigest !== sourceDigest || proof.imageId !== imageId) refuse()
  return imageId
}

if (process.argv[2] === "--record-standard-build") {
  try {
    const root = process.env.CHARIOX_SLICE_PROTECTED_IMAGE_PROOF_ROOT
    const digest = process.env.CHARIOX_SLICE_BUILD_CONTEXT_DIGEST
    if (!root || !digest || !process.argv[3]) refuse()
    const result = spawnSync("docker", ["image", "inspect", process.argv[3]], {
      encoding: "utf8", maxBuffer: 1024 * 1024, timeout: 30_000,
    })
    if (result.status !== 0) refuse()
    const images = JSON.parse(result.stdout)
    if (images.length !== 1) refuse()
    recordManagedImageProof(root, digest, images[0])
  } catch {
    process.stderr.write("Slice capture build proof could not be recorded; capture remains unavailable\n")
    process.exitCode = 1
  }
}
