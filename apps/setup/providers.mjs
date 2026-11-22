import { createHash } from "node:crypto"
import { createReadStream, constants } from "node:fs"
import { access, lstat, mkdtemp, rename, rm, stat } from "node:fs/promises"
import { isAbsolute, join } from "node:path"
import { download } from "./download.mjs"
import { command, tree } from "./machine/remote.mjs"

const codex = "package/vendor/x86_64-unknown-linux-musl"
const providers = {
  claude: {
    name: "Claude Code", binary: "package/claude",
    userPaths: [".claude/local/claude"],
    updates: "https://code.claude.com/docs/en/setup#update-claude-code",
  },
  codex: {
    name: "Codex CLI", binary: `${codex}/bin/codex`,
    helpers: [`${codex}/bin/codex-code-mode-host`, `${codex}/codex-path/rg`, `${codex}/codex-resources/bwrap`],
    userPaths: [],
    updates: "https://learn.chatgpt.com/docs/codex/cli",
  },
  opencode: {
    name: "OpenCode", binary: "package/bin/opencode",
    userPaths: [".opencode/bin/opencode"],
    updates: "https://opencode.ai/docs/cli/commands/#upgrade",
  },
}

async function executable(path) {
  try {
    if (!(await stat(path)).isFile()) return false
    await access(path, constants.X_OK)
    return true
  } catch (error) {
    if (["ENOENT", "ENOTDIR", "EACCES"].includes(error.code)) return false
    throw error
  }
}

async function findExisting(home, cli, provider, env, variable, systemBins) {
  const override = env[variable]
  if (override !== undefined) {
    if (!isAbsolute(override) || !(await executable(override))) {
      throw new Error(`${variable} must name an absolute executable path. Correct it or unset it, then retry Setup.`)
    }
    return override
  }

  const userBins = [".local/bin", ".npm-global/bin", ".bun/bin", ".cargo/bin"]
  const pnpmHome = env.PNPM_HOME ?? join(env.XDG_DATA_HOME || join(home, ".local/share"), "pnpm")
  if (isAbsolute(pnpmHome)) userBins.push(pnpmHome)
  const candidates = [
    ...(env.PATH ?? "").split(":").filter(isAbsolute).map(dir => join(dir, cli)),
    ...userBins.map(dir => join(isAbsolute(dir) ? dir : join(home, dir), cli)),
    ...provider.userPaths.map(path => join(home, path)),
    ...systemBins.map(dir => join(dir, cli)),
  ]
  for (const path of new Set(candidates)) {
    if (await executable(path)) return path
  }
}

async function readVersion(path, env) {
  try {
    const text = await command(path, ["--version"], true, undefined, env)
    return text.match(/^(?:codex-cli )?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)(?:\s|$)/m)?.[1]
  } catch {
    return undefined
  }
}

function isOlder(version, minimum) {
  const parts = version.split("-")[0].split(".").map(Number)
  const required = minimum.split(".").map(Number)
  const different = parts.findIndex((part, i) => part !== required[i])
  return different === -1 ? version.includes("-") : parts[different] < required[different]
}

async function installProvider({ home, cli, provider, artifact, env, variable, systemBins }) {
  const store = await tree(home, ".local/share/chariox/providers")
  const target = join(store, cli)
  const binary = join(target, provider.binary)
  const files = [provider.binary, ...(provider.helpers ?? [])]
  const complete = async dir => (await Promise.all(files.map(file => executable(join(dir, file))))).every(Boolean)
  if (await complete(target)) return binary
  if (await lstat(target).then(() => true, error => error.code === "ENOENT" ? false : Promise.reject(error))) {
    throw new Error(`${provider.name} has an incomplete install at ${target}. Move it aside and retry Setup.`)
  }

  const scratch = await mkdtemp(join(store, `.${cli}-`))
  try {
    const archive = join(scratch, "provider.tgz")
    await download(artifact.url, archive, 256 * 1024 ** 2)
    const digest = createHash("sha512")
    for await (const chunk of createReadStream(archive)) digest.update(chunk)
    if (`sha512-${digest.digest("base64")}` !== artifact.integrity) throw new Error("package integrity check failed")

    // Only extract the official archive identified by the integrity pin compiled into Setup.
    await command("tar", ["-xzf", archive, "--no-same-owner", "--no-same-permissions", "-C", scratch])
    if (!(await complete(scratch))) throw new Error("package is missing an executable")
    if (await readVersion(join(scratch, provider.binary), env) !== artifact.version) {
      throw new Error("package did not start or reported an unexpected version")
    }
    const existing = await findExisting(home, cli, provider, env, variable, systemBins)
    if (existing) return existing
    await rm(archive)
    try {
      await rename(scratch, target)
    } catch (error) {
      if (!["EEXIST", "ENOTEMPTY"].includes(error.code) || !(await complete(target))) throw error
    }
    return binary
  } catch (error) {
    throw new Error(`Could not install ${provider.name}: ${error.message}. Install it using ${provider.updates} or retry Setup.`, { cause: error })
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
}

export async function prepareProviders({ home, artifacts, env, notice, systemBins = ["/usr/local/bin", "/usr/bin"] }) {
  const serviceEnvironment = {}
  const probeEnv = { ...env, HOME: home }
  for (const [cli, provider] of Object.entries(providers)) {
    const variable = `CHARIOX_${cli.toUpperCase()}_BIN`
    const artifact = artifacts[cli]
    const path = await findExisting(home, cli, provider, probeEnv, variable, systemBins)
      ?? await installProvider({ home, cli, provider, artifact, env: probeEnv, variable, systemBins })
    const version = await readVersion(path, probeEnv)
    const selection = `${provider.name} ${version ?? "(version unavailable)"}: using ${path}`
    const update = !version || isOlder(version, artifact.version)
      ? `. ${artifact.version} or newer is recommended. Update this installation: ${provider.updates}` : ""
    notice(selection + update)
    serviceEnvironment[variable] = path
  }
  return serviceEnvironment
}
