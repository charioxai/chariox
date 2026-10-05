import type { RelayCloudProfile } from "./preferences.js"

/** Private client/browser authority passed explicitly to control-plane adapters.
 * It is never a kernel status profile or a CLI preference. */
export type CloudControlProfile = RelayCloudProfile & { cloudSessionToken?: string }

export function cloudControlHeaders(profile: CloudControlProfile): HeadersInit {
  const session = profile.cloudSessionToken?.trim()
  if (!session) throw new Error("Cloud control-plane actions require a client/browser login; kernel enrollment does not confer human account authority")
  return {accept: "application/json", "content-type": "application/json", authorization: `Bearer ${session}`}
}
