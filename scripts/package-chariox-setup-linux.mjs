#!/usr/bin/env node
// MP-07 / MP-08 / MP-11: unsigned generic .deb; no scripts, credentials or kernel service.
import { spawnSync } from "node:child_process"
import { copyFile, chmod, lstat, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

export async function packageSetupLinux(binary, output, version) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version ?? "")) throw new Error("public Setup version required")
  const meta = await lstat(binary)
  if (!meta.isFile() || meta.isSymbolicLink()) throw new Error("regular unsigned Setup executable required")
  const scratch = await mkdtemp(join(tmpdir(), "chariox-setup-deb-"))
  try {
    await mkdir(join(scratch, "DEBIAN")); await mkdir(join(scratch, "usr/bin"), { recursive: true })
    await copyFile(binary, join(scratch, "usr/bin/chariox-setup")); await chmod(join(scratch, "usr/bin/chariox-setup"), 0o755)
    await writeFile(join(scratch, "DEBIAN/control"), `Package: chariox-setup\nVersion: ${version}\nArchitecture: amd64\nMaintainer: Chariox <support@chariox.com>\nDepends: python3, libc6 (>= 2.35)\nSection: utils\nPriority: optional\nDescription: Chariox per-user kernel setup\n Generic installer; run as the intended user to approve and set up a kernel.\n`)
    await mkdir(dirname(resolve(output)), { recursive: true })
    if (spawnSync("dpkg-deb", ["--root-owner-group", "--build", scratch, resolve(output)], { stdio: "ignore" }).status !== 0) throw new Error("unsigned Setup .deb packaging failed")
  } finally { await rm(scratch, { recursive: true, force: true }) }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 5) throw new Error("Setup executable, unsigned .deb and version required")
  await packageSetupLinux(...process.argv.slice(2))
}
