import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const script = fileURLToPath(new URL("./docker/slice-screen.sh", import.meta.url))

for (const session of [null, "Sessions/Session_1", "Last Session", "Current Session"]) {
  test(`URL recovery ${session ? `restores ${session}` : "uses a fresh profile"} without treating the URL as flags`, async () => {
    const root = await mkdtemp(join(tmpdir(), "chariox-chromium-launch-"))
    try {
      const bin = join(root, "bin")
      const profile = join(root, "profile")
      const argsFile = join(root, "args.json")
      await mkdir(bin)
      await mkdir(join(profile, "Default", "Sessions"), { recursive: true })
      if (session) await writeFile(join(profile, "Default", session), "saved-session-fixture")
      // Only OS/browser boundaries are substituted. Run the full production
      // entry point with private profile state; never touch a user's browser.
      await writeFile(join(bin, "pgrep"), `#!/bin/sh
case "$*" in
  *Xvfb*) printf '123 Xvfb fixture\\n'; exit 0 ;;
  *chromium*) if [ -f "$CHARIOX_TEST_ARGS" ]; then printf '124 chromium fixture\\n'; exit 0; fi ;;
esac
exit 1
`, { mode: 0o700 })
      await writeFile(join(bin, "xdpyinfo"), "#!/bin/sh\nexit 0\n", { mode: 0o700 })
      await writeFile(join(bin, "timeout"), "#!/bin/sh\nexit 0\n", { mode: 0o700 })
      await writeFile(join(bin, "chromium"), `#!${process.execPath}
require('node:fs').writeFileSync(process.env.CHARIOX_TEST_ARGS, JSON.stringify(process.argv.slice(2)));
`, { mode: 0o700 })
      const url = "--no-sandbox"
      const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("CHARIOX_SLICE_")))
      const result = spawnSync("bash", [script, "open-url", url], {
        encoding: "utf8", timeout: 10_000,
        env: { ...env, PATH: `${bin}:${env.PATH}`, CHARIOX_TEST_ARGS: argsFile,
          CHARIOX_SLICE_ROOT: root, CHARIOX_SLICE_CHROME_PROFILE: profile,
          CHARIOX_SLICE_DISPLAY_MODE: "headed" },
      })
      assert.equal(result.error, undefined)
      assert.equal(result.status, 0, result.stderr)
      const args = JSON.parse(await readFile(argsFile, "utf8"))
      const separator = args.indexOf("--")
      assert.ok(separator > 0)
      assert.deepEqual(args.slice(separator), ["--", url])
      assert.equal(args.includes("--restore-last-session"), Boolean(session))
      assert.ok(args.includes("--new-window"))
      assert.ok(args.includes(`--user-data-dir=${profile}`))
      assert.equal(args.slice(0, separator).includes("--no-sandbox"), false)
      assert.equal(args.some(arg => arg.startsWith("--unsafely-treat-insecure-origin-as-secure")), false)
      if (session) assert.equal(await readFile(join(profile, "Default", session), "utf8"), "saved-session-fixture")
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  })
}
