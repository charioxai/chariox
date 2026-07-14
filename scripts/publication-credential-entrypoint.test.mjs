import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { promisify } from "node:util"
import { test } from "node:test"

const execFileAsync = promisify(execFile)
const repositoryRoot = resolve(import.meta.dirname, "..")

test("publication entrypoint imports every typed credential binding into runtime home", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-publication-credentials-"))
  const home = join(root, "home")
  const bindings = join(root, "bindings")
  const codex = join(bindings, "000", "home", ".codex")
  const claude = join(bindings, "001", "home", ".claude")
  await mkdir(codex, { recursive: true })
  await mkdir(claude, { recursive: true })
  await writeFile(join(codex, "auth.json"), "codex-fixture")
  await writeFile(join(claude, ".credentials.json"), "claude-fixture")
  try {
    await execFileAsync("bash", [join(repositoryRoot, "docker/publication/entrypoint.sh"), "true"], {
      env: {
        ...process.env,
        HOME: home,
        ARROBA_CONFIG_DIR: join(home, ".config", "arroba"),
        ARROBA_DATA_DIR: join(home, ".local", "share", "arroba"),
        ARROBA_RUNTIME_DIR: join(home, ".cache", "arroba", "runtime"),
        ARROBA_SESSION_HISTORY_DIR: join(home, ".local", "share", "arroba", "sessions"),
        ARROBA_CREDENTIAL_BINDINGS_ROOT: bindings,
        ARROBA_PROVIDER_CREDENTIALS_DIR: join(root, "missing-legacy-profile"),
        ARROBA_WORKSPACE_DIR: join(root, "workspace"),
      },
    })
    assert.equal(await readFile(join(home, ".codex", "auth.json"), "utf8"), "codex-fixture")
    assert.equal(await readFile(join(home, ".claude", ".credentials.json"), "utf8"), "claude-fixture")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
