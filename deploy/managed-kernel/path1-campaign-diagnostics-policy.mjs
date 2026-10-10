// MP-07/MP-10/MP-11: the signed upgrade policy admits one observation-only
// campaign setting. No other effective bootstrap override is authorized.
import { constants, closeSync, fstatSync, lstatSync, openSync, readFileSync } from "node:fs"

const [root, unit, paths] = process.argv.slice(2)
const directory = "/etc/systemd/system/chariox-path1-managed-bootstrap.service.d"
const file = `${root}${directory}/path1-campaign-diagnostics.conf`
const expected = Buffer.from("# MP-07/MP-08/MP-10/MP-11: campaign observation, no lifecycle authority.\n[Service]\nEnvironment=CHARIOX_RUNTIME_DIAGNOSTICS_DIR=/home/chariox/.chariox/runtime-diagnostics\n")

let descriptor
try {
  if (root === undefined || unit !== "chariox-path1-managed-bootstrap.service" || paths?.trim() !== file) {
    throw new Error("unauthorized override")
  }
  // Never follow a substituted parent or accept user-writable configuration.
  for (const path of ["/etc", "/etc/systemd", "/etc/systemd/system", directory]) {
    const metadata = lstatSync(`${root}${path}`)
    if (!metadata.isDirectory() || metadata.uid !== 0 || metadata.gid !== 0 || (metadata.mode & 0o022)) {
      throw new Error("unsafe campaign configuration directory")
    }
  }
  descriptor = openSync(file, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
  const metadata = fstatSync(descriptor)
  if (!metadata.isFile() || metadata.uid !== 0 || metadata.gid !== 0
    || metadata.nlink !== 1 || (metadata.mode & 0o7777) !== 0o644 || metadata.size !== expected.length
    || !readFileSync(descriptor).equals(expected)) {
    throw new Error("unsafe campaign configuration")
  }
} catch {
  console.error("MP-07/MP-11 unauthorized Path-1 campaign override")
  process.exitCode = 1
} finally {
  if (descriptor !== undefined) closeSync(descriptor)
}
