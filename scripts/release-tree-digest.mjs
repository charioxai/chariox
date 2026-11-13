// Digests of a packaged release file or directory tree, shared by the packagers and the signed-release assembler.
import { createHash } from "node:crypto"
import { createReadStream } from "node:fs"
import { lstat, readdir } from "node:fs/promises"
import { join, relative, sep } from "node:path"

export async function sha256File(path) {
  const hash = createHash("sha256")
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return `sha256:${hash.digest("hex")}`
}

async function updateTreeHash(root, current, hash) {
  for (const entry of await readdir(current, { withFileTypes: true }).then((entries) =>
    entries.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0),
  )) {
    const path = join(current, entry.name)
    const pathFromRoot = relative(root, path).split(sep).join("/")
    const metadata = await lstat(path)
    const mode = metadata.mode & 0o7777
    if (entry.isDirectory()) {
      hash.update(`directory:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:`)
      await updateTreeHash(root, path, hash)
      continue
    }
    if (!entry.isFile()) throw new Error("release directory contains an unsupported file type")
    hash.update(`file:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:${metadata.size}:`)
    for await (const chunk of createReadStream(path)) hash.update(chunk)
  }
}

export async function sha256Tree(root) {
  const metadata = await lstat(root)
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
    throw new Error("release directory must be a directory, not a symlink")
  }
  const hash = createHash("sha256")
  hash.update(`directory:1:.:${metadata.mode & 0o7777}:`)
  await updateTreeHash(root, root, hash)
  return `sha256:${hash.digest("hex")}`
}
