import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { generateKeyPairSync, verify } from "node:crypto"
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"
import { PLATFORMS, verifyBundle } from "./release-bundle.mjs"

const root = fileURLToPath(new URL("../", import.meta.url))
test("one owner command publishes the verified, versioned Linux Setup and signed bundle without macOS", async t => {
  const scratch = await mkdtemp(join(tmpdir(), "publish-linux-setup-"))
  t.after(() => rm(scratch, { recursive: true, force: true }))
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  const pin = publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("hex")
  const pem = join(scratch, "test-signing.pem")
  await writeFile(pem, privateKey.export({ format: "pem", type: "pkcs8" }), { mode: 0o600 })
  const executables = join(scratch, "executables"), published = join(scratch, "published"), bin = join(scratch, "bin")
  await mkdir(join(executables, "bin"), { recursive: true })
  await mkdir(published)
  await mkdir(bin)
  for (const name of PLATFORMS["linux-x64"].bin) {
    await writeFile(join(executables, "bin", name), "#!/bin/sh\nexit 0\n", { mode: 0o755 })
  }
  await writeFile(join(bin, "gh"), `#!/bin/sh
set -eu
[ "$1 $2" = 'release create' ]
printf '%s\\n' "$@" > "$PUBLISHED/args"
for asset do
  if [ -f "$asset" ]; then cp -- "$asset" "$PUBLISHED/"; fi
done
`, { mode: 0o755 })
  const git = execFileSync("sh", ["-c", "command -v git"], { encoding: "utf8" }).trim()
  assert.match(git, /^\/[A-Za-z0-9/_.-]+$/)
  await writeFile(join(bin, "git"), `#!/bin/sh
if [ "$1" = ls-remote ]; then
  printf '%s' "$SETUP_FIXTURE_TAGS"
  exit 0
fi
exec "${git}" "$@"
`, { mode: 0o755 })
  const argsForPublish = ["scripts/publish-linux-setup.mjs", "--version", "0.7.1",
    "--public-key", pin, "--signing-key", pem, "--executables", executables]
  const env = { ...process.env, PATH: bin + ":" + process.env.PATH, PUBLISHED: published, SETUP_FIXTURE_TAGS: "" }
  assert.throws(() => execFileSync(process.execPath, argsForPublish, {
    cwd: root, env: { ...env, SETUP_FIXTURE_TAGS: "0".repeat(40) + "\trefs/tags/v0.7.1\n" },
    stdio: ["ignore", "pipe", "pipe"], timeout: 10_000,
  }), /tag v0\.7\.1 belongs to a different commit/)
  assert.deepEqual(await readdir(published), [])
  execFileSync(process.execPath, argsForPublish, { cwd: root, env, timeout: 120_000 })
  const name = "chariox-0.7.1-linux-x64", setup = "chariox-setup-0.7.1-linux-x64"
  assert.deepEqual((await readdir(published)).sort(), [
    "args", "install.sh", setup, setup + ".sig", name + ".manifest.json", name + ".manifest.sig", name + ".tar.gz",
  ].sort())
  const args = await readFile(join(published, "args"), "utf8")
  assert.match(args, /v0\.7\.1/)
  const bytes = await readFile(join(published, setup)), sig = await readFile(join(published, setup + ".sig"), "utf8")
  assert.match(sig, /^[a-f0-9]{128}$/)
  assert.ok(verify(null, bytes, publicKey, Buffer.from(sig, "hex")))
  const bootstrap = await readFile(join(published, "install.sh"), "utf8")
  assert.ok(bootstrap.includes(pin))
  assert.ok(bootstrap.includes("https://github.com/charioxai/chariox/releases/download"))
  assert.match(bootstrap, /Signed macOS Setup is not available/)
  assert.match(bootstrap, /version='0\.7\.1'/)
  execFileSync("tar", ["-xzf", join(published, name + ".tar.gz"), "-C", published])
  const receipt = await verifyBundle(join(published, name), pin)
  assert.equal(receipt.platform, "linux-x64")
  assert.deepEqual(await readFile(join(published, name, "bin/chariox-setup")), bytes)
  for (const extension of ["json", "sig"]) {
    assert.deepEqual(await readFile(join(published, name + ".manifest." + extension)),
      await readFile(join(published, name, "manifest." + extension)))
  }
})
