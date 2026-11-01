import { createHash } from "node:crypto"
import { lstat, readdir, readFile, realpath } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

const excludedDirectoryNames = new Set([".git", "node_modules", ".turbo", "dist"])
const excludedRootFiles = new Set([
  ".chariox-release-revision",
  ".chariox-release-source-sha256",
])

export async function releaseSourceDigest(root) {
  const files = []
  await collectFiles(path.resolve(root), "", files)
  const digest = createHash("sha256")
  for (const file of files) {
    const contents = await readFile(file.absolutePath)
    const metadata = JSON.stringify({
      path: file.relativePath,
      executable: file.executable,
      size: contents.length,
      sha256: createHash("sha256").update(contents).digest("hex"),
    })
    digest.update(`${metadata}\n`)
  }
  return digest.digest("hex")
}

async function collectFiles(root, relativeDirectory, files) {
  const directory = path.join(root, relativeDirectory)
  const entries = await readdir(directory, { withFileTypes: true })
  entries.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0)
  for (const entry of entries) {
    if (entry.name.startsWith("._")) continue
    if (entry.isDirectory() && excludedDirectoryNames.has(entry.name)) continue
    if (!relativeDirectory && excludedRootFiles.has(entry.name)) continue
    const relativePath = relativeDirectory
      ? `${relativeDirectory}/${entry.name}`
      : entry.name
    const absolutePath = path.join(root, ...relativePath.split("/"))
    const status = await lstat(absolutePath)
    if (status.isDirectory()) {
      await collectFiles(root, relativePath, files)
    } else if (status.isFile()) {
      files.push({
        absolutePath,
        relativePath,
        executable: (status.mode & 0o111) !== 0,
      })
    } else {
      throw new Error(`release source contains unsupported entry ${relativePath}`)
    }
  }
}

async function main() {
  const root = process.argv[2]
  if (!root) throw new Error("pass the extracted release source directory")
  process.stdout.write(`${await releaseSourceDigest(root)}\n`)
}

if (process.argv[1]
  && await realpath(process.argv[1]) === await realpath(fileURLToPath(import.meta.url))) {
  await main()
}
