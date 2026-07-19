import { createHash } from "node:crypto"
import { readFile, readdir, stat } from "node:fs/promises"
import { basename, join, relative, resolve, sep } from "node:path"
import { gzipSync } from "node:zlib"

import { pack } from "tar-stream"

import type { WorkflowPublicationDeploymentContract } from "@arroba/kernel-client/workflow-publication-deployment-contract"
import { workflowPublicationPackageDigest } from "@arroba/kernel-client/workflow-publication-package-digest"
import { readPublicationPackageMetadata } from "./publication-deployment-api.js"

const maxArchiveBytes = 64 * 1024 * 1024
const maxFileBytes = 64 * 1024 * 1024
const maxFileCount = 2_048
const maxUncompressedBytes = 128 * 1024 * 1024

interface PackageFile {
  readonly path: string
  readonly content: Buffer
  readonly executable: boolean
}

export interface PreparedPublicationReleasePackage {
  readonly packageId: string
  readonly packageDigest: string
  readonly packageVersion: 3
  readonly contractVersion: 1
  readonly contract: WorkflowPublicationDeploymentContract
  readonly artifact: {
    readonly filename: string
    readonly byteSize: number
    readonly sha256: string
    readonly archiveBase64: string
  }
}

export async function preparePublicationReleasePackage(
  packagePath: string,
): Promise<PreparedPublicationReleasePackage> {
  const metadata = await readPublicationPackageMetadata(packagePath)
  if (metadata.packageVersion !== 3 || !metadata.deploymentContract) {
    throw new Error("deployed workflow releases require a package v3 deployment contract")
  }
  if (requestsPersistentPatch(metadata.agentApp)) {
    throw new Error("Persistent patches are not available for managed Cloud deployments.")
  }
  const files = await packageFiles(metadata.packageRoot)
  const packageId = workflowPublicationPackageDigest(
    files.filter((file) => file.path !== "deployment-contract.json"),
  )
  if (
    metadata.deploymentContract.package_id !== packageId
    || metadata.deploymentContract.artifact.content_digest !== packageId
  ) {
    throw new Error("publication package contents do not match the deployment contract package ID")
  }
  const archive = gzipSync(await tarArchive(files))
  if (archive.byteLength > maxArchiveBytes) {
    throw new Error(`publication package archive exceeds ${maxArchiveBytes} bytes`)
  }
  return {
    packageId,
    packageDigest: workflowPublicationPackageDigest(files),
    packageVersion: 3,
    contractVersion: 1,
    contract: metadata.deploymentContract,
    artifact: {
      filename: `${safeFilename(metadata.publicationAlias ?? metadata.publicationId)}.tar.gz`,
      byteSize: archive.byteLength,
      sha256: sha256(archive),
      archiveBase64: archive.toString("base64"),
    },
  }
}

async function packageFiles(root: string): Promise<PackageFile[]> {
  const files: PackageFile[] = []
  await collectPackageFiles(resolve(root), resolve(root), files)
  if (files.length === 0) throw new Error("publication package does not contain files")
  if (!files.some((file) => file.path === "deployment-contract.json")) {
    throw new Error("publication package is missing deployment-contract.json")
  }
  return files
}

async function collectPackageFiles(root: string, directory: string, files: PackageFile[]): Promise<void> {
  const entries = await readdir(directory, { withFileTypes: true })
  for (const entry of entries) {
    const absolutePath = join(directory, entry.name)
    if (entry.isDirectory()) {
      await collectPackageFiles(root, absolutePath, files)
      continue
    }
    if (!entry.isFile()) {
      throw new Error(`publication package entry ${packagePath(root, absolutePath)} must be a regular file`)
    }
    if (files.length >= maxFileCount) {
      throw new Error(`publication package contains more than ${maxFileCount} files`)
    }
    const fileStat = await stat(absolutePath)
    if (fileStat.size > maxFileBytes) {
      throw new Error(`publication package file ${packagePath(root, absolutePath)} exceeds ${maxFileBytes} bytes`)
    }
    const content = await readFile(absolutePath)
    files.push({
      path: packagePath(root, absolutePath),
      content,
      executable: (fileStat.mode & 0o111) !== 0,
    })
  }
  const totalBytes = files.reduce((sum, file) => sum + file.content.byteLength, 0)
  if (totalBytes > maxUncompressedBytes) {
    throw new Error(`publication package exceeds ${maxUncompressedBytes} uncompressed bytes`)
  }
}

function packagePath(root: string, absolutePath: string): string {
  const path = relative(root, absolutePath).split(sep).join("/")
  if (
    !path
    || path.startsWith("/")
    || path.includes("\\")
    || path.includes("\0")
    || path.split("/").some((segment) => !segment || segment === "." || segment === "..")
  ) {
    throw new Error(`publication package contains unsafe path ${path}`)
  }
  return path
}

async function tarArchive(files: readonly PackageFile[]): Promise<Buffer> {
  const archive = pack()
  const chunks = collectChunks(archive)
  for (const file of [...files].sort(comparePackagePaths)) {
    archive.entry({
      gid: 0,
      mode: file.executable ? 0o755 : 0o644,
      mtime: new Date(0),
      name: file.path,
      size: file.content.byteLength,
      type: "file",
      uid: 0,
    }, file.content)
  }
  archive.finalize()
  return Buffer.concat(await chunks)
}

async function collectChunks(stream: NodeJS.ReadableStream): Promise<Buffer[]> {
  const chunks: Buffer[] = []
  for await (const value of stream) {
    chunks.push(Buffer.isBuffer(value) ? value : Buffer.from(value))
  }
  return chunks
}

function comparePackagePaths(left: PackageFile, right: PackageFile): number {
  return Buffer.compare(Buffer.from(left.path), Buffer.from(right.path))
}

function sha256(value: Uint8Array): string {
  return `sha256:${createHash("sha256").update(value).digest("hex")}`
}

function safeFilename(value: string): string {
  const normalized = basename(value).trim().replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "")
  return normalized || "publication-package"
}

function requestsPersistentPatch(value: unknown): boolean {
  const config = objectRecord(value)
  if (!config) return false
  const persistentPatch = config.persistent_patch ?? config.persistentPatch
  if (persistentPatch === true || objectRecord(persistentPatch)?.enabled === true) return true
  if (!Array.isArray(config.routes)) return false
  return config.routes.some((candidate) => {
    const manipulation = objectRecord(objectRecord(candidate)?.manipulation)
    return manipulation?.level === "persistent_patch"
      || manipulation?.level === "persistentPatch"
      || manipulation?.scope === "persistent"
      || manipulation?.scope === "deployment"
  })
}

function objectRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}
