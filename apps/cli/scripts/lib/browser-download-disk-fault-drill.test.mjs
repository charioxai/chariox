import assert from "node:assert/strict"
import test from "node:test"

import {
  BROWSER_DOWNLOAD_DISK_CASE_IDS,
  BROWSER_DOWNLOAD_DISK_TEST_NAMES,
  buildBrowserDownloadDiskNodeArgs,
  parseBrowserDownloadDiskProbe,
} from "./browser-download-disk-fault-drill.mjs"

test("browser download disk drill selects only the five focused fault tests", () => {
  const args = buildBrowserDownloadDiskNodeArgs()
  assert.deepEqual(args.slice(0, 2), ["--test", "--test-concurrency=1"])
  assert.equal(args[2], `--test-name-pattern=${BROWSER_DOWNLOAD_DISK_TEST_NAMES.join("|")}`)
  assert.deepEqual(BROWSER_DOWNLOAD_DISK_CASE_IDS, [
    "fault.disk-pressure",
    "cleanup.resources",
  ])
})

test("browser download disk probe requires every safety assertion", () => {
  const output = BROWSER_DOWNLOAD_DISK_TEST_NAMES.map((name) => `✔ ${name}`).join("\n")
  assert.equal(parseBrowserDownloadDiskProbe(output).concurrentDownloadRechecked, true)
  assert.equal(parseBrowserDownloadDiskProbe(output).unmappedDownloadsCanceled, true)
  assert.throws(
    () => parseBrowserDownloadDiskProbe(output.replace(`✔ ${BROWSER_DOWNLOAD_DISK_TEST_NAMES[0]}`, "")),
    /did not pass/,
  )
})
