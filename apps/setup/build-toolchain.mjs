import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { constants } from "node:os"
import { mkdir, open, readFile, readdir, rename, rm, stat, symlink, writeFile } from "node:fs/promises"
import { isAbsolute, join } from "node:path"
import { existingTree, regular, tree } from "./machine/remote.mjs"

const directory = ".local/share/chariox/build-toolchain"
const compiler = "x86_64-conda-linux-gnu"
const version = "1"
const micromamba = {
  url: "https://conda.anaconda.org/conda-forge/linux-64/micromamba-2.3.2-0.tar.bz2",
  sha256: "5512233cdd8564a671626081026dc861537a963baa06706baab08fac6f3bb9d2",
}

export const buildToolchainHelp = "chariox-setup build-toolchain prepare|status|remove\nchariox-setup build-toolchain run -- COMMAND [ARGS...]\n"

function execute(program, args, { env, input, stdio = "inherit", cwd, capture = false } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, { env, cwd, stdio: capture ? ["ignore", "pipe", "ignore"] : input ? ["pipe", "ignore", "ignore"] : stdio })
    let stdout = ""
    child.stdout?.on("data", chunk => { stdout += chunk })
    child.once("error", reject)
    child.once("close", (code, signal) => resolve({ code: code ?? 128 + constants.signals[signal], stdout }))
    if (input) {
      child.stdin.on("error", () => {})
      child.stdin.end(input)
    }
  })
}

async function required(program, args, options) {
  const { code } = await execute(program, args, options)
  if (code !== 0) throw new Error(`${program} exited with status ${code}`)
}

async function ready(program, args, options) {
  try {
    return (await execute(program, args, { stdio: "ignore", ...options })).code === 0
  } catch (error) {
    if (error.code !== "ENOENT" && error.code !== "EACCES") throw error
    return false
  }
}

async function clangLibrary(path) {
  try {
    return (await readdir(path)).some(name => /^libclang(?:-\d+)?\.so(?:\.\d+)*$/.test(name))
  } catch (error) {
    if (error.code !== "ENOENT" && error.code !== "ENOTDIR") throw error
    return false
  }
}

async function systemClang() {
  if (process.env.LIBCLANG_PATH) {
    const path = process.env.LIBCLANG_PATH
    return await clangLibrary(path) || (await stat(path).catch(error => {
      if (error.code !== "ENOENT") throw error
    }))?.isFile() === true
  }
  // Distribution libclang development packages commonly live under /usr/lib/llvm-N.
  const llvm = (await readdir("/usr/lib")).filter(name => /^llvm-\d+$/.test(name))
  for (const name of llvm) {
    if (await clangLibrary(join("/usr/lib", name, "lib"))) return true
  }
  try {
    const { stdout } = await execute("/sbin/ldconfig", ["-p"], { capture: true })
    return /\blibclang(?:-\d+)?\.so(?:\.\d+)*\s/.test(stdout)
  } catch (error) {
    if (error.code !== "ENOENT") throw error
    return false
  }
}

async function buildPrerequisites(env, libclang) {
  for (const [program, args] of [
    ["pkg-config", ["--exists", "openssl", "dbus-1"]],
    ["cmake", ["--version"]],
    ["make", ["--version"]],
    ["perl", ["--version"]],
  ]) {
    if (!await ready(program, args, { env })) return false
  }
  for (const [program, language] of [[env.CC ?? "cc", "c"], [env.CXX ?? "c++", "c++"]]) {
    if (!await ready(program, ["-x", language, "-", "-o", "/dev/null"], { env, input: "int main(void) { return 0; }\n" })) return false
  }
  return await libclang()
}

