// Follows the App's change feed: `moved()` runs once at the start and then whenever documents
// changed since, whoever changed them (this window, another window or an agent).
const NOTHING = Number.MAX_SAFE_INTEGER

export function follower(call, moved) {
  let latest = null
  // `latest` is read before the first list, so a change in between is seen by the next poll.
  async function poll() {
    const feed = await call("view_changes", { since: latest ?? NOTHING })
    const first = latest === null
    if (!first && feed.latest === latest && !feed.dropped) return
    latest = feed.latest
    await moved()
  }
  return { poll }
}
