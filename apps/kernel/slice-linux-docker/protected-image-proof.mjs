import { spawnSync } from "node:child_process"
import { randomUUID } from "node:crypto"
import { writeProtectedLayoutReceipt, readProtectedLayoutReceipt } from "./protected-layout-store.mjs"
import { verifyPrivateHostDirectory } from "./protected-host-root.mjs"
import { parseRuntimeHash } from "./protected-runtime-proof.mjs"

function refuse() { throw new Error("Slice capture requires a trusted managed runtime build; existing state is preserved") }
function identity(imageId) {
  if (!/^sha256:[a-f0-9]{64}$/.test(imageId)) refuse()
  return imageId.slice(7)
}

// Called by the signed provisioner immediately after its standard Dockerfile
// build. Existing tags, labels, custom extensions and user-supplied images are
// not enough to mint this proof. No image environment or layer data is retained.
export function recordManagedImageProof(root, sourceDigest, inspect, kernelHash) {
  if (!/^sha256:[a-f0-9]{64}$/.test(sourceDigest)) refuse()
  verifyPrivateHostDirectory(root, process.getuid())
  const id = identity(inspect?.Id)
  if (inspect.Config?.User !== "slice" || !Array.isArray(inspect.RootFS?.Layers)) refuse()
  if (inspect.RootFS.Layers.some(layer => !/^sha256:[a-f0-9]{64}$/.test(layer))) refuse()
  if (kernelHash !== undefined && !/^[a-f0-9]{64}$/.test(kernelHash)) refuse()
  writeProtectedLayoutReceipt(root, id, {version: 1, sliceId: id, imageId: inspect.Id, sourceDigest, layers: inspect.RootFS.Layers, kernelHash})
}

export function requireManagedRuntimeHash(root, sourceDigest, imageId) {
  requireManagedImageProof(root, sourceDigest, imageId)
  const proof = readProtectedLayoutReceipt(root, identity(imageId))
  if (!/^[a-f0-9]{64}$/.test(proof.kernelHash ?? "")) refuse()
  return proof.kernelHash
}

export function requireManagedImageProof(root, sourceDigest, imageId) {
  const proof = readProtectedLayoutReceipt(root, identity(imageId))
  if (proof.sourceDigest !== sourceDigest || proof.imageId !== imageId) refuse()
  return imageId
}

// Only the broker's successful, preflighted commit may mint saved-image lineage.
// Mutable tags, image labels and user-supplied image IDs cannot do so.
export function recordCapturedImageProof(root, sourceDigest, parent, container, captured) {
  requireManagedImageProof(root, sourceDigest, parent?.Id)
  const prior = readProtectedLayoutReceipt(root, identity(parent.Id))
  if (container?.Image !== parent.Id || captured?.Parent !== parent.Id
      || captured.Config?.User !== "slice"
      || JSON.stringify(captured.Config?.Env) !== JSON.stringify(container.Config?.Env)
      || JSON.stringify(parent.RootFS?.Layers) !== JSON.stringify(prior.layers)
      || !Array.isArray(captured.RootFS?.Layers)
      || captured.RootFS.Layers.length !== prior.layers.length + 1
      || prior.layers.some((layer, index) => captured.RootFS.Layers[index] !== layer)
      || captured.RootFS.Layers.some(layer => !/^sha256:[a-f0-9]{64}$/.test(layer))) refuse()
  const id = identity(captured.Id)
  writeProtectedLayoutReceipt(root, id, {version: 1, sliceId: id, imageId: captured.Id,
    sourceDigest, parentImageId: parent.Id, layers: captured.RootFS.Layers, kernelHash: prior.kernelHash})
}

// The broker's own flattening of a proven parent's stopped or paused container
// (captured-image-depth.mjs) restarts lineage at one layer with the same runtime.
// `importedImageId` is the ID the helper reported, so a retagged image is never proven.
export function recordFlattenedImageProof(root, sourceDigest, parent, container, flattened, importedImageId) {
  requireManagedImageProof(root, sourceDigest, parent?.Id)
  const prior = readProtectedLayoutReceipt(root, identity(parent.Id))
  if (container?.Image !== parent.Id || flattened?.Id !== importedImageId || flattened?.Parent
      || flattened?.Config?.User !== "slice"
      || JSON.stringify(flattened.Config?.Env) !== JSON.stringify(container.Config?.Env)
      || JSON.stringify(parent.RootFS?.Layers) !== JSON.stringify(prior.layers)
      || !Array.isArray(flattened.RootFS?.Layers) || flattened.RootFS.Layers.length !== 1
      || !/^sha256:[a-f0-9]{64}$/.test(flattened.RootFS.Layers[0])) refuse()
  const id = identity(flattened.Id)
  writeProtectedLayoutReceipt(root, id, {version: 1, sliceId: id, imageId: flattened.Id,
    sourceDigest, parentImageId: parent.Id, layers: flattened.RootFS.Layers, kernelHash: prior.kernelHash})
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
    // Execute only the freshly built immutable, signed-context image with no
    // mounts, networking or persistent writable state. No private inputs exist.
    const helper = `chariox-runtime-proof-${randomUUID()}`
    try {
      const runtime = spawnSync("docker", ["run", "--name", helper, "--rm", "--read-only", "--network", "none",
        "--memory", "64m", "--cpus", "1", "--pids-limit", "16", "--user", "0",
        "--entrypoint", "/usr/bin/sha256sum", images[0].Id, "/opt/chariox-slice/bin/chariox-kernel"], {
        encoding: "utf8", maxBuffer: 1024, timeout: 30_000,
      })
      if (runtime.status !== 0) refuse()
      recordManagedImageProof(root, digest, images[0], parseRuntimeHash(runtime.stdout))
    } finally {
      spawnSync("docker", ["rm", "-f", helper], {stdio: "ignore", timeout: 30_000})
    }
  } catch {
    process.stderr.write("Slice capture build proof could not be recorded; capture remains unavailable\n")
    process.exitCode = 1
  }
}
