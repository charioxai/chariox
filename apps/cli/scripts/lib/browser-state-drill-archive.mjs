import assert from "node:assert/strict"
import { access, readdir, writeFile } from "node:fs/promises"
import path from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { verifyRetainedRoomArchive } from "./room-provider-retention.mjs"

export function createBrowserStateArchiveFixture({ helper, localDev, runCommand, authorizationWaitMs = 600_000, wait = delay }) {
  if (helper !== undefined) {
    assert.equal(localDev, true, "operator archive fixture requires explicit local DEV")
    assert.ok(path.isAbsolute(helper), "operator archive fixture requires an absolute helper")
    assert.ok(Number.isSafeInteger(authorizationWaitMs) && authorizationWaitMs >= 0 && authorizationWaitMs <= 600_000)
  }
  const call = async (operation, state, allowPending = false) => {
    const result = await runCommand("sudo", ["-n", "/usr/bin/python3", helper, operation], {
      stdin: JSON.stringify(state), timeoutMs: 620_000,
    })
    if (allowPending && result.code === 2) return null
    assert.equal(result.code, 0, "isolated operator archive fixture refused")
    return JSON.parse(result.stdout)
  }
  return {
    corruptionRejection: localDev
      ? /archive integrity check failed(?::[\s\S]*managed saved home archive (?:metadata is invalid|digest does not match)|; broker-owned archive was left unchanged)/
      : /archive integrity check failed.*quarantined/,
    async verify(state) {
      if (!helper) return verifyRetainedRoomArchive(state)
      const receipt = await call("verify", state)
      assert.equal(receipt.private, true)
      assert.equal(receipt.readableArchive, true)
      assert.ok(Number.isSafeInteger(receipt.sizeBytes) && receipt.sizeBytes > 0)
      assert.match(receipt.sha256, /^[a-f0-9]{64}$/)
      return receipt
    },
    async corrupt(state) {
      if (!helper) return writeFile(state.home_archive_path, "deliberately corrupted backup archive")
      const deadline = Date.now() + authorizationWaitMs
      let receipt
      while (!(receipt = await call("corrupt", state, true))) {
        assert.ok(Date.now() < deadline, "root operator corruption authorization did not arrive")
        await wait(Math.min(1000, deadline - Date.now()))
      }
      assert.equal(receipt.corrupted, true)
    },
    async verifyRejectedArchive(state) {
      const receipt = helper ? await call("quarantine", state) : {
        archivePresent: await access(state.home_archive_path).then(() => true, () => false),
        quarantineCount: (await readdir(path.dirname(state.home_archive_path)))
          .filter(entry => entry.startsWith(`${path.basename(state.home_archive_path)}.corrupt-`)).length,
      }
      assert.equal(receipt.archivePresent, localDev === true,
        localDev ? "broker-owned corrupt archive must remain at its restore path" : "corrupt archive must leave its restore path")
      assert.equal(receipt.quarantineCount, localDev ? 0 : 1,
        localDev ? "broker-owned corrupt archive must not be moved by the kernel" : "corrupt archive must remain in one owned quarantine file")
    },
  }
}
