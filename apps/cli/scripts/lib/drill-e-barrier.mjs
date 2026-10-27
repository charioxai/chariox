import assert from "node:assert/strict"

// MP-08/MP-10: fixture-only synchronization; no runtime or controller policy.
export function createDrillEBarrier({ actors, now = Date.now,
  schedule = setTimeout, unschedule = clearTimeout } = {}) {
  assert.ok(Array.isArray(actors) && actors.length === 3 && new Set(actors).size === 3,
    "Drill E barrier requires three distinct actors")
  const ready = new Map()
  const pageHolds = []
  const waiters = []
  let readyRelease = null
  let pageRelease = null
  let overlapActionIds = []
  let timer = null
  function releaseReady(reason) {
    if (readyRelease) return
    readyRelease = { reason, atMs: now() }
    for (const resolve of waiters.splice(0)) resolve()
  }
  function releasePages(reason, actionIds = []) {
    if (reason === "observed_overlap") {
      assert.ok(actionIds.length === 3 && new Set(actionIds).size === 3,
        "page release requires three observed action identities")
    }
    if (pageRelease) return
    pageRelease = { reason, atMs: now() }
    overlapActionIds = [...actionIds]
    if (timer !== null) unschedule(timer)
    timer = null
    for (const hold of pageHolds) hold.resolve?.()
  }
  return {
    arrive(actor) {
      assert.ok(actors.includes(actor), "unknown barrier actor")
      assert.ok(!ready.has(actor), "duplicate barrier arrival")
      ready.set(actor, now())
      if (ready.size === actors.length) releaseReady("all_actors_ready")
      return readyRelease ? Promise.resolve() : new Promise(resolve => waiters.push(resolve))
    },
    hold(target) {
      const hold = { target, atMs: now(), resolve: null }
      pageHolds.push(hold)
      if (pageRelease) return Promise.resolve()
      if (timer === null) timer = schedule(() => releasePages("safety_deadline"), 4_000)
      return new Promise(resolve => { hold.resolve = resolve })
    },
    releasePages,
    close() { releaseReady("cleanup"); releasePages("cleanup") },
    evidence() {
      return { mp_items: ["MP-08", "MP-10"],
        ready: [...ready].map(([actor, atMs]) => ({ actor, atMs })),
        releaseReason: readyRelease?.reason ?? null, releasedAtMs: readyRelease?.atMs ?? null,
        pageHolds: pageHolds.map(({ target, atMs }) => ({ target, atMs })),
        pageReleaseReason: pageRelease?.reason ?? null, pagesReleasedAtMs: pageRelease?.atMs ?? null,
        overlapActionIds: [...overlapActionIds], pageHoldSafetyMs: 4_000, controllerBoundMs: 5_000,
      }
    },
  }
}
