// MP-08/MP-11: a Vault fill that submitted its form succeeded when the page took the submit, including by
// replacing the document. This reports what the page did; the caller finishes the fill's protection.
const NATIVE_NAVIGATION_WAIT_MS = 8000;
const PAGE_HANDLED_WAIT_MS = 1000;
const LOAD_WAIT_MS = 5000;
const POLL_MS = 50;

// replaced(): the filled document or field is gone. page(): {url, title, ready} of the top-level page.
export async function awaitSubmitOutcome({ replaced, page, pageHandled, now = Date.now, sleep = ms => new Promise(resolve => setTimeout(resolve, ms)) }) {
  const navigationDeadline = now() + (pageHandled ? PAGE_HANDLED_WAIT_MS : NATIVE_NAVIGATION_WAIT_MS);
  let navigated = await replaced();
  while (!navigated && now() < navigationDeadline) {
    await sleep(POLL_MS);
    navigated = await replaced();
  }
  const loadDeadline = now() + LOAD_WAIT_MS;
  let state = await page().catch(() => null);
  while (navigated && !state?.ready && now() < loadDeadline) {
    await sleep(POLL_MS);
    state = await page().catch(() => null);
  }
  if (!state) throw new Error('browser page unavailable after submit');
  return { navigated, url: state.url, title: state.title };
}
