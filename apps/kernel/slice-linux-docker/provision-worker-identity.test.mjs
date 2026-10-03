import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url));
const runtime = new URL("./docker/start-runtime.sh", import.meta.url);
const fixture = JSON.parse(await readFile(new URL("../../../fixtures/slice-worker-identity.json", import.meta.url), "utf8")).cases[0];

function isolatedEnvironment(root) {
  return { ...Object.fromEntries(Object.entries(process.env).filter(([name]) =>
    !name.startsWith("CHARIOX_") && name !== "BASH_ENV")),
    HOME: root, TMPDIR: root, PATH: `${root}:${process.env.PATH}` };
}

test("production docker exec forwards canonical ID separately from the friendly alias", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-slice-worker-identity-"));
  try {
    await writeFile(join(root, "docker"), `#!/usr/bin/env node
const { writeFileSync } = require("node:fs");
const args = process.argv.slice(2);
if (args[0] === "info") { console.log("engine-synthetic"); process.exit(0); }
if (args[0] === "inspect" && !args.includes("--format") && !args.includes("-f")) {
 console.log(JSON.stringify([{Id: "a".repeat(64), State: {Running: true, Paused: false, Restarting: false, Status: "running", Pid: 123, StartedAt: "start", FinishedAt: ""}, HostConfig: {PidMode: "", RestartPolicy: {Name: "no"}}, Config: {Labels: {"io.chariox.slice.id": "slice-1", "io.chariox.slice.owner-kernel-id": "kernel-a", "io.chariox.slice.owner-machine-id": "machine-a"}}}])); process.exit(0);
}
if (args[0] === "exec" && args.includes("python3")) { console.log(JSON.stringify({disposition: "clear", profileProcessCount: 0})); process.exit(0); }
if (args[0] === "inspect") {
 const format = args[args.indexOf("--format") + 1] ?? "";
 console.log(format.includes("HostConfig.Ulimits") ? "8192:8192" : "true"); process.exit(0);
}
if (args[0] === "exec" && args.includes("df")) {
 console.log("Filesystem 1M-blocks Used Available Use% Mounted\\nfixture 10000 1 9999 1% /home/slice"); process.exit(0);
}
if (args[0] === "exec" && args.at(-1) === "/opt/chariox-slice/start-runtime.sh") {
 writeFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify(args.filter(value => /^CHARIOX_SLICE_DAEMON_(ID|ALIAS)=/.test(value))));
}
process.exit(0);
`, { mode: 0o700 });
    const log = join(root, "docker.json");
    const result = spawnSync("bash", [provisioner, "start-runtime"], {
      encoding: "utf8", timeout: 60_000,
      env: { ...isolatedEnvironment(root), CHARIOX_TEST_DOCKER_LOG: log,
        CHARIOX_SLICE_NAME: "chariox-synthetic-identity", CHARIOX_SLICE_ID: "slice-1", CHARIOX_SLICE_OWNER_KERNEL_ID: "kernel-a", CHARIOX_SLICE_OWNER_MACHINE_ID: "machine-a", CHARIOX_SLICE_RELAY_TOKEN: "synthetic",
        CHARIOX_SLICE_DAEMON_ID: fixture.workerKernelRef, CHARIOX_SLICE_DAEMON_ALIAS: `slice:${fixture.localName}` },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(await readFile(log, "utf8")).sort(), [
      `CHARIOX_SLICE_DAEMON_ID=${fixture.workerKernelRef}`, `CHARIOX_SLICE_DAEMON_ALIAS=slice:${fixture.localName}`,
    ].sort());
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("the production kernel startup function uses canonical ID for normal and probe launches", async () => {
  const source = await readFile(runtime, "utf8");
  const startup = source.match(/^start_slice_kernel\(\) \{[\s\S]*?^\}/m)?.[0];
  const readiness = source.match(/^wait_for_kernel_auth_consumption\(\) \{[\s\S]*?^\}/m)?.[0];
  assert.ok(startup);
  assert.ok(readiness);
  const root = await mkdtemp(join(tmpdir(), "chariox-slice-kernel-identity-"));
  try {
    for (const probe of [false, true]) {
      const script = `set -eu
ROOT="$HOME"; KERNEL_LOCAL_AUTH_FILE="$HOME/auth-fixture"; kernel_relay_env=()
KERNEL_PORT=1; MCP_PORT=2; CODEX_PORT_RANGE=3; OPENCODE_PORT_RANGE=4; PROVIDER_BIND_HOST=fixture
DAEMON_ID="$TEST_DAEMON_ID"; DAEMON_ALIAS=slice:drill; MACHINE_ID=slice:synthetic; MACHINE_ALIAS=drill
SLICE_ID=slice-1; SLICE_OWNER_KERNEL_ID=kernel-a; SLICE_OWNER_MACHINE_ID=machine-a; SLICE_OWNER_PUBLIC_KEY=public-fixture
CAPABILITY_ISOLATION_ROOT="$HOME/capabilities"; BROWSER_DOWNLOAD_DIR="$HOME/downloads"; BROWSER_UPLOAD_ROOTS="$HOME/uploads"; PROVIDER_HOME="$HOME/provider"
dd() { printf synthetic-auth; }; sleep() { :; }; wait_for_screen_session() { :; }
screen() {
 for value in "$@"; do
  case "$value" in CHARIOX_DAEMON_ID=*|CHARIOX_DAEMON_ALIAS=*|CHARIOX_ACCEPT_REMOTE_LEASES=*) printf '%s\n' "$value";; esac
 done
 rm -f "$KERNEL_LOCAL_AUTH_FILE"
}
${readiness}
${startup}
start_slice_kernel ${probe ? "CHARIOX_ACCEPT_REMOTE_LEASES=0" : ""}
`;
      const result = spawnSync("bash", ["-c", script], { encoding: "utf8", timeout: 10_000,
        env: { ...isolatedEnvironment(root), TEST_DAEMON_ID: fixture.workerKernelRef } });
      assert.equal(result.status, 0, result.stderr);
      assert.ok(result.stdout.split("\n").includes(`CHARIOX_DAEMON_ID=${fixture.workerKernelRef}`), result.stdout);
      assert.ok(result.stdout.split("\n").includes("CHARIOX_DAEMON_ALIAS=slice:drill"));
      if (probe) assert.ok(result.stdout.split("\n").includes("CHARIOX_ACCEPT_REMOTE_LEASES=0"));
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});
