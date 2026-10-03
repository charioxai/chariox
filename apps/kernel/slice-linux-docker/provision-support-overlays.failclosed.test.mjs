import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url));
const requiredOverlayDestinations = [
  ["runtime launcher", "/opt/chariox-slice/start-runtime.sh"],
  ["provider launcher", "/opt/chariox-slice/start-providers.sh"],
  ["screen launcher", "/opt/chariox-slice/slice-screen.sh"],
  ["screen text finder", "/opt/chariox-slice/slice-text-finder.py"],
  ["provider bridge", "/opt/chariox-slice/provider-port-bridge.mjs"],
  ["provider isolation probe", "/opt/chariox-slice/managed-provider-isolation-probe.mjs"],
  ["provider isolation wrapper", "/opt/chariox-slice/managed-provider-isolation-probe-wrapper.sh"],
  ["screen validator", "/opt/chariox-slice/validate-screen.sh"],
  ["browser CDP helper", "/opt/chariox-slice/browser-cdp.mjs"],
  ["browser quota store", "/opt/chariox-slice/browser-upload-store.py"],
  ["browser lifecycle owner", "/opt/chariox-slice/browser-lifecycle.py"],
  ["Selkies lifecycle", "/opt/chariox-slice/slice-selkies.py"],
  ["Selkies streaming", "/opt/chariox-slice/slice-selkies-stream.py"],
  ["Selkies viewer module", "/opt/chariox-slice/selkies_viewers.py"],
  ...[
    "browser-controller-actions.mjs",
    "browser-controller-cdp.mjs",
    "browser-controller-resources.mjs",
    "browser-controller-cookie-fence.mjs",
    "browser-controller-dialogs.mjs",
    "browser-controller-compatibility.mjs",
    "browser-controller-events.mjs",
    "browser-controller-files.mjs",
    "browser-controller-upload-staging.mjs",
    "browser-controller-frames.mjs",
    "browser-controller-history.mjs",
    "browser-controller-permissions.mjs",
    "browser-controller-snapshot.mjs",
    "browser-controller.mjs",
  ].map(name => [`Browser Controller ${name}`, `/opt/chariox-slice/${name}`]),
  ...[
    "chrome-cookie-batch.mjs",
    "controller-cookie-import.mjs",
    "cookie-import-completion.mjs",
    "cookie-import-journal.mjs",
    "cookie-import-transaction.mjs",
    "production-destination.mjs",
  ].map(name => [`browser import ${name}`, `/opt/chariox-slice/browser-session-import/${name}`]),
];

test("each required overlay remains wired to a fail-closed refresh", async () => {
  const source = await readFile(provisioner, "utf8");
  const refresh = source.slice(source.indexOf("copy_required_slice_overlay() {"), source.indexOf("\nwait_for_container_running()"));
  assert.notEqual(refresh.indexOf("copy_required_slice_overlay() {"), -1);

  for (const [label, destination] of requiredOverlayDestinations) {
    const filename = destination.slice(destination.lastIndexOf("/") + 1);
    assert.ok(refresh.includes(filename), `${label} must remain in the support refresh`);
    if (label.startsWith("Browser Controller") || ["screen text finder", "provider bridge",
      "provider isolation probe", "provider isolation wrapper", "screen validator"].includes(label)) {
      const lines = refresh.split("\n");
      const commandIndex = lines.findIndex(line => line.includes(filename)
        && (line.includes("copy_required_slice_overlay") || line.includes("docker cp")));
      assert.notEqual(commandIndex, -1, `${label} must have an overlay copy`);
      if (!lines[commandIndex].includes("copy_required_slice_overlay")) {
        assert.match(lines[commandIndex + 1] ?? "", /\|\| fail/, `${label} must fail after a rejected copy`);
      }
    }
  }
  for (const filename of [
    "start-runtime.sh",
    "start-providers.sh",
    "slice-screen.sh",
    "slice-text-finder.py",
    "provider-port-bridge.mjs",
    "managed-provider-isolation-probe.mjs",
    "managed-provider-isolation-probe-wrapper.sh",
    "validate-screen.sh",
    "browser-cdp.mjs",
    "slice-selkies.py",
    "slice-selkies-stream.py",
    "selkies_viewers.py",
    "browser-controller-actions.mjs",
    "browser-controller-cdp.mjs",
  ]) {
    assert.ok(refresh.split("\n").some(line => line.includes("copy_required_slice_overlay") && line.includes(filename)),
      `${filename} must use the shared fail-closed copy helper`);
  }
  const importStart = refresh.indexOf("for browser_import_module in");
  const importLoop = refresh.slice(importStart, refresh.indexOf("\n  done", importStart));
  for (const [, destination] of requiredOverlayDestinations.filter(([label]) => label.startsWith("browser import"))) {
    const filename = destination.slice(destination.lastIndexOf("/") + 1);
    assert.ok(importLoop.includes(filename), `${filename} must remain in the import overlay list`);
  }
  assert.match(importLoop, /docker cp[\s\S]*?\|\| fail "failed to refresh required slice support overlay:/,
    "every browser import module must use the required fail-closed copy path");
});

