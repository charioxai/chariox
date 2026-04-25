import { spawn } from "node:child_process"
import { access, readdir, stat } from "node:fs/promises"
import path from "node:path"
import process from "node:process"
import { fileURLToPath } from "node:url"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(__dirname, "..")
const kernelDir = path.join(repoRoot, "apps/kernel")
const relayDir = path.join(repoRoot, "apps/relay")

export async function startRustBin(binaryName) {
  const binaryPath = path.join(kernelDir, "target/debug", binaryName)
  const inputs = [
    path.join(kernelDir, "Cargo.toml"),
    path.join(kernelDir, "Cargo.lock"),
    path.join(relayDir, "Cargo.toml"),
    ...await collectFiles(path.join(kernelDir, "src"), [".rs", ".md"]),
    ...await collectFiles(path.join(relayDir, "src"), [".rs"]),
  ]

  if (await needsBuild(inputs, [binaryPath])) {
    await run("cargo", ["build", "--manifest-path", path.join(kernelDir, "Cargo.toml"), "--bin", binaryName], repoRoot)
  }

  const child = spawn(binaryPath, process.argv.slice(2), {
    cwd: repoRoot,
    stdio: "inherit",
    env: process.env,
  })

  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal)
      return
    }
    process.exit(code ?? 1)
  })
}

async function needsBuild(inputPaths, outputPaths) {
  for (const outputPath of outputPaths) {
    try {
      await access(outputPath)
    } catch {
      return true
    }
  }
  const newestInput = await newestModifiedTime(inputPaths)
  const oldestOutput = await oldestModifiedTime(outputPaths)
  return newestInput > oldestOutput
}

async function newestModifiedTime(paths) {
  let newest = 0
  for (const filePath of paths) {
    const { mtimeMs } = await stat(filePath)
    newest = Math.max(newest, mtimeMs)
  }
  return newest
}

async function oldestModifiedTime(paths) {
  let oldest = Number.POSITIVE_INFINITY
  for (const filePath of paths) {
    const { mtimeMs } = await stat(filePath)
    oldest = Math.min(oldest, mtimeMs)
  }
  return oldest
}

async function collectFiles(root, extensions) {
  const files = []
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const fullPath = path.join(root, entry.name)
    if (entry.isDirectory()) {
      files.push(...await collectFiles(fullPath, extensions))
      continue
    }
    if (!entry.isFile()) {
      continue
    }
    if (!extensions.some((extension) => entry.name.endsWith(extension))) {
      continue
    }
    files.push(fullPath)
  }
  return files
}

async function run(command, args, cwd) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
      env: process.env,
      stdio: "inherit",
    })
    child.on("exit", (code) => {
      if (code === 0) {
        resolve()
        return
      }
      reject(new Error(`${command} ${args.join(" ")} exited with code ${code ?? 1}`))
    })
    child.on("error", reject)
  })
}
