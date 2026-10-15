export const CLOUD_SIGNED_OUT_NOTICE = "Signed out. Sign in again: run /cloud login."

/** BY-03: the one definitive state when Cloud ends this terminal's login. */
export function presentCloudSignedOut(deps: { clearProfile: () => void; clearDirectory: () => void; notify: (message: string) => void }): void {
  deps.clearProfile()
  deps.clearDirectory()
  deps.notify(CLOUD_SIGNED_OUT_NOTICE)
}
