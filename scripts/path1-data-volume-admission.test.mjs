import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { test } from "node:test"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const dataRoot = "/var/lib/chariox-docker/data"

function section(contents, name) {
  const sections = new Map()
  let current = ""
  for (const line of contents.split(/\r?\n/)) {
    const header = line.match(/^\[(.+)\]$/)
    if (header) {
      current = header[1]
      sections.set(current, [])
    } else if (current && line.trim()) {
      sections.get(current).push(line)
    }
  }
  return sections.get(name) ?? []
}

test("Path-1 admission is nonpersistent and mount-bound consumers fail closed", async () => {
  const admission = await readFile(
    join(repositoryRoot, "apps/kernel/slice-linux-docker/chariox-data-volume-admission.service"),
    "utf8",
  )
  assert.ok(section(admission, "Service").includes("Type=oneshot"))
  assert.doesNotMatch(admission, /^RemainAfterExit=/m)
  assert.match(admission, /^RequiresMountsFor=\/var\/lib\/chariox-docker\/data$/m)
  const rootlessService = await readFile(
    join(repositoryRoot, "deploy/managed-kernel/chariox-rootless-docker.service"),
    "utf8",
  )
  assert.ok(section(rootlessService, "Unit").includes(
    "After=network-online.target chariox-slice-disk-quota-allocator.service",
  ))

  for (const [path, label] of [
    ["chariox-rootless-docker.path1-data-volume.conf", "rootless Docker"],
    ["chariox-slice-disk-quota-allocator.path1-data-volume.conf", "quota allocator"],
  ]) {
    const dropIn = await readFile(join(repositoryRoot, "apps/kernel/slice-linux-docker", path), "utf8")
    const directives = section(dropIn, "Unit")
    for (const required of [
      "Requires=chariox-data-volume-admission.service",
      "After=chariox-data-volume-admission.service",
      "After=var-lib-chariox\\x2ddocker-data.mount",
      "BindsTo=var-lib-chariox\\x2ddocker-data.mount",
      "AssertPathIsMountPoint=" + dataRoot,
    ]) {
      assert.ok(directives.includes(required), label + " drop-in must declare " + required)
    }
    if (label === "rootless Docker") {
      assert.ok(directives.includes("BindsTo=chariox-slice-disk-quota-allocator.service"))
    }
  }
})

function extractShellFunction(source, name) {
  const expression = new RegExp("^" + name + "\\(\\) \\{\\n[\\s\\S]*?^\\}", "m")
  const match = source.match(expression)
  assert.ok(match, "production helper " + name + " must exist")
  return match[0]
}

