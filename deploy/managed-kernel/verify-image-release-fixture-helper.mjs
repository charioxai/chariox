import { chmod, lstat, mkdir, readFile, writeFile } from "node:fs/promises"
import { dirname, join, relative, sep } from "node:path"
import { fileURLToPath } from "node:url"
import { spawnSync } from "node:child_process"

const repositoryRoot = fileURLToPath(new URL("../../", import.meta.url))
// Mirrors SLICE_BUILD_CONTEXT_SOURCES in scripts/package-managed-kernel-release.mjs.
const SLICE_BUILD_CONTEXT_SOURCES = [
  "Cargo.toml",
  "Cargo.lock",
  "deploy/local-linux/provision-docker-admission-locks.py",
  "deploy/managed-kernel/chariox-docker-admission-locks.service",
  "adapters/rust",
  "apps/aegs-dummy",
  "apps/browser-session-import",
  "apps/kernel",
  "apps/relay",
  // Signed upgrade tooling: an installed release updates itself with its own scripts.
  "deploy/managed-kernel",
  "examples/workflow-code",
  "packages/aegs-sdk",
  "packages/app-package",
  "packages/app-runtime",
  "packages/app-sdk",
  "packages/event-protocol",
]
const SLICE_BUILD_CONTEXT_PATH = "/usr/lib/chariox/slice-build-context"
const RELEASE_SOURCE_ARTIFACTS = new Map([
  ["chariox-managed-bootstrap.service", "deploy/managed-kernel/chariox-managed-bootstrap.service"],
  ["chariox-path1-managed-bootstrap.service", "deploy/managed-kernel/chariox-path1-managed-bootstrap.service"],
  ["chariox-disposable-worker-bootstrap.service", "deploy/managed-kernel/chariox-disposable-worker-bootstrap.service"],
  ["chariox-rootless-docker.service", "deploy/managed-kernel/chariox-rootless-docker.service"],
  ["chariox-slice-broker.service", "deploy/managed-kernel/chariox-slice-broker.service"],
  ["chariox-data-volume-admission.service", "apps/kernel/slice-linux-docker/chariox-data-volume-admission.service"],
  ["chariox-rootless-docker.path1-data-volume.conf", "apps/kernel/slice-linux-docker/chariox-rootless-docker.path1-data-volume.conf"],
  ["chariox-slice-disk-quota-allocator.path1-data-volume.conf", "apps/kernel/slice-linux-docker/chariox-slice-disk-quota-allocator.path1-data-volume.conf"],
])
const EXECUTABLE_CONTEXT_FILES = new Set([
  "apps/kernel/slice-linux-docker/prebuilt/chariox-kernel",
  "apps/kernel/slice-linux-docker/prebuilt/chariox-relay",
  "apps/kernel/slice-linux-docker/enter-rootless-docker-namespace.sh",
  "apps/kernel/slice-linux-docker/managed-rootless-service.sh",
  "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
  "apps/kernel/slice-linux-docker/managed-publication-access.sh",
])

export const PATH1_DATA_VOLUME_ARTIFACTS = Object.freeze([
  Object.freeze({
    name: "chariox-data-volume-admission.service",
    path: "/etc/systemd/system/chariox-data-volume-admission.service",
  }),
  Object.freeze({
    name: "chariox-rootless-docker.path1-data-volume.conf",
    path: "/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf",
  }),
  Object.freeze({
    name: "chariox-slice-disk-quota-allocator.path1-data-volume.conf",
    path: "/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf",
  }),
])

async function ensureDirectory(root, directory) {
  await mkdir(root, { recursive: true, mode: 0o755 })
  await chmod(root, 0o755)
  const relativeDirectory = relative(root, directory)
  let current = root
  for (const segment of relativeDirectory.split(sep).filter(Boolean)) {
    current = join(current, segment)
    await mkdir(current, { recursive: true, mode: 0o755 })
    await chmod(current, 0o755)
  }
}

async function copyContextFile(treeRoot, relativePath) {
  if (relativePath.startsWith("/") || relativePath.split("/").includes("..")) {
    throw new Error(`invalid signed release fixture source path: ${relativePath}`)
  }
  const sourcePath = join(repositoryRoot, relativePath)
  const metadata = await lstat(sourcePath)
  if (!metadata.isFile()) throw new Error(`signed release fixture source is not a regular file: ${relativePath}`)
  const destination = join(treeRoot, relativePath)
  await ensureDirectory(treeRoot, dirname(destination))
  const mode = EXECUTABLE_CONTEXT_FILES.has(relativePath) ? 0o755 : 0o644
  await writeFile(destination, await readFile(sourcePath), { mode })
  await chmod(destination, mode)
}

export async function stageReleaseFixtureSourceAssets(rootfs) {
  const listing = spawnSync(
    "git",
    ["ls-files", "-z", "--", ...SLICE_BUILD_CONTEXT_SOURCES],
    { cwd: repositoryRoot, maxBuffer: 128 * 1024 * 1024 },
  )
  if (listing.status !== 0) {
    throw new Error(`signed release fixture source tree cannot be listed: ${listing.stderr.toString().trim()}`)
  }

  const treeRoot = join(rootfs, SLICE_BUILD_CONTEXT_PATH.slice(1))
  await ensureDirectory(treeRoot, treeRoot)
  for (const relativePath of listing.stdout.toString("utf8").split("\0").filter(Boolean)) {
    await copyContextFile(treeRoot, relativePath)
  }

  const contentByName = new Map()
  for (const [name, relativePath] of RELEASE_SOURCE_ARTIFACTS) {
    const sourcePath = join(repositoryRoot, relativePath)
    const metadata = await lstat(sourcePath)
    if (!metadata.isFile()) throw new Error(`signed release fixture artifact is not a regular file: ${relativePath}`)
    contentByName.set(name, await readFile(sourcePath))
  }
  return contentByName
}
