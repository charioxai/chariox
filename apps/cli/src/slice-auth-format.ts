import type {
  SliceRecord,
} from "./cli-types.js"
import type {
  SliceProviderLogin,
} from "./slice-command-handler-types.js"

export function formatSliceProviderLogin(login: SliceProviderLogin): string {
  return [
    `slice auth login ${login.provider}: ${login.status}`,
    login.verification_url ? `url=${login.verification_url}` : "",
    login.user_code ? `code=${login.user_code}` : "",
    login.message,
  ].filter(Boolean).join("\n")
}

export function formatSliceProviderAuthActionResult(
  action: "import" | "remove",
  slice: SliceRecord,
  provider: string,
  status: string,
): string {
  const sliceRef = slice.name || slice.id
  if ((action === "import" && status === "imported") || (action === "remove" && status === "removed")) {
    return `slice auth ${action} ${provider}: ${status}`
  }
  if (status === "not_implemented") {
    const fallback = action === "import"
      ? `use /slice auth login ${sliceRef} ${provider} <account-profile>, open /slice screen ${sliceRef} to configure the account inside the slice, or update/restart the worker kernel if auth import should be available`
      : `open /slice screen ${sliceRef} to remove the provider account inside the slice, or update/restart the worker kernel if auth removal should be available`
    return `slice auth ${action} ${provider} is unavailable on this kernel. Next action: ${fallback}.`
  }
  return `slice auth ${action} ${provider} failed${status ? ` with status ${status}` : ""}. Next action: run /slice doctor ${sliceRef}, then retry or use /slice auth login ${sliceRef} ${provider} <account-profile>.`
}
