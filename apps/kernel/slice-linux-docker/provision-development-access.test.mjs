import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

const directory = fileURLToPath(new URL(".", import.meta.url))

// Runs prepare_development_workspace_access alone, with a docker stub that records its calls.
async function prepare(environment) {
  const root = await mkdtemp(join(tmpdir(), "chariox-development-access-"))
  try {
    const source = await readFile(join(directory, "provision-linux-docker-slice.sh"), "utf8")
    const script = join(root, "invoke.sh")
    await writeFile(script, source.replace(/^SCRIPT_DIR=.*$/m, `SCRIPT_DIR='${directory}'`).replace('main "$@"', "prepare_development_workspace_access"))
    await mkdir(join(root, "bin"))
    await writeFile(join(root, "bin/docker"), ["#!/bin/sh", `echo "$*" >> '${root}/docker.log'`, "cat > /dev/null", ""].join("\n"), { mode: 0o755 })
    const result = spawnSync("bash", [script], {
      encoding: "utf8", timeout: 30_000,
      env: { PATH: `${join(root, "bin")}:${process.env.PATH}`, TMPDIR: root, HOME: root, CHARIOX_SLICE_NAME: "chariox-slice-access-fixture",
        CHARIOX_SLICE_BUILD_CONTEXT_DIGEST: `sha256:${"b".repeat(64)}`, CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT: "1",
        CHARIOX_SLICE_DEVELOPMENT_MOUNT_0: "/var/lib/chariox-slice-share/slices/development/slice-1/development", ...environment },
    })
    const calls = (await readFile(join(root, "docker.log"), "utf8").catch(() => "")).trim().split("\n").filter(Boolean)
    return { status: result.status, calls, stderr: result.stderr }
  } finally { await rm(root, { recursive: true, force: true }) }
}

test("an ordinary Docker publication is prepared for the slice's worker inside the container", async () => {
  const result = await prepare({})
  assert.equal(result.status, 0, result.stderr)
  assert.equal(result.calls.length, 1)
  assert.match(result.calls[0], /^exec -i -u root chariox-slice-access-fixture python3 /)
})

test("a publication the managed broker host already prepared is not chowned again from inside the user namespace", async () => {
  const result = await prepare({ CHARIOX_SLICE_BROKER_PREPARED_PUBLICATION_ACCESS: "1" })
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(result.calls, [], "the rootless container cannot chown files owned by an unmapped host uid; the broker's ACLs already give the worker access")
})