function buildEnvironment(prefix) {
  const bin = join(prefix, "bin")
  const lib = join(prefix, "lib")
  const cc = join(bin, `${compiler}-cc`)
  const sysroot = join(prefix, compiler, "sysroot")
  // Libclang has no compiler driver to discover these locations for bindgen.
  const clangResources = join(lib, "clang", "23")
  const bindgenArgs = [
    `--sysroot=${JSON.stringify(sysroot)}`,
    `-resource-dir=${JSON.stringify(clangResources)}`,
    process.env.BINDGEN_EXTRA_CLANG_ARGS,
  ].filter(Boolean).join(" ")
  return {
    ...process.env,
    PATH: `${bin}:${process.env.PATH ?? "/usr/bin:/bin"}`,
    CC: cc,
    CXX: join(bin, `${compiler}-c++`),
    AR: join(bin, `${compiler}-ar`),
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER: cc,
    CMAKE_PREFIX_PATH: prefix,
    TMPDIR: process.env.HOME,
    LIBCLANG_PATH: lib,
    BINDGEN_EXTRA_CLANG_ARGS: bindgenArgs,
    OPENSSL_DIR: prefix,
    PKG_CONFIG_PATH: `${lib}/pkgconfig:${prefix}/share/pkgconfig`,
    PKG_CONFIG_LIBDIR: `${lib}/pkgconfig:${prefix}/share/pkgconfig`,
  }
}

async function installedVersion(root) {
  if (!root) return null
  try {
    const installed = (await regular(join(root, "version"), 32)).toString().trim()
    if (!/^\d{1,10}$/.test(installed)) throw new Error("Managed build toolchain version is invalid. Remove it and prepare again.")
    const cc = await stat(join(root, "env", "bin", `${compiler}-cc`))
    return cc.isFile() ? installed : null
  } catch (error) {
    if (error.code === "ENOENT") return null
    throw error
  }
}

// Reads the one `bin/micromamba` member with Python, which Setup already requires. GNU tar -j would
// need the host's bzip2, a package a minimal Linux machine may lack; only the installed environment
// carries bzip2, and that is installed afterwards.
const extractMicromambaScript = `
import os, shutil, sys, tarfile
archive, destination = sys.argv[1:3]
with tarfile.open(archive, "r:bz2") as bundle:
    member = bundle.getmember("bin/micromamba")
    if not member.isreg() or member.size > 256 * 1024 * 1024:
        raise SystemExit("bin/micromamba is not a regular file of a sane size")
    os.makedirs(os.path.dirname(destination), mode=0o700, exist_ok=True)
    with bundle.extractfile(member) as source, os.fdopen(os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o700), "wb") as target:
        shutil.copyfileobj(source, target)
`

export async function extractMicromamba(archive, destination, options) {
  await required("python3", ["-I", "-c", extractMicromambaScript, archive, destination], options)
}

async function install(root, prefix, home, packageLock, lockFd) {
  const bootstrap = join(root, "bootstrap"), cache = join(root, "cache"), temp = join(root, "tmp")
  const archive = join(bootstrap, "micromamba.tar.bz2"), lock = join(bootstrap, "linux-x64.lock")
  const env = {
    HOME: home,
    PATH: "/usr/bin:/bin",
    http_proxy: process.env.http_proxy,
    https_proxy: process.env.https_proxy,
    no_proxy: process.env.no_proxy,
    HTTPS_PROXY: process.env.HTTPS_PROXY,
    NO_PROXY: process.env.NO_PROXY,
    MAMBA_ROOT_PREFIX: cache,
    XDG_CACHE_HOME: cache,
    TMPDIR: temp,
  }
  const stdio = ["inherit", "inherit", "inherit", lockFd]
  try {
    for (const path of [prefix, bootstrap, cache, temp]) await rm(path, { recursive: true, force: true })
    await mkdir(bootstrap, { mode: 0o700 })
    await mkdir(temp, { mode: 0o700 })
    await required("curl", ["-q", "--fail", "--silent", "--show-error", "--location", "--max-redirs", "4",
      "--proto", "=https", "--proto-redir", "=https", "--max-time", "120", "--max-filesize", "10000000",
      "--output", archive, micromamba.url], { env, stdio })
    const digest = createHash("sha256").update(await readFile(archive)).digest("hex")
    if (digest !== micromamba.sha256) throw new Error("Micromamba download failed SHA-256 verification")
    await extractMicromamba(archive, join(bootstrap, "bin/micromamba"), { env, stdio })
    await writeFile(lock, packageLock)
    await required(join(bootstrap, "bin/micromamba"), ["--no-rc", "create", "--yes", "--prefix", prefix, "--file", lock], { env, stdio })
    for (const tool of ["cc", "c++", "gcc", "g++", "ar", "as", "ld", "nm", "ranlib", "strip"]) {
      await symlink(`${compiler}-${tool}`, join(prefix, "bin", tool)).catch(error => {
        if (error.code !== "EEXIST") throw error
      })
    }
    if (!await buildPrerequisites(buildEnvironment(prefix), () => clangLibrary(join(prefix, "lib")))) {
      throw new Error("Installed build tools did not pass the native compiler and library check")
    }
    await writeFile(join(bootstrap, "version"), `${version}\n`, { mode: 0o600 })
    await rename(join(bootstrap, "version"), join(root, "version"))
  } catch (error) {
    await rm(prefix, { recursive: true, force: true })
    throw new Error(`Build toolchain preparation failed. Check the error and retry prepare. ${error.message}`, { cause: error })
  } finally {
    for (const path of [bootstrap, cache, temp]) await rm(path, { recursive: true, force: true })
  }
}

