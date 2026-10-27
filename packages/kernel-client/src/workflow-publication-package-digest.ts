import { createHash, type Hash } from "node:crypto"

export interface WorkflowPublicationPackageDigestFile {
  readonly path: string
  readonly content: Uint8Array
  readonly executable: boolean
}

export function workflowPublicationPackageDigest(
  files: readonly WorkflowPublicationPackageDigestFile[],
): string {
  const hash = createHash("sha256")
  const canonicalFiles = [...files].sort((left, right) => (
    Buffer.compare(Buffer.from(left.path), Buffer.from(right.path))
  ))
  updateUnsignedLength(hash, canonicalFiles.length)
  for (const file of canonicalFiles) {
    updateLengthFramedBytes(hash, Buffer.from(file.path))
    updateLengthFramedBytes(hash, file.content)
    hash.update(Buffer.from([file.executable ? 1 : 0]))
  }
  return `sha256:${hash.digest("hex")}`
}

function updateLengthFramedBytes(hash: Hash, value: Uint8Array): void {
  updateUnsignedLength(hash, value.byteLength)
  hash.update(value)
}

function updateUnsignedLength(hash: Hash, value: number): void {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new RangeError("Workflow publication package length is invalid")
  }
  const length = Buffer.allocUnsafe(8)
  length.writeBigUInt64BE(BigInt(value))
  hash.update(length)
}
