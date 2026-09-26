import { createRelayKeypair } from "@chariox/kernel-client/browser-relay-crypto"
import {
  getRoomEnvironmentSliceRequest,
  getSliceDisplayEndpointRequest,
  getSliceRequest,
  roomEnvironmentSliceBindingMinimumProtocolVersion,
} from "@chariox/kernel-client/ipc-requests"
import type {
  RoomEnvironmentSliceResponse,
  SliceDisplayEndpoint,
  SliceRecord,
} from "@chariox/kernel-client/kernel-types"

import { sendWithProtocolMinimum } from "./protocol-minimum-diagnostic.js"
import {
  evaluateSliceViewerAvailability,
  isSliceViewerFailureStaleOrOffline,
  safeSliceViewerDiagnostic,
  validateRoomBoundSliceIdentity,
  type SliceViewerAvailability,
} from "./slice-viewer-availability.js"

type SliceResponse = { Slice: { slice: SliceRecord } }
type SliceDisplayEndpointResponse = { SliceDisplayEndpoint: { endpoint: SliceDisplayEndpoint } }

export type RoomViewerAvailabilityResult = {
  binding: NonNullable<RoomEnvironmentSliceResponse["RoomEnvironmentSlice"]["binding"]> | null
  availability: SliceViewerAvailability
}

export type RoomViewerAvailabilityDeps = {
  attachmentId?: () => string | null
  createViewerPublicKey?: () => Promise<string>
  isRelayConnection?: () => boolean
  send: <TResponse>(request: unknown) => Promise<TResponse>
}

export async function readRoomViewerAvailability(
  deps: RoomViewerAvailabilityDeps,
  sessionId: string,
  retryCommand: "/room status" | "/room view",
  checkEndpoint: boolean,
): Promise<RoomViewerAvailabilityResult> {
  let bindingResponse: RoomEnvironmentSliceResponse
  try {
    bindingResponse = await sendWithProtocolMinimum<RoomEnvironmentSliceResponse>(
      deps.send,
      getRoomEnvironmentSliceRequest(sessionId),
      {
        capability: "Room slice binding",
        requestVariant: "GetRoomEnvironmentSlice",
        minimumProtocolVersion: roomEnvironmentSliceBindingMinimumProtocolVersion,
      },
    )
  } catch (error) {
    if (error instanceof Error && /requires kernel protocol \d+ or newer:/i.test(error.message)) {
      throw error
    }
    return {
      binding: null,
      availability: {
        state: "error",
        message: formatRoomViewerReadError("the Room slice binding", error, retryCommand),
      },
    }
  }
  if (!bindingResponse || typeof bindingResponse !== "object" || !("RoomEnvironmentSlice" in bindingResponse)) {
    return {
      binding: null,
      availability: { state: "error", message: "Room slice binding response is malformed" },
    }
  }
  const binding = bindingResponse.RoomEnvironmentSlice.binding
  if (!binding) {
    return {
      binding: null,
      availability: {
        state: "unavailable",
        message: "Room Environment has no bound slice; bind a headed slice with /room bind <slice-ref>",
      },
    }
  }
  if (binding.session_id !== sessionId) {
    return {
      binding,
      availability: { state: "error", message: "Room Environment slice binding belongs to a different session" },
    }
  }

  let sliceResponse: SliceResponse
  try {
    sliceResponse = await deps.send<SliceResponse>(getSliceRequest(binding.slice_id))
  } catch (error) {
    return {
      binding,
      availability: {
        state: "error",
        message: formatRoomViewerReadError(`Room-bound slice ${binding.slice_id}`, error, retryCommand),
      },
    }
  }
  if (!sliceResponse || typeof sliceResponse !== "object" || !("Slice" in sliceResponse)) {
    return {
      binding,
      availability: { state: "error", message: `kernel returned a malformed record for Room slice ${binding.slice_id}` },
    }
  }
  const slice = sliceResponse.Slice.slice
  const identityError = validateRoomBoundSliceIdentity(sessionId, binding, slice)
  if (identityError) {
    return { binding, availability: { state: "error", message: identityError } }
  }

  const preflight = evaluateSliceViewerAvailability(slice)
  if (preflight.state !== "check_required" || !checkEndpoint) {
    return { binding, availability: preflight }
  }
  if (deps.isRelayConnection?.() && !deps.createViewerPublicKey) {
    return {
      binding,
      availability: {
        state: "error",
        message: "remote Room viewing requires this CLI's paired key-bound relay identity; issue a bound token with /relay cloud client-token",
      },
    }
  }
  const attachmentId = deps.attachmentId?.()
  if (!attachmentId) {
    return {
      binding,
      availability: {
        state: "error",
        message: `Room Environment display check for slice ${slice.id} requires an active Room attachment; reconnect and retry`,
      },
    }
  }
  try {
    const viewerPublicKey = deps.createViewerPublicKey
      ? await deps.createViewerPublicKey()
      : (await createRelayKeypair()).publicKeyBase64
    const endpointResponse = await deps.send<SliceDisplayEndpointResponse>(
      getSliceDisplayEndpointRequest(slice.id, {
        sessionId,
        attachmentId,
        viewerPublicKey,
      }),
    )
    if (!endpointResponse || typeof endpointResponse !== "object" || !("SliceDisplayEndpoint" in endpointResponse)) {
      return {
        binding,
        availability: {
          state: "error",
          message: `kernel returned a malformed display endpoint for Room slice ${slice.id}`,
        },
      }
    }
    return {
      binding,
      availability: evaluateSliceViewerAvailability(slice, {
        endpoint: endpointResponse.SliceDisplayEndpoint.endpoint,
      }),
    }
  } catch (error) {
    return {
      binding,
      availability: evaluateSliceViewerAvailability(slice, { error }),
    }
  }
}

function formatRoomViewerReadError(
  resource: string,
  error: unknown,
  retryCommand: "/room status" | "/room view",
): string {
  const detail = safeSliceViewerDiagnostic(error)
  const condition = isSliceViewerFailureStaleOrOffline(error) ? "; the kernel or worker connection appears stale or offline" : ""
  return `kernel could not read ${resource}${condition}${detail ? `: ${detail}` : ""}; check the worker and relay, then retry ${retryCommand}`
}