test("recovery fails closed on representative required overlay errors before starting services", async t => {
  const root = await mkdtemp(join(tmpdir(), "chariox-support-overlay-"));
  try {
    await writeFakeDocker(root);
    await t.test("legacy browser admission refusal prevents package setup, every overlay and runtime startup", async () => {
      const result = await recover(root, { browserDisposition: "unowned" });
      assert.notEqual(result.status, 0, result.stdout + result.stderr);
      assert.match(result.stderr, /legacy or unowned Chromium/);
      assert.match(result.stderr, /\/slice stop fixture-slice.*\/slice start fixture-slice/);
      const calls = await readFile(join(root, "docker-calls"), "utf8");
      assert.doesNotMatch(calls, /(?:^|\n)cp\0/, "no overlay copy may precede browser admission");
      assert.doesNotMatch(calls, /apt-get -qq update/, "package preparation must follow browser admission");
      assert.doesNotMatch(calls, /(?:^|\n)(?:stop|kill|restart)\0/, "admission must not stop live provider turns");
      assert.equal(await runtimeStarted(root), false);
    });
    const rejectedDestinations = [
      ["runtime launcher", "/opt/chariox-slice/start-runtime.sh"],
      ["provider bridge", "/opt/chariox-slice/provider-port-bridge.mjs"],
      ["Selkies lifecycle", "/opt/chariox-slice/slice-selkies.py"],
      ["Browser Controller entrypoint", "/opt/chariox-slice/browser-controller.mjs"],
      ["browser import controller", "/opt/chariox-slice/browser-session-import/controller-cookie-import.mjs"],
    ];
    for (const [label, destination] of rejectedDestinations) {
      await t.test(`${label} copy failure`, async () => {
        const result = await recover(root, { failCopyDestination: `chariox-slice-fixture:${destination}` });
        assert.notEqual(result.status, 0, result.stdout + result.stderr);
        assert.match(result.stderr, /error: failed to refresh required slice support overlay:/);
        assert.equal(await runtimeStarted(root), false, "runtime startup must follow successful overlay refresh");
      });
    }

    await t.test("browser import directory failure", async () => {
      const result = await recover(root, { failImportDirectory: true });
      assert.notEqual(result.status, 0, result.stdout + result.stderr);
      assert.match(result.stderr, /error: failed to create required browser import runtime directory/);
      assert.equal(await runtimeStarted(root), false);
    });

    await t.test("required executable permission refresh failure", async () => {
      const result = await recover(root, { failRequiredChmod: true });
      assert.notEqual(result.status, 0, result.stdout + result.stderr);
      assert.match(result.stderr, /error: failed to set permissions on required slice support overlays/);
      assert.equal(await runtimeStarted(root), false);
    });

    await t.test("Selkies copy failure preserves explicit noVNC recovery", async () => {
      const result = await recover(root, {
        backend: "novnc",
        failCopyDestination: "chariox-slice-fixture:/opt/chariox-slice/slice-selkies.py",
      });
      assert.equal(result.status, 0, result.stderr);
      assert.equal(await runtimeStarted(root), true);
    });

    await t.test("optional taskbar copy failure remains nonfatal", async () => {
      const result = await recover(root, {
        failCopyDestination: "chariox-slice-fixture:/opt/chariox-slice/tint2rc",
      });
      assert.equal(result.status, 0, result.stderr);
      assert.equal(await runtimeStarted(root), true);
    });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

async function recover(root, options = {}) {
  const log = join(root, "docker-calls");
  const started = join(root, "runtime-started");
  await Promise.all([rm(log, { force: true }), rm(started, { force: true })]);
  const inherited = Object.fromEntries(Object.entries(process.env).filter(([name]) =>
    !name.startsWith("CHARIOX_SLICE_") && !name.startsWith("CHARIOX_TEST_")));
  const result = spawnSync("bash", [provisioner, "recover"], {
    encoding: "utf8",
    timeout: 60_000,
    env: {
      ...inherited,
      HOME: root,
      PATH: `${root}:${inherited.PATH}`,
      TMPDIR: root,
      CHARIOX_TEST_DOCKER_LOG: log,
      CHARIOX_TEST_RUNTIME_STARTED: started,
      CHARIOX_TEST_FAIL_COPY_DESTINATION: options.failCopyDestination ?? "",
      CHARIOX_TEST_FAIL_IMPORT_DIRECTORY: options.failImportDirectory ? "1" : "0",
      CHARIOX_TEST_FAIL_REQUIRED_CHMOD: options.failRequiredChmod ? "1" : "0",
      CHARIOX_TEST_BROWSER_DISPOSITION: options.browserDisposition ?? "clear",
      CHARIOX_SLICE_NAME: "chariox-slice-fixture",
      CHARIOX_SLICE_ID: "fixture-slice",
      CHARIOX_SLICE_OWNER_KERNEL_ID: "fixture-kernel",
      CHARIOX_SLICE_OWNER_MACHINE_ID: "fixture-machine",
      CHARIOX_SLICE_VIEWER_BACKEND: options.backend ?? "selkies",
      CHARIOX_SLICE_START_DESKTOP: "0",
      CHARIOX_SLICE_START_RUNTIME: "1",
      CHARIOX_SLICE_START_PROVIDER_SERVERS: "0",
      CHARIOX_SLICE_MIN_FREE_MB: "0",
    },
  });
  return result;
}

async function runtimeStarted(root) {
  try {
    return (await readFile(join(root, "runtime-started"), "utf8")).length > 0;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

async function writeFakeDocker(root) {
  await writeFile(join(root, "docker"), `#!/usr/bin/env bash
set -euo pipefail
{
  printf '%s\\0' "$@"
  printf '\\n'
} >> "$CHARIOX_TEST_DOCKER_LOG"

if [[ "\${1:-}" == info ]]; then
  if [[ "$*" == *'{{.ID}}'* ]]; then printf 'fixture-engine-1234\\n'; fi
  exit 0
fi
if [[ "\${1:-}" == container && "\${2:-}" == inspect ]]; then
  exit 0
fi
if [[ "\${1:-}" == inspect ]]; then
  case "$*" in
    *HostConfig.Ulimits*) printf '8192:8192\\n' ;;
    *State.Paused*) printf 'false\\n' ;;
    *State.Running*) printf 'true\\n' ;;
    *) printf '%s\\n' '${JSON.stringify([{ Id: "a".repeat(64), Image: `sha256:${"b".repeat(64)}`,
      Created: "2026-09-27T10:00:00Z", State: { Running: true, Paused: false, Restarting: false,
        Status: "running", Pid: 123, StartedAt: "2026-09-27T10:00:00Z", FinishedAt: "0001-01-01T00:00:00Z" },
      HostConfig: { PidMode: "" }, Config: { Env: ["HOME=/home/slice"], Labels: {
        "io.chariox.slice.id": "fixture-slice", "io.chariox.slice.owner-kernel-id": "fixture-kernel",
        "io.chariox.slice.owner-machine-id": "fixture-machine",
      } },
    }])}' ;;
  esac
  exit 0
fi
if [[ "\${1:-}" == cp ]]; then
  target="\${@: -1}"
  if [[ "$target" == "\${CHARIOX_TEST_FAIL_COPY_DESTINATION:-}" ]]; then
    printf 'fake Docker rejected copy destination %s\\n' "$target" >&2
    exit 97
  fi
fi
if [[ "\${1:-}" == exec ]]; then
  command_line=" $* "
  if [[ "\${5:-}" == python3 && "\${6:-}" == -c && "$command_line" == *profileProcessCount* ]]; then
    case "$CHARIOX_TEST_BROWSER_DISPOSITION" in
      clear) printf '%s\\n' '{"disposition":"clear","profileProcessCount":0}' ;;
      unowned) printf '%s\\n' '{"disposition":"unowned","profileProcessCount":1}' ;;
      *) exit 98 ;;
    esac
    exit 0
  fi
  if [[ "\${CHARIOX_TEST_FAIL_IMPORT_DIRECTORY:-0}" == 1 \\
    && "$command_line" == *"mkdir -p /opt/chariox-slice/browser-session-import"* ]]; then
    printf 'fake Docker rejected browser import directory creation\\n' >&2
    exit 97
  fi
  if [[ "\${CHARIOX_TEST_FAIL_REQUIRED_CHMOD:-0}" == 1 \\
    && "$command_line" == *"chmod +x"* \\
    && "$command_line" == *"/opt/chariox-slice/browser-controller.mjs"* ]]; then
    printf 'fake Docker rejected required overlay chmod\\n' >&2
    exit 97
  fi
  if [[ "$command_line" == *"df -Pm"* ]]; then
    printf 'Filesystem 1M-blocks Used Available Use%% Mounted\\nfixture 10000 1 9999 1%% /home/slice\\n'
  fi
  if [[ "$command_line" == *" -u slice "* \\
    && "\${@: -1}" == /opt/chariox-slice/start-runtime.sh ]]; then
    printf 'started\\n' >> "$CHARIOX_TEST_RUNTIME_STARTED"
  fi
fi
exit 0
`, { mode: 0o700 });
}
