import { readNamespaceEntry } from "./protected-namespace-entry.mjs"
export { mappedSliceOwner, SLICE_CONTAINER_UID } from "./protected-namespace-entry.mjs"

export function managedRootlessSliceOwner() {
  // The broker runs inside the verified daemon namespace. Filesystem ownership
  // is expressed there: namespace control UID0, worker UID1001. Host subordinate
  // IDs are retained only in public entry proof, never used for namespace chown.
  return readNamespaceEntry().dataUid
}
