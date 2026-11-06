// KR-06 verdict rules that do not need a running kernel.

// A lane the drill requested only counts as load if its provider run launched and at least one
// prompt cycle (SubmitPrompt, then CompletePrompt) finished; otherwise the missing SubmitPrompt
// latency distribution would read as zero and hide a provider-launch regression.
export function laneChecks(lanes) {
  return {
    lanes_launch_failed: { value: lanes.filter(lane => !lane.launch).length, limit: 0 },
    lanes_without_prompt_cycle: { value: lanes.filter(lane => !(lane.prompt_cycles > 0)).length, limit: 0 },
    // One turn can be in flight when the load stops; any more means completed turns left their task working.
    lanes_with_unsettled_tasks: { value: lanes.filter(lane => lane.unsettled_tasks > 1).length, limit: 0 },
  }
}

export function verdictOf(checks) {
  const red = Object.entries(checks).filter(([, check]) => check.value > check.limit).map(([name]) => name)
  return { red, verdict: red.length ? 'RED' : 'GREEN' }
}

// Least-squares slope (MB/min) of RSS samples [{ t_s, rss_kb }] taken after the warm-up. The kernel's
// start-up ramp (caches, allocator arenas) is not a leak; a leak is a slope that stays after it.
export function rssSlopeAfterWarmup(samples, warmupSeconds) {
  const tail = samples.filter(sample => sample.t_s >= warmupSeconds)
  if (tail.length < 2) return null
  const meanT = tail.reduce((sum, s) => sum + s.t_s, 0) / tail.length
  const meanR = tail.reduce((sum, s) => sum + s.rss_kb, 0) / tail.length
  const num = tail.reduce((sum, s) => sum + (s.t_s - meanT) * (s.rss_kb - meanR), 0)
  const den = tail.reduce((sum, s) => sum + (s.t_s - meanT) ** 2, 0)
  return den === 0 ? null : +((num / den) * 60 / 1024).toFixed(2)
}
