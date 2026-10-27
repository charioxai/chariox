import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs"
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


test("MP-02/MP-08/MP-10/MP-11 config-parent preparation repairs old root ownership and refuses aliases", () => {
  const helper = script.match(/^ensure_rootless_config_parent\(\) \{\n[\s\S]*?^\}/m)?.[0]
  assert.ok(helper, "production config-parent helper exists")
  const root = mkdtempSync(join(tmpdir(), "lownits-rootless-config-"))
  try {
    const home = join(root, "home")
    const bin = join(root, "bin")
    mkdirSync(home)
    mkdirSync(bin)
    const fixtureUid = process.getuid() === 0 ? 61002 : process.getuid()
    const fixtureGid = process.getuid() === 0 ? 61002 : process.getgid()
    // Numeric fixture ownership avoids creating or modifying host service accounts.
    writeFileSync(join(bin, "install"), '#!/bin/bash\nargs=()\nwhile (( $# )); do case "$1" in -o|-g) shift 2 ;; *) args+=("$1"); shift ;; esac; done\n/usr/bin/install "${args[@]}" || exit $?\nexec /usr/bin/chown "$FIXTURE_UID:$FIXTURE_GID" "${args[-1]}"\n')
    chmodSync(join(bin, "install"), 0o755)
    const run = () => spawnSync("/bin/sh", ["-c", helper + "\nensure_rootless_config_parent"], {
      env: { PATH: bin + ":/usr/bin:/bin", ROOTLESS_HOME: home, FIXTURE_UID: String(fixtureUid), FIXTURE_GID: String(fixtureGid) }, encoding: "utf8",
    })
    mkdirSync(join(home, ".config"), { mode: 0o700 })
    const result = run()
    assert.equal(result.status, 0, result.stderr)
    assert.equal(statSync(join(home, ".config")).uid, fixtureUid)
    assert.equal(statSync(join(home, ".config")).mode & 0o777, 0o700)
    rmSync(join(home, ".config"), { recursive: true })
    symlinkSync(bin, join(home, ".config"))
    const alias = run()
    assert.equal(alias.status, 1)
    assert.match(alias.stderr, /config parent is not a real directory/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
