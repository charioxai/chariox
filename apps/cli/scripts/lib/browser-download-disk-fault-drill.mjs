export const BROWSER_DOWNLOAD_DISK_CASE_IDS = Object.freeze([
  "fault.disk-pressure",
  "cleanup.resources",
])

export const BROWSER_DOWNLOAD_DISK_TEST_NAMES = Object.freeze([
  "downloads fail closed before changing browser policy when storage headroom is low",
  "downloads fail closed when storage capacity cannot be measured safely",
  "controller cancels every active download when slice storage drops below its reserve",
  "controller guards downloads before their frame is mapped to a tab",
  "a download arriving during disk-pressure cancellation receives a follow-up check",
])

export function buildBrowserDownloadDiskNodeArgs() {
  return [
    "--test",
    "--test-concurrency=1",
    `--test-name-pattern=${BROWSER_DOWNLOAD_DISK_TEST_NAMES.join("|")}`,
    "apps/kernel/slice-linux-docker/docker/browser-controller-files.test.mjs",
    "apps/kernel/slice-linux-docker/docker/browser-controller-cdp.test.mjs",
  ]
}

export function parseBrowserDownloadDiskProbe(output) {
  const text = String(output ?? "")
  const missing = BROWSER_DOWNLOAD_DISK_TEST_NAMES.filter(
    (name) => !text.includes(`✔ ${name}`),
  )
  if (missing.length > 0) {
    throw new Error(`browser download disk probe did not pass: ${missing.join(", ")}`)
  }
  return {
    schema: "chariox.browser_download_disk_probe.v1",
    configurationClosesBeforeWrite: true,
    unavailableCapacityClosesBeforeWrite: true,
    activeDownloadsCanceled: true,
    unmappedDownloadsCanceled: true,
    concurrentDownloadRechecked: true,
  }
}
