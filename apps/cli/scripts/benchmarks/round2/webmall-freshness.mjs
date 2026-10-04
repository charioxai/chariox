// MP-08 / MP-10 / MP-11 H6. Fixture preparation, not official-grader acceptance.
export async function sampleExistingPages(context, validate) {
  const rows = []
  // A read-only round trip pumps externally generated navigation events before
  // the evaluator's cached page.url gate. Do not open/navigate/alter any page.
  for (const page of context.pages()) {
    await page.title()
    if (page.url().startsWith('chrome-extension:')) continue
    rows.push({ observedUrl: page.url(), result: await validate(page) })
  }
  return rows
}
