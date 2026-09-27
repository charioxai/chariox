import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
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
      `AssertPathIsMountPoint=${dataRoot}`,
    ]) {
      assert.ok(directives.includes(required), `${label} drop-in must declare ${required}`)
    }
    if (label === "rootless Docker") {
      assert.ok(directives.includes("BindsTo=chariox-slice-disk-quota-allocator.service"))
    }
  }
})

test("Path-1 upgrade start helper stops runtime units and re-admits before starting them", async () => {
  const upgrade = await readFile(join(repositoryRoot, "deploy/managed-kernel/upgrade-image.sh"), "utf8")
  const stop = upgrade.match(/^stop_path1_runtime_services\(\) \{\n[\s\S]*?^\}/m)?.[0]
  const start = upgrade.match(/^start_path1_runtime_services\(\) \{\n[\s\S]*?^\}/m)?.[0]
  assert.ok(stop)
  assert.ok(start)
  for (const command of [
    "systemctl stop chariox-rootless-docker.service",
    "systemctl stop \"user@$path1_docker_uid.service\"",
    "systemctl stop chariox-slice-disk-quota-allocator.service",
    "systemctl stop chariox-data-volume-admission.service",
  ]) {
    assert.ok(stop.includes(command), `Path-1 stop helper must include ${command}`)
  }
  const orderedStartSteps = [
    "stop_path1_runtime_services || return 1",
    "systemctl start chariox-data-volume-admission.service || return 1",
    "systemctl start chariox-slice-disk-quota-allocator.service || return 1",
    "systemctl start chariox-rootless-docker.service || return 1",
  ]
  let previous = -1
  for (const step of orderedStartSteps) {
    const position = start.indexOf(step)
    assert.ok(position > previous, `Path-1 start helper must order ${step} after its predecessor`)
    previous = position
  }
})
