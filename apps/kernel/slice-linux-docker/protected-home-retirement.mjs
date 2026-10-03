import { validateSliceDiskQuotaIdentity } from "./slice-disk-quota-contract.mjs"
import { dockerObjectNotFound } from "./slice-disk-quota-admission.mjs"

// Explicit slice destruction retires only broker-retained, owner-labelled homes.
// Docker refuses removal of any volume still mounted by a container.
export function retireProtectedQuotaHomes(volumes, identity, docker) {
  for (const volume of volumes) {
    validateSliceDiskQuotaIdentity({...identity, homeVolumeName: volume})
    const result = docker(["volume", "inspect", volume])
    if (result.status === 1 && !result.error && !result.signal && dockerObjectNotFound(result.stderr, "volume", volume)) continue
    if (result.status !== 0) throw new Error("retained quota home inspection failed; reservation is preserved")
    const records = JSON.parse(result.stdout)
    const labels = records?.[0]?.Labels
    if (records.length !== 1 || records[0].Name !== volume || records[0].Driver !== "local"
        || labels?.["io.chariox.slice.id"] !== identity.sliceId
        || labels?.["io.chariox.slice.owner-kernel-id"] !== identity.ownerKernelId
        || labels?.["io.chariox.slice.owner-machine-id"] !== identity.ownerMachineId) {
      throw new Error("retained quota home ownership is unverified; reservation is preserved")
    }
    if (docker(["volume", "rm", volume]).status !== 0) {
      throw new Error("retained quota home is still in use; reservation is preserved")
    }
  }
}
