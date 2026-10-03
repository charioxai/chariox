#!/usr/bin/env node

import { requestSliceDiskQuota } from "./slice-disk-quota-client.mjs"
import { SLICE_DISK_QUOTA_PROTOCOL_VERSION } from "./slice-disk-quota-contract.mjs"

try {
  const result = await requestSliceDiskQuota({
    protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
    operation: "probe",
  })
  process.stdout.write(`${JSON.stringify(result)}\n`)
  if (result.supported !== true) process.exitCode = 1
} catch (error) {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
  process.exitCode = 1
}
