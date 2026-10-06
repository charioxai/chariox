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
if (args[0] === "info") { console.log("engine-fixture"); process.exit(0); }
if (args[0] === "container" && args[1] === "inspect") { console.log("fixture-image"); process.exit(0); }
if (args[0] === "image" && args[1] === "inspect") { console.log("fixture-image"); process.exit(0); }
if (args[0] === "inspect" && !args.includes("--format") && !args.includes("-f")) {
  console.log(JSON.stringify([{
    Id: "a".repeat(64), Image: "fixture-image", Created: "fixture-created",
    State: { Running: true, Paused: false, Restarting: false, Status: "running", Pid: 123, StartedAt: "fixture-started", FinishedAt: "" },
    HostConfig: { PidMode: "" }, Config: { Env: ["HOME=/home/slice"], Labels: {
      "io.chariox.slice.id": process.env.CHARIOX_SLICE_ID,
      "io.chariox.slice.owner-kernel-id": process.env.CHARIOX_SLICE_OWNER_KERNEL_ID,
      "io.chariox.slice.owner-machine-id": process.env.CHARIOX_SLICE_OWNER_MACHINE_ID,
    } },
  }]));
  process.exit(0);
}
if (args[0] === "inspect") {
  const format = args[args.indexOf("--format") + 1] || args[args.indexOf("-f") + 1] || "";
  console.log(format.includes("HostConfig.Ulimits") ? "8192:8192" : "true");
  process.exit(0);
}
if (args[0] === "ps") { console.log("chariox-download-reserve-fixture"); process.exit(0); }
if (args[0] === "exec" && args.includes("python3")) {
  console.log(JSON.stringify({ disposition: "clear", profileProcessCount: 0 }));
  process.exit(0);
}
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
      // Includes owned-process inspection on shared builders.
      timeout: 45_000,
      env: {
        ...env,
        PATH: `${root}:${env.PATH}`,
        TMPDIR: root,
        CHARIOX_TEST_DOCKER_LOG: log,
        CHARIOX_SLICE_BUILD_CONTEXT_DIGEST: `sha256:${"a".repeat(64)}`,
        CHARIOX_SLICE_BUILD_IMAGE: "never",
        CHARIOX_SLICE_NAME: "chariox-download-reserve-fixture",
        CHARIOX_SLICE_ID: "slice-fixture",
        CHARIOX_SLICE_OWNER_KERNEL_ID: "kernel-fixture",
        CHARIOX_SLICE_OWNER_MACHINE_ID: "machine-fixture",
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
  // MP-02/MP-08/MP-10: launch must use the no-new-privs mode admitted by the probe,
  // including on rootful Docker where setuid bwrap can fail at the sysctl write.
  assert.match(launcher, /exec \/usr\/bin\/setpriv --no-new-privs \/usr\/bin\/bwrap --seccomp 3 "\$@"/)
  assert.doesNotMatch(launcher, /\/proc\/self\/uid_map/)
  // The compatibility probe must exercise the same unprivileged launch mode.
  assert.match(source, /setpriv --no-new-privs\s*\\\s*bwrap[\s\S]*--disable-userns/)
  assert.match(seccomp, /SCMP_SYS\(unshare\)/)
  assert.match(seccomp, /SCMP_SYS\(clone3\)/)
  assert.match(runtimeSource, /CHARIOX_MANAGED_PROVIDER_BWRAP="\/usr\/local\/libexec\/chariox\/managed-provider-bwrap"/)
  assert.match(profile, /profile chariox-slice-provider flags=\(unconfined\)/)
  assert.match(profile, /^\s*userns,\s*$/m)
})

test("protected default provider accounts stay in the private root, outside the shared provider HOME", async () => {
  const runtimeSource = await readFile(runtime, "utf8")

  assert.match(runtimeSource, /DEFAULT_PROVIDER_ROOT="\$CHARIOX_SLICE_PRIVATE_ROOT\/provider-default"/)
  assert.match(runtimeSource, /CODEX_HOME="\$DEFAULT_PROVIDER_ROOT\/codex" CLAUDE_CONFIG_DIR="\$DEFAULT_PROVIDER_ROOT\/claude"/)
  assert.doesNotMatch(runtimeSource, /(?:CODEX_HOME|CLAUDE_CONFIG_DIR)="\$PROVIDER_HOME/)
  assert.match(runtimeSource, /screen -dmS chariox-slice-kernel env \\\n\s+"\$\{default_provider_env\[@\]\}"/)
})

test("the isolation probe runs on its own kernel and the slice kernel starts without probe env", async () => {
  const source = await readFile(runtime, "utf8")
  const probeStart = source.match(/^  start_slice_kernel \\\n((?:    .*\n)+)/m)?.[1] ?? ""
  assert.match(probeStart, /CHARIOX_ACCEPT_REMOTE_LEASES=0/)
  assert.match(probeStart, /CHARIOX_CODEX_BIN="\$ROOT\/managed-provider-isolation-probe-wrapper\.sh"/)
  // Probe overrides come after the defaults so they win in env(1).
  assert.match(source, /CHARIOX_ACCEPT_REMOTE_LEASES=1 \\\n    "\$@" \\\n    "\$ROOT\/bin\/chariox-kernel"/)
  assert.match(source, /pkill -TERM -f "codex\[\^ \]\* app-server"/)
  assert.match(source, /\nfi\nstart_slice_kernel\n/)
})
