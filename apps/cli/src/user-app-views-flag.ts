/** Shared display-tier prototype opt-in; off unless explicitly set to 1. */
export function userAppViewsPrototypeEnabled(): boolean {
  return process.env.CHARIOX_USER_APP_VIEWS_PROTOTYPE === "1"
}
