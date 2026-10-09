// MP-08/MP-10/MP-11: optional telemetry must never prevent owned teardown.
export async function collectRelayDiagnostics(page, receipt) {
  if (!page) return;
  try {
    const count = await page.evaluate(() => mdBinaryRelayFrames);
    if (Number.isSafeInteger(count) && count >= 0) receipt.binary_relay_frames = count;
  } catch { /* Keep the last sample when the viewer never started or retired. */ }
}
