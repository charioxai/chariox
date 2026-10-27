import { mkdtemp, realpath } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

export async function makePrivateTestDirectory(prefix) {
  const canonicalTempRoot = await realpath(os.tmpdir())
  return mkdtemp(path.join(canonicalTempRoot, prefix))
}