async function runRuntimeHelpers(context, {
  topology = "path1",
  failCall = "",
  operation = "start_path1_runtime_services",
  initiallyActive = [
    "chariox-rootless-docker.service",
    "user@1001.service",
    "chariox-slice-disk-quota-allocator.service",
    "chariox-data-volume-admission.service",
  ],
} = {}) {
  const upgrade = await readFile(join(repositoryRoot, "deploy/managed-kernel/upgrade-image.sh"), "utf8")
  const helperScript = [
    extractShellFunction(upgrade, "stop_path1_runtime_services"),
    extractShellFunction(upgrade, "start_path1_runtime_services"),
    operation,
  ].join("\n")
  const root = await mkdtemp(join(tmpdir(), "chariox-path1-admission-shell-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const bin = join(root, "bin")
  const state = join(root, "state")
  const trace = join(root, "trace.log")
  await mkdir(bin)
  await mkdir(state)

  const putExecutable = async (name, lines) => {
    const path = join(bin, name)
    await writeFile(path, lines.join("\n") + "\n")
    await chmod(path, 0o755)
  }
  await putExecutable("id", [
    "#!/bin/sh",
    "printf 'id %s\\n' \"$*\" >> \"$HARNESS_TRACE\"",
    "[ \"$1\" = \"-u\" ] && [ \"$2\" = \"chariox-docker\" ] || exit 64",
    "printf '1001\\n'",
  ])
  await putExecutable("systemctl", [
    "#!/bin/sh",
    "set -eu",
    "printf 'systemctl %s\\n' \"$*\" >> \"$HARNESS_TRACE\"",
    "action=$1",
    "case \"$action\" in",
    "  stop)",
    "    unit=$2",
    "    [ \"$FAIL_CALL\" != \"stop $unit\" ] || exit 1",
    "    /usr/bin/rm -f -- \"$HARNESS_STATE/active-$unit\"",
    "    ;;",
    "  start)",
    "    unit=$2",
    "    case \"$unit\" in",
    "      chariox-data-volume-admission.service)",
    "        for dependency in chariox-rootless-docker.service user@1001.service chariox-slice-disk-quota-allocator.service chariox-data-volume-admission.service; do",
    "          [ ! -f \"$HARNESS_STATE/active-$dependency\" ] || exit 20",
    "        done",
    "        [ \"$FAIL_CALL\" != \"start $unit\" ] || exit 1",
    "        /usr/bin/touch \"$HARNESS_STATE/admission-ran\"",
    "        ;;",
    "      chariox-slice-disk-quota-allocator.service)",
    "        [ -f \"$HARNESS_STATE/admission-ran\" ] || exit 21",
    "        [ \"$FAIL_CALL\" != \"start $unit\" ] || exit 1",
    "        /usr/bin/touch \"$HARNESS_STATE/active-$unit\"",
    "        ;;",
    "      chariox-rootless-docker.service)",
    "        [ -f \"$HARNESS_STATE/admission-ran\" ] || exit 22",
    "        [ -f \"$HARNESS_STATE/active-chariox-slice-disk-quota-allocator.service\" ] || exit 23",
    "        [ \"$FAIL_CALL\" != \"start $unit\" ] || exit 1",
    "        /usr/bin/touch \"$HARNESS_STATE/active-$unit\" \"$HARNESS_STATE/active-user@1001.service\"",
    "        ;;",
    "      *) exit 64 ;;",
    "    esac",
    "    ;;",
    "  is-active)",
    "    [ \"$2\" = \"--quiet\" ] || exit 64",
    "    [ -f \"$HARNESS_STATE/active-$3\" ]",
    "    ;;",
    "  *) exit 64 ;;",
    "esac",
  ])
  await putExecutable("mountpoint", [
    "#!/bin/sh",
    "printf 'mountpoint %s\\n' \"$*\" >> \"$HARNESS_TRACE\"",
    "[ \"$1\" = \"--quiet\" ] && [ \"$2\" = \"/var/lib/chariox-docker/data\" ]",
  ])
  for (const unit of initiallyActive) {
    await writeFile(join(state, "active-" + unit), "active\n")
  }

  const result = spawnSync("/bin/sh", ["-c", helperScript], {
    encoding: "utf8",
    env: {
      PATH: bin + ":/usr/bin:/bin",
      HARNESS_STATE: state,
      HARNESS_TRACE: trace,
      FAIL_CALL: failCall,
      managed_provider_topology: topology,
    },
  })
  const traceText = await readFile(trace, "utf8").catch(() => "")
  return {
    status: result.status,
    stderr: result.stderr,
    calls: traceText.trim() ? traceText.trim().split("\n") : [],
  }
}

const stopCalls = [
  "id -u chariox-docker",
  "id -u chariox-docker",
  "systemctl stop chariox-rootless-docker.service",
  "systemctl stop user@1001.service",
  "systemctl stop chariox-slice-disk-quota-allocator.service",
  "systemctl stop chariox-data-volume-admission.service",
]

test("extracted Path-1 helpers stop active services before admission and start in order", async (context) => {
  const result = await runRuntimeHelpers(context)
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(result.calls, [
    ...stopCalls,
    "systemctl start chariox-data-volume-admission.service",
    "systemctl start chariox-slice-disk-quota-allocator.service",
    "systemctl start chariox-rootless-docker.service",
    "systemctl is-active --quiet chariox-slice-disk-quota-allocator.service",
    "systemctl is-active --quiet chariox-rootless-docker.service",
    "systemctl is-active --quiet user@1001.service",
    "mountpoint --quiet /var/lib/chariox-docker/data",
  ])
})

test("admission failure prevents Path-1 allocator and Docker starts", async (context) => {
  const result = await runRuntimeHelpers(context, {
    failCall: "start chariox-data-volume-admission.service",
  })
  assert.equal(result.status, 1)
  assert.deepEqual(result.calls, [
    ...stopCalls,
    "systemctl start chariox-data-volume-admission.service",
  ])
})

test("Path-1 stop failure prevents admission and all service starts", async (context) => {
  const failedStop = "systemctl stop chariox-slice-disk-quota-allocator.service"
  const result = await runRuntimeHelpers(context, {
    failCall: "stop chariox-slice-disk-quota-allocator.service",
  })
  assert.equal(result.status, 1)
  assert.deepEqual(result.calls, [
    ...stopCalls.slice(0, 4),
    failedStop,
  ])
})

test("quota allocator start failure prevents rootless Docker start", async (context) => {
  const result = await runRuntimeHelpers(context, {
    failCall: "start chariox-slice-disk-quota-allocator.service",
  })
  assert.equal(result.status, 1)
  assert.deepEqual(result.calls, [
    ...stopCalls,
    "systemctl start chariox-data-volume-admission.service",
    "systemctl start chariox-slice-disk-quota-allocator.service",
  ])
})

test("ordinary topology leaves the Path-1 runtime helpers as no-ops", async (context) => {
  const result = await runRuntimeHelpers(context, {
    topology: "shared_host",
    operation: "stop_path1_runtime_services\nstart_path1_runtime_services",
  })
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(result.calls, [])
})
