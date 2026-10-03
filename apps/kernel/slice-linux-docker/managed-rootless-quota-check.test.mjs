import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

const script = readFileSync(new URL("./managed-rootless-service.sh", import.meta.url), "utf8")
const start = script.indexOf("quota_storage_ready() {")
const check = script.slice(start, script.indexOf("\n}\n", start) + 3)
const mount = "/var/lib/chariox-docker/data xfs rw,relatime,inode64,prjquota"

function quotaReady(findmntOutput) {
  const bin = mkdtempSync(join(tmpdir(), "rootless-quota-"))
  try {
    for (const [name, body] of [
      ["findmnt", `printf '%s\\n' ${findmntOutput.map(line => `'${line}'`).join(" ")}`],
      ["xfs_quota", "printf 'Project quota state on x\\n  Accounting: ON\\n  Enforcement: ON\\n'"],
      ["xfs_info", "echo 'naming =version 2 bsize=4096 ascii-ci=0, ftype=1'"],
      ["grep", 'exec /usr/bin/grep "$@"'],
      ["sort", 'exec /usr/bin/sort "$@"'],
    ]) {
      writeFileSync(join(bin, name), `#!/bin/sh\n${body}\n`)
      chmodSync(join(bin, name), 0o755)
    }
    return spawnSync("/bin/sh", ["-c", `ROOTLESS_DATA_ROOT=/var/lib/chariox-docker/data\n${check}\nquota_storage_ready`],
      { env: { PATH: bin } }).status
  } finally { rmSync(bin, { recursive: true, force: true }) }
}

test("Path-1 quota check accepts one mount listed twice by peer propagation", () => {
  assert.equal(quotaReady([mount]), 0)
  assert.equal(quotaReady([mount, mount]), 0)
})

test("Path-1 quota check still rejects distinct or non-quota mounts", () => {
  assert.equal(quotaReady([mount, "/var/lib/chariox-docker/data xfs rw,relatime,noquota"]), 1)
  assert.equal(quotaReady(["/var/lib/chariox-docker/data ext4 rw,relatime"]), 1)
})

test("fresh-image quota config keeps ~/.config owned by the rootless user", () => {
  const userConfig = script.indexOf('install -d -o chariox-docker -g chariox-docker -m 0700 "$ROOTLESS_HOME/.config"')
  const rootConfig = script.indexOf('install -d -o root -g root -m 0755 "$ROOTLESS_HOME/.config/docker"')
  assert.ok(userConfig > 0 && rootConfig > userConfig)
})
