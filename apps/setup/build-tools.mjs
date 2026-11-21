import { createHash } from "node:crypto"
import { createReadStream } from "node:fs"
import { lstat, mkdir, mkdtemp, readlink, rename, rm, symlink, writeFile } from "node:fs/promises"
import { basename, join } from "node:path"
import { pathToFileURL } from "node:url"
import { download } from "./download.mjs"
import { command, regular, tree } from "./machine/remote.mjs"
import toolchain from "./build-tools-linux-x64.lock.json" with { type: "json" }

const runner = `#!/bin/sh
set -eu
prefix=$(dirname "$(readlink -f -- "$0")")/env
export PATH="$prefix/bin:$PATH"
export TMPDIR="$prefix/tmp"
export CC="$prefix/bin/x86_64-conda-linux-gnu-gcc"
export CXX="$prefix/bin/x86_64-conda-linux-gnu-g++"
export AR="$prefix/bin/x86_64-conda-linux-gnu-ar"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$CC"
export LIBCLANG_PATH="$prefix/lib"
export PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig:$prefix/share/pkgconfig"
if [ ! -x "$prefix/bin/cargo" ] || [ ! -x "$CC" ]; then
  printf '%s\\n' "Build tools are incomplete at $prefix. Move its parent directory aside and rerun Setup with --build-tools." >&2
  exit 1
fi
exec "$@"
`

const absent = error => error.code === "ENOENT" ? null : Promise.reject(error)

async function verifiedArchive(pin, directory) {
  const name = basename(new URL(pin.url).pathname)
  const path = join(directory, name)
  await download(pin.url, path, pin.size)
  const hash = createHash("sha256")
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  if (hash.digest("hex") !== pin.sha256) throw new Error(`SHA-256 mismatch for ${name}`)
  return path
}

async function preparePrefix(store, id, pin) {
  const root = join(store, id)
  const prefix = join(root, "env")
  const run = join(root, "run")
  if (await lstat(root).catch(absent)) {
    await tree(store, id, false)
    if ((await regular(run).catch(absent))?.toString() === runner) return run
    throw new Error(`Incomplete build tools at ${root}. Move that directory aside and retry Setup.`)
  }

  await mkdir(root, { mode: 0o700 })
  try {
    const scratch = await mkdtemp(join(root, ".install-"))
    const env = { HOME: scratch, PATH: "/usr/bin:/bin", TMPDIR: scratch }
    const archive = await verifiedArchive(pin.micromamba, scratch)
    await command("tar", ["-xjf", archive, "--no-same-owner", "--no-same-permissions", "-C", scratch, "bin/micromamba"], false, undefined, env)

    // Use a fresh, offline package cache so micromamba only sees archives that Setup verified.
    const spec = ["@EXPLICIT"]
    for (const item of pin.packages) {
      const path = await verifiedArchive(item, scratch)
      spec.push(`${pathToFileURL(path).href}#${item.sha256}`)
    }
    const input = join(scratch, "packages.txt")
    await writeFile(input, spec.join("\n") + "\n", { mode: 0o600 })
    await command(join(scratch, "bin/micromamba"), [
      "--no-rc", "--no-env", "--root-prefix", join(scratch, "cache"),
      "create", "--offline", "--yes", "--prefix", prefix, "--file", input,
    ], false, undefined, env)

    await mkdir(join(prefix, "tmp"), { mode: 0o700 })
    await rm(scratch, { recursive: true })
    await writeFile(run, runner, { flag: "wx", mode: 0o700 })
    return run
  } catch (error) {
    await rm(root, { recursive: true, force: true })
    throw new Error(`Could not install build tools: ${error.message}. Retry Setup to download fresh copies.`, { cause: error })
  }
}

export async function prepareBuildTools(home, pin = toolchain) {
  const store = await tree(home, ".local/share/chariox/build-tools")
  const bin = await tree(home, ".local/bin")
  const link = join(bin, "chariox-build")
  const lock = join(store, ".install.lock")
  const id = createHash("sha256").update(JSON.stringify(pin)).update(runner).digest("hex")
  try { await mkdir(lock, { mode: 0o700 }) }
  catch (error) {
    if (error.code !== "EEXIST") throw error
    throw new Error(`Build tools installation already holds ${lock}. Wait for it to finish. If interrupted, confirm Setup has stopped before removing the empty lock and retrying.`)
  }

  try {
    const existing = await lstat(link).catch(absent)
    const target = existing?.isSymbolicLink() ? await readlink(link) : null
    if (existing && !(target?.startsWith(store + "/") && /^[a-f0-9]{64}\/run$/.test(target.slice(store.length + 1)))) {
      throw new Error(`${link} belongs to another installation. Move it aside and retry Setup.`)
    }
    const run = await preparePrefix(store, id, pin)
    if (target !== run) {
      const scratch = await mkdtemp(join(bin, ".chariox-build-"))
      try {
        const staged = join(scratch, "run")
        await symlink(run, staged)
        await rename(staged, link)
      } finally { await rm(scratch, { recursive: true, force: true }) }
    }
    return link
  } finally { await rm(lock, { recursive: true }) }
}
