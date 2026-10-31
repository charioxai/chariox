import { open, lstat, realpath, unlink } from "node:fs/promises"
import { basename, dirname, isAbsolute, join, parse, relative, resolve, sep } from "node:path"
import { fileURLToPath } from "node:url"

// Writes provider-neutral Path-1 evidence captures outside the repository, private and exclusive.
const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../../..")

function requireValue(condition, message) {
  if (!condition) throw new Error(message)
}

function within(root, target) {
  const path = relative(root, target)
  return path === "" || (path !== ".." && !path.startsWith(`..${sep}`) && !isAbsolute(path))
}

async function assertNoSymlinkPath(directory) {
  const absolute = resolve(directory)
  const root = parse(absolute).root
  let current = root
  for (const component of absolute.slice(root.length).split(sep).filter(Boolean)) {
    current = join(current, component)
    let metadata
    try {
      metadata = await lstat(current)
    } catch {
      throw new Error("output parent path is unavailable")
    }
    requireValue(!metadata.isSymbolicLink(), "output path contains a symlink")
    requireValue(metadata.isDirectory(), "output parent path is not a directory")
  }
  return absolute
}

export async function validateCaptureOutput(file, repositoryRoot = REPOSITORY_ROOT) {
  requireValue(typeof file === "string" && isAbsolute(file), "output path must be absolute")
  const output = resolve(file)
  const parent = await assertNoSymlinkPath(dirname(output))
  const canonicalParent = await realpath(parent)
  const canonicalRoot = await realpath(repositoryRoot)
  requireValue(!within(canonicalRoot, join(canonicalParent, basename(output))),
    "output must be outside the repository")
  try {
    await lstat(output)
    throw new Error("output must be a new file")
  } catch (error) {
    if (error?.message === "output must be a new file") throw error
    if (error?.code !== "ENOENT") throw new Error("output cannot be inspected")
  }
  return output
}

export async function writeCaptureOutput(file, capture, repositoryRoot = REPOSITORY_ROOT) {
  const output = await validateCaptureOutput(file, repositoryRoot)
  const handle = await open(output, "wx", 0o600)
  try {
    await handle.writeFile(`${JSON.stringify(capture, null, 2)}\n`, "utf8")
    await handle.chmod(0o600)
  } catch {
    await handle.close().catch(() => {})
    await unlink(output).catch(() => {})
    throw new Error("capture evidence could not be written")
  }
  await handle.close()
  return output
}
