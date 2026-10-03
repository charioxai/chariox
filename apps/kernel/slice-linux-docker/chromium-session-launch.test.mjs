import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const script = fileURLToPath(new URL("./docker/slice-screen.sh", import.meta.url))

// "fresh" has no profile yet; "no tab sessions" keeps a profile without them.
for (const scenario of ["fresh", "no tab sessions", "Sessions/Session_1", "Last Session", "Current Session", "Cookies", "truncated", "encrypted", "malformed"]) {
  const session = scenario === "fresh" ? undefined : scenario === "no tab sessions" ? null
    : ["truncated", "encrypted", "malformed"].includes(scenario) ? "Sessions/Session_1" : scenario;
  test(`URL recovery ${scenario} launches Chromium without treating the URL as flags`, async () => {
    const root = await mkdtemp(join(tmpdir(), "chariox-chromium-launch-"))
    try {
      const bin = join(root, "bin")
      const profile = join(root, "profile")
      const argsFile = join(root, "args.json")
      await mkdir(bin)
      if (session !== undefined) await mkdir(join(profile, "Default", "Sessions"), { recursive: true })
      const header = Buffer.from("534e535303000000", "hex")
      const saved = scenario === "truncated" ? Buffer.concat([header, Buffer.from([20, 0, 6])])
        : scenario === "encrypted" ? Buffer.from("534e535304000000", "hex")
        : scenario === "malformed" ? Buffer.concat([header, Buffer.from([1, 0, 6])]) : header;
      if (session) await writeFile(join(profile, "Default", session), saved)
      await writeFile(join(root, "browser-app-restore.mjs"), await readFile(new URL("./docker/browser-app-restore.mjs", import.meta.url)))
      // MP-08/MP-10: argv-only fixture substitutes process ownership as well
      // as Chromium. Real child retirement is tested by the lifecycle suite.
      await writeFile(join(root, "browser-lifecycle.py"),
        "import subprocess, sys\nif sys.argv[1] == 'start': subprocess.run(sys.argv[4:], check=True)\n")
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
      assert.equal(result.status, 0, result.stdout + result.stderr)
      const args = JSON.parse(await readFile(argsFile, "utf8"))
      const separator = args.indexOf("--")
      assert.ok(separator > 0)
      assert.deepEqual(args.slice(separator), ["--", url])
      const quarantined = ["encrypted", "malformed"].includes(scenario);
      assert.equal(args.includes("--restore-last-session"), session !== undefined && !quarantined)
      assert.ok(args.includes("--new-window"))
      assert.ok(args.includes(`--user-data-dir=${profile}`))
      assert.equal(args.slice(0, separator).includes("--no-sandbox"), false)
      assert.equal(args.some(arg => arg.startsWith("--unsafely-treat-insecure-origin-as-secure")), false)
      if (quarantined) {
        await assert.rejects(readFile(join(profile, "Default", session)), {code:"ENOENT"});
        const { readdir } = await import("node:fs/promises");
        const backup = (await readdir(join(profile, "Default"))).find(name => name.startsWith("chariox-unrestorable-"));
        assert.deepEqual(await readFile(join(profile, "Default", backup, "Session_1")), saved);
      } else if (session) assert.deepEqual(await readFile(join(profile, "Default", session)), header)
    } finally {
      try {
        const pid = Number(await readFile(join(root, "logs/chromium-supervisor.pid"), "utf8"));
        process.kill(pid, "SIGTERM");
        for (let n=0;n<40;n++) {
          try { await readFile(join(root, "logs/chromium-supervisor.pid")); }
          catch { break; }
          await new Promise(resolve=>setTimeout(resolve,50));
        }
      } catch {}
      await rm(root, { recursive: true, force: true })
    }
  })
}
