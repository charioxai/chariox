// scripts/compile.mjs defines this when it compiles the CLI into the single
// release executable; it is undeclared when the CLI runs from source or dist.
declare const CHARIOX_RELEASE_VERSION: string | undefined

/** The bundle version of a compiled `chariox` release executable, else undefined. */
export const releaseVersion: string | undefined =
  typeof CHARIOX_RELEASE_VERSION === "string" ? CHARIOX_RELEASE_VERSION : undefined

/**
 * `chariox __claude-hook HANDLER` runs a native-Claude hook handler inside the
 * release executable, which ships without a separate `node` (release-main.ts).
 */
export const claudeHookCommand = "__claude-hook"
