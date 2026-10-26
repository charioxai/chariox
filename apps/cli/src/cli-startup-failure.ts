// MP-08 / MP-10 / MP-11: a fresh hosted terminal needs its own Cloud login.
import { CloudClientAuthError } from "./cloud-client-http.js"

export function cliStartupFailureText(error: unknown): string {
  if (error instanceof CloudClientAuthError) {
    if (["login_required", "signed_out", "client_revoked", "refresh_reuse_detected"].includes(error.code)) {
      return "Sign in with chariox cloud login (use --api-url for your own Cloud server), then retry this session."
    }
    if (error.code === "profile_conflict") {
      return "This terminal has a different Cloud account. Use a separate CHARIOX_HOME, sign in, then retry this session."
    }
  }
  return "Unable to attach to this session."
}
