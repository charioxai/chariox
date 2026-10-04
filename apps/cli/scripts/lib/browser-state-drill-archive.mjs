import assert from "node:assert/strict"
import { access, readdir, writeFile } from "node:fs/promises"
import path from "node:path"
import { verifyRetainedRoomArchive } from "./room-provider-retention.mjs"

export function createBrowserStateArchiveFixture({ helper, localDev, runCommand }) {
  if (helper !== undefined) {
    assert.equal(localDev, true, "operator archive fixture requires explicit local DEV")
    assert.ok(path.isAbsolute(helper), "operator archive fixture requires an absolute helper")
  }
  const call = async (operation, state) => {
    const result = await runCommand("sudo", ["-n", "/usr/bin/python3", helper, operation], {
      stdin: JSON.stringify(state), timeoutMs: 620_000,
    })
    assert.equal(result.code, 0, "isolated operator archive fixture refused")
    return JSON.parse(result.stdout)
  }
  return {
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
      assert.equal((await call("corrupt", state)).corrupted, true)
    },
    async verifyQuarantine(state) {
      const receipt = helper ? await call("quarantine", state) : {
        archivePresent: await access(state.home_archive_path).then(() => true, () => false),
        quarantineCount: (await readdir(path.dirname(state.home_archive_path)))
          .filter(entry => entry.startsWith(`${path.basename(state.home_archive_path)}.corrupt-`)).length,
      }
      assert.equal(receipt.archivePresent, false, "corrupt archive must leave its restore path")
      assert.equal(receipt.quarantineCount, 1, "corrupt archive must remain in one owned quarantine file")
    },
  }
}
