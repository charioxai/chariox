import { createHash } from "node:crypto"
import { constants as fsConstants } from "node:fs"
import { open, lstat, readFile, readdir, realpath } from "node:fs/promises"
import { isAbsolute, join, resolve } from "node:path"
import { isStateDirectoryExternal } from "./managed-release-settlement.mjs"

const OCI_DIGEST_PATTERN = /^sha256:[a-f0-9]{64}$/
const OCI_INDEX_MEDIA_TYPE = "application/vnd.oci.image.index.v1+json"
const OCI_MANIFEST_MEDIA_TYPE = "application/vnd.oci.image.manifest.v1+json"
const OCI_CONFIG_MEDIA_TYPE = "application/vnd.oci.image.config.v1+json"
const OCI_LAYER_MEDIA_TYPES = new Set([
  "application/vnd.oci.image.layer.v1.tar",
  "application/vnd.oci.image.layer.v1.tar+gzip",
  "application/vnd.oci.image.layer.v1.tar+zstd",
])
const MAX_OCI_JSON_BYTES = 1024 * 1024
const MAX_OCI_BLOB_BYTES = 256 * 1024 * 1024
const MAX_OCI_CLOSURE_BYTES = 512 * 1024 * 1024
const MAX_OCI_LAYERS = 32

function sha256(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`
}

function parseOciJson(bytes, label) {
  try {
    const value = JSON.parse(bytes.toString("utf8"))
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error()
    return value
  } catch {
    throw new Error(`${label} is malformed JSON`)
  }
}

function assertReadOnly(metadata, label, directory) {
  if ((directory && !metadata.isDirectory()) || (!directory && !metadata.isFile()) ||
      (metadata.mode & 0o222) !== 0 || (!directory && metadata.nlink !== 1)) {
    throw new Error(`${label} must be a read-only ${directory ? "directory" : "regular file"}`)
  }
}

async function openReadOnlyRegularFile(path, label, { maximumBytes, beforeOpen } = {}) {
  const before = await lstat(path)
  if (before.isSymbolicLink()) throw new Error(`${label} must not be a symlink`)
  assertReadOnly(before, label, false)
  if (maximumBytes !== undefined && before.size > maximumBytes) throw new Error(`${label} exceeds the size limit`)
  if (beforeOpen) await beforeOpen(path)
  const handle = await open(path, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW | fsConstants.O_NONBLOCK)
  try {
    const opened = await handle.stat()
    if (!opened.isFile()) throw new Error(`${label} changed to a non-regular file while being opened`)
    assertReadOnly(opened, label, false)
    if (opened.dev !== before.dev || opened.ino !== before.ino || opened.size !== before.size) {
      throw new Error(`${label} changed while being opened`)
    }
    return { handle, metadata: opened }
  } catch (error) {
    await handle.close().catch(() => {})
    throw error
  }
}

async function assertPathStillSameFile(path, label, opened) {
  const after = await lstat(path)
  if (after.isSymbolicLink() || !after.isFile() || after.dev !== opened.dev ||
      after.ino !== opened.ino || after.size !== opened.size) {
    throw new Error(`${label} changed while being read`)
  }
}

async function readLayoutFile(path, label, maximumBytes, { beforeOpen } = {}) {
  const { handle, metadata } = await openReadOnlyRegularFile(path, label, { maximumBytes, beforeOpen })
  try {
    const bytes = await handle.readFile()
    await assertPathStillSameFile(path, label, metadata)
    return bytes
  } finally {
    await handle.close()
  }
}

async function hashLayoutBlob(layoutPath, descriptor, label, { retainBytes = false, beforeOpen } = {}) {
  validateOciDescriptor(descriptor, label, new Set([
    OCI_INDEX_MEDIA_TYPE,
    OCI_MANIFEST_MEDIA_TYPE,
    OCI_CONFIG_MEDIA_TYPE,
    ...OCI_LAYER_MEDIA_TYPES,
  ]))
  if (descriptor.size > MAX_OCI_BLOB_BYTES) throw new Error(`${label} exceeds the blob size limit`)
  const digestHex = descriptor.digest.slice("sha256:".length)
  const path = join(layoutPath, "blobs", "sha256", digestHex)
  const { handle, metadata } = await openReadOnlyRegularFile(path, `${label} blob`, {
    maximumBytes: descriptor.size,
    beforeOpen,
  })
  const hash = createHash("sha256")
  const chunks = retainBytes ? [] : null
  let size = 0
  try {
    if (metadata.size !== descriptor.size) throw new Error(`${label} blob size does not match its descriptor`)
    const stream = handle.createReadStream({ autoClose: false })
    for await (const chunk of stream) {
      size += chunk.length
      if (size > descriptor.size || size > MAX_OCI_BLOB_BYTES) throw new Error(`${label} blob exceeds its declared size`)
      hash.update(chunk)
      if (chunks) chunks.push(chunk)
    }
    await assertPathStillSameFile(path, `${label} blob`, metadata)
  } finally {
    await handle.close()
  }
  if (size !== descriptor.size || `sha256:${hash.digest("hex")}` !== descriptor.digest) {
    throw new Error(`${label} blob does not match its content digest`)
  }
  return chunks ? Buffer.concat(chunks, size) : null
}

function validateOciDescriptor(descriptor, label, allowedMediaTypes, { allowAuxiliaryArtifact = false } = {}) {
  if (!descriptor || typeof descriptor !== "object" || Array.isArray(descriptor) ||
      Object.keys(descriptor).some((key) => !["mediaType", "digest", "size", "platform", "annotations", "artifactType"].includes(key)) ||
      Object.hasOwn(descriptor, "urls") || Object.hasOwn(descriptor, "data") ||
      !allowedMediaTypes.has(descriptor.mediaType) || !OCI_DIGEST_PATTERN.test(descriptor.digest ?? "") ||
      !Number.isSafeInteger(descriptor.size) || descriptor.size < 1 || descriptor.size > MAX_OCI_BLOB_BYTES) {
    throw new Error(`${label} descriptor is malformed or uses an unsupported media type`)
  }
  if (descriptor.annotations !== undefined && (!descriptor.annotations || typeof descriptor.annotations !== "object" ||
      Array.isArray(descriptor.annotations) || Object.values(descriptor.annotations).some((value) => typeof value !== "string"))) {
    throw new Error(`${label} descriptor annotations are malformed`)
  }
  if (descriptor.artifactType !== undefined &&
      (!allowAuxiliaryArtifact || descriptor.mediaType !== OCI_MANIFEST_MEDIA_TYPE || descriptor.platform !== undefined ||
        descriptor.artifactType !== "application/vnd.dev.sigstore.bundle.v0.3+json" ||
        !OCI_DIGEST_PATTERN.test(descriptor.annotations?.["io.containerd.manifest.subject"] ?? ""))) {
    throw new Error(`${label} auxiliary artifact descriptor is unsupported or selected as an image`)
  }
  if (descriptor.platform !== undefined) {
    const platform = descriptor.platform
    if (!platform || typeof platform !== "object" || Array.isArray(platform) ||
        Object.keys(platform).some((key) => !["os", "architecture", "variant", "os.version", "os.features", "features"].includes(key)) ||
        typeof platform.os !== "string" || !/^[a-z0-9][a-z0-9._-]{0,63}$/.test(platform.os) ||
        typeof platform.architecture !== "string" || !/^[a-z0-9][a-z0-9._-]{0,63}$/.test(platform.architecture) ||
        (platform.variant !== undefined && (typeof platform.variant !== "string" || !/^[a-z0-9][a-z0-9._-]{0,63}$/.test(platform.variant))) ||
        (platform["os.version"] !== undefined && typeof platform["os.version"] !== "string") ||
        [platform["os.features"], platform.features].some((items) => items !== undefined &&
          (!Array.isArray(items) || items.some((item) => typeof item !== "string" || item.length > 128)))) {
      throw new Error(`${label} descriptor platform is malformed`)
    }
  }
  return descriptor
}

function validateOciIndex(index, label, { allowRootAuxiliaryArtifact = false } = {}) {
  if (index.schemaVersion !== 2 || index.mediaType !== OCI_INDEX_MEDIA_TYPE ||
      !Array.isArray(index.manifests) || index.manifests.length < 1 || index.manifests.length > 128) {
    throw new Error(`${label} is not a bounded OCI image index`)
  }
  if (Object.keys(index).some((key) => !["schemaVersion", "mediaType", "manifests", "annotations"].includes(key)) ||
      (index.annotations !== undefined && (!index.annotations || typeof index.annotations !== "object" ||
        Array.isArray(index.annotations) || Object.values(index.annotations).some((value) => typeof value !== "string")))) {
    throw new Error(`${label} contains unsupported index fields`)
  }
  const seen = new Set()
  for (let position = 0; position < index.manifests.length; position++) {
    const descriptor = validateOciDescriptor(index.manifests[position], `${label} manifest ${position + 1}`, new Set([
      OCI_INDEX_MEDIA_TYPE,
      OCI_MANIFEST_MEDIA_TYPE,
    ]), { allowAuxiliaryArtifact: allowRootAuxiliaryArtifact })
    if (seen.has(descriptor.digest)) throw new Error(`${label} contains a duplicate or ambiguous descriptor`)
    seen.add(descriptor.digest)
    if (descriptor.mediaType === OCI_MANIFEST_MEDIA_TYPE && !descriptor.platform && descriptor.artifactType === undefined) {
      throw new Error(`${label} image manifest descriptor has no platform`)
    }
  }
  return index.manifests
}

async function validateLayoutTree(layoutPath) {
  const root = await lstat(layoutPath)
  if (root.isSymbolicLink()) throw new Error("OCI layout root must not be a symlink")
  assertReadOnly(root, "OCI layout root", true)
  const rootEntries = (await readdir(layoutPath)).sort()
  if (rootEntries.join(",") !== "blobs,index.json,oci-layout") {
    throw new Error("OCI layout root must contain only blobs, index.json, and oci-layout")
  }
  for (const name of ["blobs"]) {
    const metadata = await lstat(join(layoutPath, name))
    if (metadata.isSymbolicLink()) throw new Error("OCI layout blob directory must not be a symlink")
    assertReadOnly(metadata, "OCI layout blob directory", true)
  }
  const algorithmEntries = await readdir(join(layoutPath, "blobs"))
  if (algorithmEntries.length !== 1 || algorithmEntries[0] !== "sha256") {
    throw new Error("OCI layout must contain only sha256 blobs")
  }
  const algorithmDirectory = join(layoutPath, "blobs", "sha256")
  const algorithmMetadata = await lstat(algorithmDirectory)
  if (algorithmMetadata.isSymbolicLink()) throw new Error("OCI sha256 blob directory must not be a symlink")
  assertReadOnly(algorithmMetadata, "OCI sha256 blob directory", true)
  const blobs = await readdir(algorithmDirectory)
  if (blobs.length > 4096 || blobs.some((name) => !/^[a-f0-9]{64}$/.test(name))) {
    throw new Error("OCI layout blob inventory is malformed or exceeds its entry limit")
  }
  let blobInventoryBytes = 0
  for (const name of blobs) {
    const metadata = await lstat(join(algorithmDirectory, name))
    if (metadata.isSymbolicLink()) throw new Error("OCI layout contains a symlink blob")
    assertReadOnly(metadata, "OCI layout blob", false)
    if (metadata.size > MAX_OCI_BLOB_BYTES) throw new Error("OCI layout contains a blob larger than the drill limit")
    blobInventoryBytes += metadata.size
    if (blobInventoryBytes > MAX_OCI_CLOSURE_BYTES) throw new Error("OCI layout blob inventory exceeds the drill transfer limit")
  }
  for (const name of ["index.json", "oci-layout"]) {
    const metadata = await lstat(join(layoutPath, name))
    if (metadata.isSymbolicLink()) throw new Error(`OCI ${name} must not be a symlink`)
    assertReadOnly(metadata, `OCI ${name}`, false)
  }
  return blobInventoryBytes
}

async function verifyOciBaseLayout({ layoutPath, indexDigest, manifestDigest, repository }) {
  if (typeof layoutPath !== "string" || !isAbsolute(layoutPath) || !OCI_DIGEST_PATTERN.test(indexDigest ?? "") ||
      !OCI_DIGEST_PATTERN.test(manifestDigest ?? "")) {
    throw new Error("offline OCI base input is malformed")
  }
  const canonicalLayout = await realpath(layoutPath)
  if (canonicalLayout !== resolve(layoutPath) || !isStateDirectoryExternal(canonicalLayout, repository)) {
    throw new Error("OCI layout path must be canonical and external to the source repository")
  }
  if (/[\s?#@]/.test(canonicalLayout)) throw new Error("OCI layout path contains characters unsupported by the Buildx OCI context URI")
  const layoutBlobBytes = await validateLayoutTree(canonicalLayout)
  const layoutBytes = await readLayoutFile(join(canonicalLayout, "oci-layout"), "OCI layout marker", MAX_OCI_JSON_BYTES)
  const layout = parseOciJson(layoutBytes, "OCI layout marker")
  if (layout.imageLayoutVersion !== "1.0.0" || Object.keys(layout).some((key) => key !== "imageLayoutVersion")) {
    throw new Error("OCI layout marker does not declare the supported image-layout version")
  }
  const rootIndexBytes = await readLayoutFile(join(canonicalLayout, "index.json"), "OCI root index", MAX_OCI_JSON_BYTES)
  const rootIndex = parseOciJson(rootIndexBytes, "OCI root index")
  const rootIndexDigest = sha256(rootIndexBytes)
  const rootDescriptors = validateOciIndex(rootIndex, "OCI root index", { allowRootAuxiliaryArtifact: true })
  const rootMatchesDigest = rootIndexDigest === indexDigest
  const indexDescriptors = rootDescriptors.filter((descriptor) => descriptor.digest === indexDigest)
  if (rootMatchesDigest === (indexDescriptors.length > 0) || indexDescriptors.length > 1) {
    throw new Error("OCI index digest is ambiguous or does not identify a verified index")
  }

  let pinnedIndex = rootIndex
  let pinnedIndexBytes = rootIndexBytes
  if (!rootMatchesDigest) {
    const [descriptor] = indexDescriptors
    if (descriptor.mediaType !== OCI_INDEX_MEDIA_TYPE) {
      throw new Error("pinned OCI index digest points to a non-index descriptor")
    }
    if (descriptor.size > MAX_OCI_JSON_BYTES) throw new Error("pinned OCI index exceeds the JSON size limit")
    pinnedIndexBytes = await hashLayoutBlob(canonicalLayout, descriptor, "pinned OCI index", { retainBytes: true })
    pinnedIndex = parseOciJson(pinnedIndexBytes, "pinned OCI index blob")
    validateOciIndex(pinnedIndex, "pinned OCI index")
  }
  if (pinnedIndexBytes.length > MAX_OCI_JSON_BYTES) throw new Error("pinned OCI index exceeds the JSON size limit")
  if (sha256(pinnedIndexBytes) !== indexDigest) throw new Error("pinned OCI index blob digest does not match the supplied digest")

  const pinnedDescriptors = validateOciIndex(pinnedIndex, "pinned OCI index")
  if (pinnedDescriptors.some((descriptor) => descriptor.mediaType !== OCI_MANIFEST_MEDIA_TYPE)) {
    throw new Error("pinned OCI index contains a nested or unsupported descriptor")
  }
  const candidates = pinnedDescriptors.filter((descriptor) =>
    descriptor.platform?.os === "linux" && descriptor.platform?.architecture === "amd64")
  if (candidates.length !== 1 || candidates[0].digest !== manifestDigest) {
    throw new Error("pinned OCI index does not identify exactly the requested linux/amd64 manifest")
  }
  const selectedManifest = candidates[0]
  if (selectedManifest.size > MAX_OCI_JSON_BYTES) throw new Error("OCI image manifest exceeds the JSON size limit")
  const manifestBytes = await hashLayoutBlob(canonicalLayout, selectedManifest, "linux/amd64 image manifest", { retainBytes: true })
  if (manifestBytes.length > MAX_OCI_JSON_BYTES) throw new Error("OCI image manifest exceeds the JSON size limit")
  const manifest = parseOciJson(manifestBytes, "OCI image manifest")
  if (manifest.schemaVersion !== 2 || manifest.mediaType !== OCI_MANIFEST_MEDIA_TYPE ||
      !manifest.config || !Array.isArray(manifest.layers) || manifest.layers.length > MAX_OCI_LAYERS ||
      Object.keys(manifest).some((key) => !["schemaVersion", "mediaType", "config", "layers", "annotations", "artifactType", "subject"].includes(key)) ||
      manifest.artifactType !== undefined || manifest.subject !== undefined) {
    throw new Error("OCI image manifest has unsupported or malformed fields")
  }
  const configDescriptor = validateOciDescriptor(manifest.config, "OCI image config", new Set([OCI_CONFIG_MEDIA_TYPE]))
  if (configDescriptor.platform !== undefined) throw new Error("OCI config descriptor must not declare a platform")
  if (configDescriptor.size > MAX_OCI_JSON_BYTES) throw new Error("OCI image config exceeds the JSON size limit")
  let closureBytes = pinnedIndexBytes.length + selectedManifest.size + configDescriptor.size
  if (closureBytes > MAX_OCI_CLOSURE_BYTES) throw new Error("OCI base closure exceeds the size limit")
  const configBytes = await hashLayoutBlob(canonicalLayout, configDescriptor, "OCI image config", { retainBytes: true })
  if (configBytes.length > MAX_OCI_JSON_BYTES) throw new Error("OCI image config exceeds the JSON size limit")
  const config = parseOciJson(configBytes, "OCI image config")
  const layerDescriptors = manifest.layers.map((descriptor, index) =>
    validateOciDescriptor(descriptor, `OCI layer ${index + 1}`, OCI_LAYER_MEDIA_TYPES))
  if (layerDescriptors.some((descriptor) => descriptor.platform !== undefined)) {
    throw new Error("OCI layer descriptors must not declare a platform")
  }
  if (!config.rootfs || config.rootfs.type !== "layers" || !Array.isArray(config.rootfs.diff_ids) ||
      config.rootfs.diff_ids.length !== layerDescriptors.length ||
      config.rootfs.diff_ids.some((digest) => !OCI_DIGEST_PATTERN.test(digest)) ||
      config.os !== "linux" || config.architecture !== "amd64") {
    throw new Error("OCI config does not match the selected linux/amd64 layer closure")
  }
  const layers = []
  for (let index = 0; index < layerDescriptors.length; index++) {
    const descriptor = layerDescriptors[index]
    closureBytes += descriptor.size
    if (closureBytes > MAX_OCI_CLOSURE_BYTES) throw new Error("OCI base closure exceeds the size limit")
    await hashLayoutBlob(canonicalLayout, descriptor, `OCI layer ${index + 1}`)
    layers.push({ digest: descriptor.digest, mediaType: descriptor.mediaType, size: descriptor.size })
  }
  return {
    mode: "oci-layout",
    layoutPath: canonicalLayout,
    sourceContext: `oci-layout://${canonicalLayout}@${manifestDigest}`,
    indexDigest,
    rootIndexDigest,
    manifestDigest,
    platform: "linux/amd64",
    config: { digest: configDescriptor.digest, size: configDescriptor.size },
    layers,
    closureBytes,
    layoutBlobBytes,
  }
}

export {
  hashLayoutBlob,
  readLayoutFile,
  verifyOciBaseLayout,
}
