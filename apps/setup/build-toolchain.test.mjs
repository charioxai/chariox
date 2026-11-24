// The micromamba bootstrap is read with Python, never with GNU tar -j, which needs the host's bzip2.
import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { mkdir, mkdtemp, readFile, realpath, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { promisify } from "node:util"
import { extractMicromamba } from "./build-toolchain.mjs"
const execute = promisify(execFile)

async function archiveWith(root, members) {
  const archive = join(root, "micromamba.tar.bz2")
  const script = `
import io, sys, tarfile
with tarfile.open(sys.argv[1], "w:bz2") as bundle:
    for name, body, kind in __import__("json").loads(sys.argv[2]):
        info = tarfile.TarInfo(name)
        if kind == "link":
            info.type = tarfile.SYMTYPE
            info.linkname = body
            bundle.addfile(info)
        else:
            info.size = len(body)
            info.mode = 0o644
            bundle.addfile(info, io.BytesIO(body.encode()))
`
  await execute("python3", ["-I", "-c", script, archive, JSON.stringify(members)])
  return archive
}

// A machine with Python, curl and systemd but no tar or bzip2 on PATH: only python3 is reachable.
async function pythonOnly(root) {
  const bin = join(root, "only-python")
  await mkdir(bin)
  await symlink(await realpath("/usr/bin/python3"), join(bin, "python3"))
  return { PATH: bin, HOME: root }
}

test("the bootstrap is extracted without the host's tar or bzip2", async t => {
  const root = await mkdtemp(join(await realpath(tmpdir()), "toolchain-bootstrap-")); t.after(() => rm(root, { recursive: true, force: true }))
  const archive = await archiveWith(root, [["info/index.json", "{}", "file"], ["bin/micromamba", "#!/bin/sh\necho micromamba\n", "file"]])
  const env = await pythonOnly(root)
  await assert.rejects(execute("tar", ["-xjf", archive, "bin/micromamba"], { cwd: root, env }), { code: "ENOENT" }, "tar is not on this PATH")
  const destination = join(root, "bootstrap", "bin", "micromamba")
  await extractMicromamba(archive, destination, { env, stdio: "ignore" })
  assert.equal(await readFile(destination, "utf8"), "#!/bin/sh\necho micromamba\n")
  assert.equal((await stat(destination)).mode & 0o777, 0o700)
  await assert.rejects(stat(join(root, "bootstrap", "info")), { code: "ENOENT" }, "only bin/micromamba is read")
})

test("an archive whose bin/micromamba is a link, or is missing, is refused", async t => {
  const root = await mkdtemp(join(await realpath(tmpdir()), "toolchain-bootstrap-")); t.after(() => rm(root, { recursive: true, force: true }))
  const env = await pythonOnly(root)
  for (const [name, members] of [
    ["link", [["bin/micromamba", "/etc/passwd", "link"]]],
    ["missing", [["bin/other", "x", "file"]]],
  ]) {
    const archive = await archiveWith(root, members)
    const destination = join(root, name, "bin", "micromamba")
    await assert.rejects(extractMicromamba(archive, destination, { env, stdio: "ignore" }), /python3 exited with status/, name)
    await assert.rejects(stat(destination), { code: "ENOENT" }, `${name}: nothing was written`)
  }
})
