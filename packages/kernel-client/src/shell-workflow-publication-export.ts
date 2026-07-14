import { mkdir, writeFile } from "node:fs/promises"
import { dirname, resolve as resolvePath, sep } from "node:path"

import type { WorkflowPublicationPackageFile } from "./kernel-types.js"

export async function writeWorkflowPublicationExportPackage(
  outputRoot: string,
  packageFiles: readonly WorkflowPublicationPackageFile[],
): Promise<string[]> {
  const resolvedRoot = resolvePath(outputRoot)
  await mkdir(resolvedRoot, { recursive: true })
  const paths: string[] = []
  const seen = new Set<string>()
  for (const file of packageFiles) {
    const relativePath = safePackagePath(file.path)
    if (seen.has(relativePath)) {
      throw new Error(`workflow publication package contains duplicate path ${relativePath}`)
    }
    seen.add(relativePath)
    const filePath = resolvePath(resolvedRoot, relativePath)
    if (!filePath.startsWith(`${resolvedRoot}${sep}`)) {
      throw new Error(`workflow publication package path escapes output directory: ${relativePath}`)
    }
    await mkdir(dirname(filePath), { recursive: true })
    await writeFile(filePath, decodeBase64(file.content_base64), {
      mode: file.executable ? 0o755 : 0o644,
    })
    paths.push(filePath)
  }
  return paths
}

function safePackagePath(value: string): string {
  if (
    !value
    || value.startsWith("/")
    || value.includes("\\")
    || value.includes("\0")
    || value.split("/").some((segment) => !segment || segment === "." || segment === "..")
  ) {
    throw new Error(`workflow publication package contains unsafe path ${value}`)
  }
  return value
}

function decodeBase64(value: string): Buffer {
  const normalized = value.replace(/\s+/g, "")
  const decoded = Buffer.from(normalized, "base64")
  if (!normalized || decoded.toString("base64").replace(/=+$/, "") !== normalized.replace(/=+$/, "")) {
    throw new Error("workflow publication package file is not valid base64")
  }
  return decoded
}
