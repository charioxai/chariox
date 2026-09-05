import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))
const apparmorProfile = fileURLToPath(new URL("./chariox-slice-provider.apparmor", import.meta.url))
const dockerfile = fileURLToPath(new URL("./docker/Dockerfile", import.meta.url))
const bubblewrapLauncher = fileURLToPath(new URL("./docker/managed-provider-bwrap.sh", import.meta.url))
const seccompSource = fileURLToPath(new URL("./docker/managed-provider-seccomp.c", import.meta.url))
const runtime = fileURLToPath(new URL("./docker/start-runtime.sh", import.meta.url))

test("a configured slice memory limit does not acquire extra swap", async () => {
  const source = await readFile(provisioner, "utf8")

  assert.match(source, /--memory \"\$SLICE_DOCKER_MEMORY\"/)
  assert.match(source, /--memory-swap \"\$SLICE_DOCKER_MEMORY\"/)
})

test("the shared slice disk reserve reaches the browser controller container", async () => {
  const source = await readFile(provisioner, "utf8")

  assert.match(source, /-e "CHARIOX_SLICE_MIN_FREE_MB=\$SLICE_MIN_FREE_MB"/)
  assert.match(source, /-e CHARIOX_SLICE_MIN_FREE_MB="\$SLICE_MIN_FREE_MB"/)
})

test("a reused slice starts its runtime with the current disk reserve", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-download-reserve-"))
  try {
    const log = join(root, "docker.jsonl")
    await writeFile(
      join(root, "docker"),
      `#!/usr/bin/env node
const fs = require("node:fs");
const args = process.argv.slice(2);
fs.appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify(args) + "\\n");
if (args[0] === "info") process.exit(0);
if (args[0] === "container" && args[1] === "inspect") { console.log("fixture-image"); process.exit(0); }
if (args[0] === "image" && args[1] === "inspect") { console.log("fixture-image"); process.exit(0); }
if (args[0] === "inspect") { console.log("true"); process.exit(0); }
if (args[0] === "ps") { console.log("chariox-download-reserve-fixture"); process.exit(0); }
if (args[0] === "exec" && args.includes("df")) {
  console.log("Filesystem 1024-blocks Used Available Capacity Mounted on");
  console.log("fixture 10000000 1 9999999 1% /");
}
if (["cp", "exec", "update"].includes(args[0])) process.exit(0);
throw new Error("unexpected Docker call: " + args[0]);
`,
      { mode: 0o700 },
    )
    const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith("CHARIOX_SLICE_")))
    const result = spawnSync("bash", [provisioner, "start-runtime"], {
      encoding: "utf8",
      timeout: 15_000,
      env: {
        ...env,
        PATH: `${root}:${env.PATH}`,
        TMPDIR: root,
        CHARIOX_TEST_DOCKER_LOG: log,
        CHARIOX_SLICE_BUILD_CONTEXT_DIGEST: `sha256:${"a".repeat(64)}`,
        CHARIOX_SLICE_BUILD_IMAGE: "never",
        CHARIOX_SLICE_NAME: "chariox-download-reserve-fixture",
        CHARIOX_SLICE_MIN_FREE_MB: "777",
      },
    })
    assert.equal(result.error, undefined)
    assert.equal(result.status, 0, result.stderr)
    const calls = (await readFile(log, "utf8"))
      .trim()
      .split("\n")
      .map(JSON.parse)
    const runtime = calls.find((args) => args[0] === "exec" && args.at(-1) === "/opt/chariox-slice/start-runtime.sh")
    assert.ok(runtime, "fixture must start the reused container runtime")
    assert.ok(runtime.includes("CHARIOX_SLICE_MIN_FREE_MB=777"), `runtime reserve missing from ${runtime.join(" ")}`)
    assert.equal(calls.some((args) => args[0] === "create"), false)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("provider listener ranges are reserved from container ephemeral ports", async () => {
  const source = await readFile(provisioner, "utf8")

  assert.match(
    source,
    /--sysctl \"net\.ipv4\.ip_local_reserved_ports=\$SLICE_CODEX_PORT_RANGE,\$SLICE_OPENCODE_PORT_RANGE\"/,
  )
})

test("nested provider namespaces can use a host-installed AppArmor profile", async () => {
  const [source, profile, image, launcher, seccomp, runtimeSource] = await Promise.all([
    readFile(provisioner, "utf8"),
    readFile(apparmorProfile, "utf8"),
    readFile(dockerfile, "utf8"),
    readFile(bubblewrapLauncher, "utf8"),
    readFile(seccompSource, "utf8"),
    readFile(runtime, "utf8"),
  ])

  assert.match(source, /CHARIOX_SLICE_APPARMOR_PROFILE/)
  assert.match(source, /--cap-add SYS_ADMIN/)
  assert.match(source, /--cap-add NET_ADMIN/)
  assert.match(source, /--cap-add SYS_PTRACE/)
  assert.match(source, /--security-opt apparmor="\$SLICE_APPARMOR_PROFILE"/)
  assert.match(image, /chmod 4755 \/usr\/bin\/bwrap/)
  assert.match(launcher, /\/usr\/bin\/bwrap --seccomp 3/)
  assert.match(seccomp, /SCMP_SYS\(unshare\)/)
  assert.match(seccomp, /SCMP_SYS\(clone3\)/)
  assert.match(runtimeSource, /CHARIOX_MANAGED_PROVIDER_BWRAP="\/usr\/local\/libexec\/chariox\/managed-provider-bwrap"/)
  assert.match(profile, /profile chariox-slice-provider flags=\(unconfined\)/)
  assert.match(profile, /^\s*userns,\s*$/m)
})