async function withLock(root, access, action) {
  const handle = await open(root, "r")
  try {
    // The open directory description keeps the advisory lock after flock exits.
    const { code } = await execute("/usr/bin/flock", ["-n", access, "3"], { stdio: ["ignore", "ignore", "inherit", handle.fd] })
    if (code !== 0) throw new Error("The build toolchain is in use. Wait for the current build or preparation to finish and retry.")
    return await action(handle.fd)
  } finally {
    await handle.close()
  }
}

export async function runBuildToolchain(argv, packageLock, write = text => process.stdout.write(text)) {
  const [action, separator, ...command] = argv
  if (action === "--help" && argv.length === 1) { write(buildToolchainHelp); return }
  if (!(["prepare", "remove", "status"].includes(action) && argv.length === 1) &&
      !(action === "run" && separator === "--" && command.length > 0)) {
    throw new Error(buildToolchainHelp.trim())
  }
  if (process.platform !== "linux" || process.arch !== "x64") throw new Error("Managed build tools are available on Linux x86-64")
  const home = process.env.HOME
  if (!home || !isAbsolute(home)) throw new Error("An absolute user home is required")
  const system = action !== "remove" && await buildPrerequisites({ ...process.env, TMPDIR: home }, systemClang)
  if (system) {
    if (action === "run") process.exitCode = (await execute(command[0], command.slice(1))).code
    else write("Build toolchain: system\n")
    return
  }
  const found = await existingTree(home, directory)
  if (action === "status") {
    const installed = await installedVersion(found)
    write(`Build toolchain: ${installed ? `managed version ${installed}` : "missing"}\n`)
    return
  }
  if (action === "run") {
    if (!found) throw new Error("Build toolchain missing. Run chariox-setup build-toolchain prepare first.")
    return await withLock(found, "-s", async lockFd => {
      const installed = await installedVersion(found)
      if (!installed) throw new Error("Build toolchain missing. Run chariox-setup build-toolchain prepare first.")
      if (installed !== version) throw new Error("Build toolchain version changed. Run chariox-setup build-toolchain prepare first.")
      const env = buildEnvironment(join(found, "env"))
      const stdio = ["inherit", "inherit", "inherit", lockFd]
      process.exitCode = (await execute(command[0], command.slice(1), { env, stdio })).code
    })
  }
  if (action === "remove" && !found) { write("Build toolchain: removed\n"); return }
  const root = found ?? await tree(home, directory)
  await withLock(root, "-x", async lockFd => {
    if (action === "remove") {
      for (const name of ["version", "env", "bootstrap", "cache", "tmp"]) await rm(join(root, name), { recursive: true, force: true })
      write("Build toolchain: removed\n")
      return
    }
    if (await installedVersion(root) !== version) {
      await rm(join(root, "version"), { force: true })
      if (!packageLock) throw new Error("This Setup executable lacks the pinned build toolchain")
      await install(root, join(root, "env"), home, packageLock, lockFd)
    }
    write(`Build toolchain: managed version ${version}\n`)
  })
}
