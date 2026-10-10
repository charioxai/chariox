import { cloudControlFetch, type CloudControlProfile } from "./cloud-control-auth.js"

// MP-08 / MP-11: metadata-only human Cloud calls. Kernel credentials and
// context packages never enter this adapter.
export async function managedEnvironmentJson<T>(profile: CloudControlProfile, pathname: string, body?: object, signal?: AbortSignal): Promise<T> {
  const url = new URL(`${profile.apiUrl.replace(/\/+$/, "")}${pathname}`)
  if (!body) url.searchParams.set("accountId", profile.accountId)
  const response = await cloudControlFetch(profile, url, {...(body ? {method: "POST", body: JSON.stringify({...body, accountId: profile.accountId})} : {}), ...(signal ? {signal} : {})})
  const result = await response.json().catch(() => null)
  if (!response.ok) throw new Error(result?.error?.message ?? `Managed environment request failed with ${response.status}`)
  if (!result) throw new Error("Cloud returned an invalid managed environment response")
  return result as T
}
