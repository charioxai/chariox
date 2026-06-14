import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { promisify } from "node:util"

import {
  remoteHomeExtensionHetznerPreflightCommand,
} from "./remote-home-extension-hetzner-helpers.mjs"

const execFile = promisify(execFileWithCallback)

test("remote Hetzner preflight reports checkout commit skew before launch", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-hetzner-preflight-"))
  try {
    const command = remoteHomeExtensionHetznerPreflightCommand({
      hetznerRepo: path.join(rootDir, "missing-repo"),
      remoteRoot: path.join(rootDir, "remote-root"),
      workerWorktree: path.join(rootDir, "remote-root", "workspace"),
      expectedRepoHead: "2222222",
    })

    await assert.rejects(
      execFile("bash", ["-lc", command]),
      (error) => {
        assert.equal(error.code, 18)
        assert.match(error.stderr, /remote worker checkout `.*missing-repo` is at commit unknown, but home checkout expects 2222222/)
        assert.match(error.stderr, /Upgrade\/rebuild the remote worker checkout/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("remote Hetzner preflight reports insufficient remote filesystem capacity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-hetzner-preflight-"))
  try {
    const command = remoteHomeExtensionHetznerPreflightCommand({
      hetznerRepo: rootDir,
      remoteRoot: path.join(rootDir, "remote-root"),
      workerWorktree: path.join(rootDir, "remote-root", "workspace"),
      minFreeKb: Number.MAX_SAFE_INTEGER,
    })

    await assert.rejects(
      execFile("bash", ["-lc", command]),
      (error) => {
        assert.equal(error.code, 19)
        assert.match(error.stderr, /remote host filesystem for repo/)
        assert.match(error.stderr, /Free disk on the remote host/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

