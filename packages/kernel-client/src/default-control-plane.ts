/**
 * The one place a Chariox client names the connected service it signs in to when none is
 * configured. A deployment that uses another service sets `CHARIOX_CLOUD_API_URL`, a saved
 * preference, or `--api-url`; Setup is built with this value unless `--control-plane-url` says
 * otherwise (scripts/build-chariox-setup.mjs reads it from here).
 */
export const DEFAULT_CONTROL_PLANE_URL = "https://chariox.com"
