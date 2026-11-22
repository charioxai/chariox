// MP-07 / MP-08 / MP-11: actual generic executable, no Node/Bun on target PATH.
import assert from "node:assert/strict"
import test from "node:test"
import { createServer } from "node:http"
import { execFile } from "node:child_process"
import { mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { setupFixture } from "./fixture.mjs"
const execute = promisify(execFile)
test("MP-07 generic unsigned Setup verifies and installs with system Python and the three existing provider CLIs", async t => {
  const root = await mkdtemp(join(await realpath(process.env.CHARIOX_BYOM_TEST_STATE ?? tmpdir()), "setup-compiled-")); t.after(() => rm(root, { recursive: true, force: true }))
  const f = await setupFixture(root), artifact = await readFile(f.archive)
  const responses = new Map([[`.manifest.json`, f.manifest], [`.manifest.sig`, f.signature], [`.tar.gz`, artifact]])
  const server = createServer((req, res) => { const suffix = [...responses.keys()].find(key => req.url.endsWith(key)); if (!suffix) { res.writeHead(404); return res.end() }; res.end(responses.get(suffix)) })
  await new Promise(r => server.listen(0, "127.0.0.1", r)); t.after(() => new Promise(r => server.close(r)))
  const output = join(root, "build")
  await execute(process.execPath, [fileURLToPath(new URL("../../scripts/build-chariox-setup.mjs", import.meta.url)), "--version", f.version, "--public-key", f.publicKeyHex, "--target", "linux-x64", "--output", output], { timeout: 60_000 })
  const binary = join(output, "chariox-setup"), home = join(root, "home"), tools = join(root, "tools")
  await mkdir(home); await mkdir(tools)
  await symlink("/usr/bin/python3", join(tools, "python3"))
  await writeFile(join(tools, "systemctl"), '#!/bin/sh\n[ "$1" = "--user" ] || exit 1\nif [ "$2" = "show" ]; then printf "LoadState=not-found\\nFragmentPath=\\nDropInPaths=\\nActiveState=inactive\\nUnitFileState=disabled\\n"; fi\n', { mode: 0o755 })
  for (const provider of ["claude", "codex", "opencode"]) {
    await writeFile(join(tools, provider), "#!/bin/sh\necho 100.0.0\n", { mode: 0o755 })
  }
  const env = { HOME: home, PATH: tools }
  assert.equal((await execute(binary, ["--version"], { env })).stdout, `Chariox Setup ${f.version}\n`)
  assert.equal((await execute(binary, ["build-toolchain", "status"], { env })).stdout, "Build toolchain: missing\n")
  await assert.rejects(execute(binary, ["build-toolchain", "run", "--", "python3", "--version"], { env }),
    /Build toolchain missing.*build-toolchain prepare/)
  const toolchain = join(home, ".local/share/chariox/build-toolchain")
  const compiler = join(toolchain, "env/bin/x86_64-conda-linux-gnu-cc")
  await mkdir(join(toolchain, "env/bin"), { recursive: true })
  await writeFile(compiler, "", { mode: 0o700 })
  await writeFile(join(toolchain, "version"), "1\n")
  assert.equal((await execute(binary, ["build-toolchain", "status"], { env })).stdout, "Build toolchain: managed version 1\n")
  const build = await execute(binary, ["build-toolchain", "run", "--", "/bin/sh", "-c", 'printf "%s" "$CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"'], { env })
  assert.equal(build.stdout, compiler)
  const bindgen = await execute(binary, ["build-toolchain", "run", "--", "/bin/sh", "-c", 'printf "%s" "$BINDGEN_EXTRA_CLANG_ARGS"'], { env })
  assert.equal(bindgen.stdout, `--sysroot="${toolchain}/env/x86_64-conda-linux-gnu/sysroot" -resource-dir="${toolchain}/env/lib/clang/23"`)
  await assert.rejects(execute(binary, ["build-toolchain", "run", "--", "/bin/sh", "-c", "exit 17"], { env }), { code: 17 })
  assert.equal((await execute(binary, ["build-toolchain", "remove"], { env })).stdout, "Build toolchain: removed\n")
  assert.equal((await execute(binary, ["build-toolchain", "status"], { env })).stdout, "Build toolchain: missing\n")
  const result = await execute(binary, ["--install-only", "--release-base", `http://127.0.0.1:${server.address().port}`, "--api-url", "http://127.0.0.1:1"], { env, timeout: 30_000 })
  assert.match(result.stdout, /Chariox installed/)
  assert.equal(await readFile(join(home, ".local/share/chariox/machines/local/current/bin/chariox-kernel"), "utf8"), await readFile(join(f.bundle, "bin/chariox-kernel"), "utf8"))
  assert.equal((await execute(binary, ["--uninstall"], { env, timeout: 30_000 })).stdout.includes("Chariox removed"), true)
})
