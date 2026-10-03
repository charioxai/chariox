import { constants, openSync, closeSync, writeFileSync, readFileSync, fsyncSync, lstatSync, renameSync } from "node:fs"
import { join } from "node:path"
import { verifyPrivateHostDirectory } from "./protected-host-root.mjs"

function refuse() { throw new Error("Protected slice layout proof is unavailable; existing identity and saved state are preserved") }
const identifier = value => typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(value)

// Public metadata only. This root is durable host storage, outside Docker homes,
// images, transient broker inputs/outputs, workspace and artifact staging.
export function writeProtectedLayoutReceipt(root, sliceId, receipt) {
  verifyPrivateHostDirectory(root, process.getuid())
  if (!identifier(sliceId) || receipt.sliceId !== sliceId || receipt.version !== 1) refuse()
  const temporary = join(root, `.${sliceId}.${process.pid}.pending`)
  const destination = join(root, `${sliceId}.json`)
  const fd = openSync(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600)
  try { writeFileSync(fd, JSON.stringify(receipt)); fsyncSync(fd) } finally { closeSync(fd) }
  renameSync(temporary, destination)
  const directory = openSync(root, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW)
  try { fsyncSync(directory) } finally { closeSync(directory) }
}

export function readProtectedLayoutReceipt(root, sliceId) {
  verifyPrivateHostDirectory(root, process.getuid())
  if (!identifier(sliceId)) refuse()
  const path = join(root, `${sliceId}.json`)
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW)
  try {
    const metadata = lstatSync(path)
    if (!metadata.isFile() || metadata.nlink !== 1 || metadata.uid !== process.getuid() || (metadata.mode & 0o077) !== 0 || metadata.size > 64 * 1024) refuse()
    const receipt = JSON.parse(readFileSync(fd, "utf8"))
    if (receipt.version !== 1 || receipt.sliceId !== sliceId) refuse()
    return receipt
  } finally { closeSync(fd) }
}

export function requireRetainedRuntimeIdentity(privateRoot, paths, uid) {
  verifyPrivateHostDirectory(privateRoot, uid)
  if (!Array.isArray(paths) || paths.length === 0 || paths.length > 16) refuse()
  for (const relative of paths) {
    if (typeof relative !== "string" || !relative.startsWith("kernel/") || relative.split("/").includes("..") || !(relative.endsWith("/identity.json") || relative === "kernel/kernels/registry.json")) refuse()
    const parts = relative.split("/")
    const filename = parts.pop()
    const directory = join(privateRoot, ...parts)
    verifyPrivateHostDirectory(directory, uid)
    const metadata = lstatSync(join(directory, filename))
    if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.nlink !== 1 || metadata.uid !== uid || (metadata.mode & 0o077) !== 0) refuse()
  }
}
