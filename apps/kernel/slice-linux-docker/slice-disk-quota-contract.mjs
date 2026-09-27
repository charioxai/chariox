export const SLICE_DISK_QUOTA_PROTOCOL_VERSION = 1
export const SLICE_DISK_QUOTA_DATA_ROOT = "/var/lib/chariox-docker/data"
export const SLICE_DISK_QUOTA_STATE_PATH = "/var/lib/chariox-slice-disk-quota/reservations.json"
export const SLICE_DISK_QUOTA_SOCKET_PATH = "/run/chariox-slice-disk-quota/allocator.sock"
export const SLICE_DISK_QUOTA_DOCKER_HOST = "unix:///run/chariox-docker/docker.sock"
export const SLICE_DISK_QUOTA_PROJECT_ID_MIN = 1_073_741_824
export const SLICE_DISK_QUOTA_PROJECT_ID_MAX = 2_147_483_647
export const SLICE_DISK_QUOTA_HOST_RESERVE_MIN_BYTES = 2 * 1024 * 1024 * 1024
export const SLICE_DISK_QUOTA_FRAME_MAX_BYTES = 16 * 1024
export const SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS = 15 * 60 * 1000

const ID_FIELD = /^[a-zA-Z0-9_.:-]{1,180}$/
const CONTAINER_NAME = /^chariox-slice-[a-z0-9][a-z0-9-]{0,100}$/
const PROJECT_ID_MIN = SLICE_DISK_QUOTA_PROJECT_ID_MIN
const PROJECT_ID_MAX = SLICE_DISK_QUOTA_PROJECT_ID_MAX
const MEBIBYTE = 1024 * 1024
const MAX_LIMIT_BYTES = 4_294_967_295 * MEBIBYTE

function fail(message) {
  throw new Error(message)
}

function exactKeys(value, expected, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail(`${label} is invalid`)
  const actual = Object.keys(value).sort()
  const wanted = [...expected].sort()
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    fail(`${label} contains unsupported fields`)
  }
}

export function validateSliceDiskQuotaIdentity(identity) {
  exactKeys(identity, ["ownerKernelId", "ownerMachineId", "sliceId", "containerName", "homeVolumeName"], "quota identity")
  for (const key of ["ownerKernelId", "ownerMachineId", "sliceId"]) {
    if (typeof identity[key] !== "string" || !ID_FIELD.test(identity[key])) fail(`quota identity ${key} is invalid`)
  }
  if (
    typeof identity.containerName !== "string" ||
    !CONTAINER_NAME.test(identity.containerName) ||
    identity.homeVolumeName !== `${identity.containerName}-home`
  ) {
    fail("quota container and home-volume identity are invalid")
  }
  return identity
}

export function validateSliceDiskQuotaRequest(request) {
  if (!request || typeof request !== "object" || Array.isArray(request)) fail("quota request is invalid")
  if (request.protocolVersion !== SLICE_DISK_QUOTA_PROTOCOL_VERSION) fail("quota protocol version is unsupported")
  switch (request.operation) {
    case "reserve": {
      exactKeys(request, ["protocolVersion", "operation", "identity", "limits"], "quota reserve request")
      validateSliceDiskQuotaIdentity(request.identity)
      exactKeys(request.limits, ["writableLayerBytes", "persistentHomeBytes"], "quota limits")
      for (const name of ["writableLayerBytes", "persistentHomeBytes"]) {
        const bytes = request.limits[name]
        if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > MAX_LIMIT_BYTES || bytes % MEBIBYTE !== 0) {
          fail(`quota ${name} must be a positive whole MiB value`)
        }
      }
      return request
    }
    case "apply_home":
    case "apply_layer":
    case "status":
    case "verify":
    case "release":
      exactKeys(request, ["protocolVersion", "operation", "identity"], `quota ${request.operation} request`)
      validateSliceDiskQuotaIdentity(request.identity)
      return request
    case "ensure_before_start":
      exactKeys(request, ["protocolVersion", "operation", "containerName"], "quota start request")
      if (typeof request.containerName !== "string" || !CONTAINER_NAME.test(request.containerName)) {
        fail("quota start container name is invalid")
      }
      return request
    case "probe":
      exactKeys(request, ["protocolVersion", "operation"], "quota probe request")
      return request
    default:
      fail("quota operation is not allowed")
  }
}

export function validateSliceDiskQuotaState(state) {
  exactKeys(state, ["schemaVersion", "nextProjectId", "reservations"], "quota state")
  if (state.schemaVersion !== SLICE_DISK_QUOTA_PROTOCOL_VERSION) fail("quota state version is unsupported")
  if (!Number.isSafeInteger(state.nextProjectId) || state.nextProjectId < PROJECT_ID_MIN || state.nextProjectId > PROJECT_ID_MAX + 1) {
    fail("quota state project allocator is invalid")
  }
  if (!state.reservations || typeof state.reservations !== "object" || Array.isArray(state.reservations)) {
    fail("quota state reservations are invalid")
  }
  const projectIds = new Set()
  const containerNames = new Set()
  let maxProjectId = 0
  for (const [key, record] of Object.entries(state.reservations)) {
    exactKeys(record, ["identity", "limits", "projectIds"], "quota reservation")
    validateSliceDiskQuotaIdentity(record.identity)
    if (key !== sliceDiskQuotaIdentityKey(record.identity)) fail("quota reservation identity key is stale")
    if (containerNames.has(record.identity.containerName)) fail("quota reservation container name is duplicated")
    containerNames.add(record.identity.containerName)
    validateSliceDiskQuotaRequest({
      protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
      operation: "reserve",
      identity: record.identity,
      limits: record.limits,
    })
    exactKeys(record.projectIds, ["writableLayer", "persistentHome"], "quota project IDs")
    for (const id of Object.values(record.projectIds)) {
      if (!Number.isInteger(id) || id < PROJECT_ID_MIN || id > PROJECT_ID_MAX || projectIds.has(id)) {
        fail("quota project ID is stale or duplicated")
      }
      projectIds.add(id)
      if (id > maxProjectId) maxProjectId = id
    }
  }
  if (state.nextProjectId <= maxProjectId) fail("quota state allocator would replay an allocated project ID")
  return state
}

export function sliceDiskQuotaIdentityKey(identity) {
  validateSliceDiskQuotaIdentity(identity)
  return [identity.ownerKernelId, identity.ownerMachineId, identity.sliceId].join("\0")
}

export function isManagedSliceContainerName(value) {
  return typeof value === "string" && CONTAINER_NAME.test(value)
}
