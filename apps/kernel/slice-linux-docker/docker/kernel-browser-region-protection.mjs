// MP-08/MP-11: visual protection is applied by captureProtectedPage's exact
// Vault fill targets. Additional generic field/frame/shadow masks are obsolete.
export async function protectedHostRegions() { return []; }
export async function captureRegionMasks() {
  return {async afterCapture() { return []; }};
}
