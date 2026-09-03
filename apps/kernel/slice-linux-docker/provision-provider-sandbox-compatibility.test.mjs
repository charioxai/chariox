import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))

async function captureDockerCreateArgs({ allowUnconfinedSeccomp, allowProviderSandboxCompatibility }) {
  const root = await mkdtemp(join(tmpdir(), "chariox-provider-sandbox-create-"))
  try {
    const dockerLog = join(root, "docker.jsonl")
    await writeFile(join(root, "docker"), `#!/usr/bin/env node
const { appendFileSync } = require("node:fs");
const args = process.argv.slice(2);
if (args[0] === "info") process.exit(0);
if (args[0] === "container" && args[1] === "inspect") process.exit(1);
if (args[0] === "volume" && args[1] === "create") process.exit(0);
if (args[0] === "create") {
  appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify(args) + "\\n");
  process.exit(42);
}
throw new Error("unexpected Docker operation: " + JSON.stringify(args));
`, { mode: 0o700 })
    const environment = Object.fromEntries(Object.entries(process.env).filter(([name]) =>
      !name.startsWith("CHARIOX_SLICE_")))
    const result = spawnSync("bash", [provisioner, "start-runtime"], {
      encoding: "utf8",
      timeout: 60_000,
      env: {
        ...environment,
        PATH: `${root}:${environment.PATH}`,
        TMPDIR: root,
        CHARIOX_TEST_DOCKER_LOG: dockerLog,
        CHARIOX_SLICE_NAME: "chariox-provider-sandbox-create-fixture",
        CHARIOX_SLICE_ALLOW_UNCONFINED_SECCOMP: allowUnconfinedSeccomp ? "1" : "0",
        CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY:
          allowProviderSandboxCompatibility ? "1" : "0",
      },
    })
    assert.notEqual(result.status, 0, "the stubbed Docker create should stop the provisioner")
    return JSON.parse((await readFile(dockerLog, "utf8")).trim())
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}

test("provider sandbox compatibility fails before runtime startup when its isolation probe fails", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-provider-sandbox-probe-"))
  try {
    const dockerLog = join(root, "docker.jsonl")
    await writeFile(join(root, "docker"), `#!/usr/bin/env node
const { appendFileSync } = require("node:fs");
const args = process.argv.slice(2);
appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify(args) + "\\n");
if (args[0] === "info") process.exit(0);
if (args[0] === "container" && args[1] === "inspect" && args.includes("-f")) {
  console.log("sha256:fixture");
  process.exit(0);
}
if (args[0] === "container" && args[1] === "inspect") process.exit(0);
if (args[0] === "inspect") { console.log("true"); process.exit(0); }
if (args[0] === "image" && args[1] === "inspect") { console.log("sha256:fixture"); process.exit(0); }
if (args[0] === "cp") process.exit(0);
if (args[0] === "exec" && args.includes("bwrap")) process.exit(41);
if (args[0] === "exec") process.exit(0);
throw new Error("unexpected Docker operation: " + JSON.stringify(args));
`, { mode: 0o700 })

    const environment = Object.fromEntries(Object.entries(process.env).filter(([name]) =>
      !name.startsWith("CHARIOX_SLICE_")))
    const result = spawnSync("bash", [provisioner, "start-runtime"], {
      encoding: "utf8",
      timeout: 60_000,
      env: {
        ...environment,
        PATH: `${root}:${environment.PATH}`,
        TMPDIR: root,
        CHARIOX_TEST_DOCKER_LOG: dockerLog,
        CHARIOX_SLICE_NAME: "chariox-provider-sandbox-fixture",
        CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY: "1",
      },
    })

    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /provider sandbox compatibility probe failed/)
    const calls = (await readFile(dockerLog, "utf8")).trim().split("\n").map(JSON.parse)
    const probe = calls.find((args) => args[0] === "exec" && args.includes("bwrap"))
    assert.ok(probe)
    assert.ok(probe.includes("setpriv"))
    assert.ok(probe.includes("--no-new-privs"))
    assert.ok(probe.includes("--unshare-user"))
    assert.ok(probe.includes("--ro-bind"))
    assert.equal(
      calls.some((args) => args.at(-1) === "/opt/chariox-slice/start-runtime.sh"),
      false,
      "the worker runtime must not start after a failed isolation probe",
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("provider sandbox compatibility has a separate explicit Docker security grant", async () => {
  const legacyArgs = await captureDockerCreateArgs({
    allowUnconfinedSeccomp: true,
    allowProviderSandboxCompatibility: false,
  })
  assert.ok(legacyArgs.includes("seccomp=unconfined"))
  assert.equal(legacyArgs.some((arg) => arg.startsWith("apparmor=")), false)
  assert.equal(legacyArgs.includes("systempaths=unconfined"), false)

  const compatibilityArgs = await captureDockerCreateArgs({
    allowUnconfinedSeccomp: false,
    allowProviderSandboxCompatibility: true,
  })
  assert.ok(compatibilityArgs.includes("seccomp=unconfined"))
  assert.ok(compatibilityArgs.includes("apparmor=unconfined"))
  assert.ok(compatibilityArgs.includes("systempaths=unconfined"))
})
