import assert from "node:assert/strict"
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

const helperUrl = new URL("../apps/kernel/slice-linux-docker/managed-publication-access.sh", import.meta.url)
const validatorUrl = new URL("../apps/kernel/slice-linux-docker/managed-publication-acl.awk", import.meta.url)
const validatorPath = fileURLToPath(validatorUrl)

const access = [
  "user::rwx",
  "user:999:rw-",
  "user:232072:rw-",
  "group::---",
  "mask::rwx",
  "other::---",
].join("\n")
const directoryAccess = access
  .replace("user:999:rw-", "user:999:rwx")
  .replace("user:232072:rw-", "user:232072:rwx")
const defaults = [
  "default:user::rwx",
  "default:user:999:rwx",
  "default:user:232072:rwx",
  "default:group::---",
  "default:mask::rwx",
  "default:other::---",
].join("\n")

function validate(mode, acl) {
  return spawnSync("awk", [
    "-v", `mode=${mode}`,
    "-v", "chariox_uid=999",
    "-v", "docker_uid=995",
    "-v", "mapped_slice_uid=232072",
    "-f", validatorPath,
  ], { input: `${acl}\n`, encoding: "utf8" })
}

test("managed publication ACL validation accepts only the exact principal set", () => {
  const traversal = "user::rwx\nuser:995:--x\ngroup::---\nmask::--x\nother::---"
  assert.equal(validate("traversal", traversal).status, 0)
  assert.equal(validate("repository", `${directoryAccess}\nuser:995:--x\n${defaults}`).status, 0)
  assert.equal(validate("directory", `${directoryAccess}\n${defaults}`).status, 0)
  assert.equal(validate("file", access).status, 0)

  for (const [mode, unexpected] of [
    ["file", `${access}\nuser:1234:r--`],
    ["file", `${access}\ngroup:1234:r--`],
    ["file", `${access}\nuser:999:rw-`],
    ["file", access.replace("group::---", "group::r--")],
    ["file", access.replace("other::---", "other::r--")],
    ["file", access.replace("mask::rwx", "mask::--x")],
    ["traversal", traversal.replace("mask::--x", "mask::rwx")],
    ["traversal", `${traversal}\nuser:999:rwx`],
    ["directory", `${directoryAccess}\n${defaults}\ndefault:user:1234:rwx`],
    ["directory", `${directoryAccess}\n${defaults}\ndefault:group:1234:r-x`],
    ["directory", `${directoryAccess}\nuser:995:--x\n${defaults}`],
  ]) {
    assert.notEqual(validate(mode, unexpected).status, 0, unexpected)
  }
})

test("managed publication helper reports recursive getfacl failure with a redacted code", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-publication-access-test-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const share = join(root, "share")
  const storage = join(share, "slices", "development", "slice-1")
  const destination = join(storage, "development")
  const repository = join(destination, "repository")
  const bin = join(root, "bin")
  await mkdir(repository, { recursive: true })
  await writeFile(join(repository, "file"), "content")
  await mkdir(bin)
  const source = (await readFile(helperUrl, "utf8"))
    .replace("share_root=/var/lib/chariox-slice-share", `share_root=${share}`)
    .replace(/\[ "\$\(grep -Fxc[\s\S]*?\n  \|\| fail "chariox-docker subordinate UID mapping is not pinned"/, ":")
  const helper = join(root, "managed-publication-access.sh")
  await writeFile(helper, source, { mode: 0o755 })
  await writeFile(join(root, "managed-publication-acl.awk"), await readFile(validatorUrl), { mode: 0o644 })
  await writeFile(join(bin, "getfacl"), `#!/bin/sh
for value do path=$value; done
if [ "$path" = "${join(repository, "file")}" ]; then exit 73; fi
if [ "$path" = "${repository}" ]; then
  printf '%s\\n' '${directoryAccess}' 'user:995:--x' '${defaults}'
else
  printf '%s\\n' 'user::rwx' 'user:995:--x' 'group::---' 'mask::--x' 'other::---'
fi
`, { mode: 0o755 })
  await chmod(join(bin, "getfacl"), 0o755)

  const result = spawnSync(helper, ["verify", storage, destination, repository], {
    encoding: "utf8",
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
  })
  assert.equal(result.status, 1)
  assert.equal(result.stderr, "managed-publication-access.sh: ACL_INSPECTION_FAILED\n")
  assert.doesNotMatch(result.stderr, new RegExp(root.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")))
})
