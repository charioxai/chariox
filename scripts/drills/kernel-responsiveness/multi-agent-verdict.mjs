// KR-06 verdict rules that do not need a running kernel.

// A lane the drill requested only counts as load if its provider run launched and at least one
// prompt cycle (SubmitPrompt, then CompletePrompt) finished; otherwise the missing SubmitPrompt
// latency distribution would read as zero and hide a provider-launch regression.
export function laneChecks(lanes) {
  return {
    lanes_launch_failed: { value: lanes.filter(lane => !lane.launch).length, limit: 0 },
    lanes_without_prompt_cycle: { value: lanes.filter(lane => !(lane.prompt_cycles > 0)).length, limit: 0 },
  }
}

export function verdictOf(checks) {
  const red = Object.entries(checks).filter(([, check]) => check.value > check.limit).map(([name]) => name)
  return { red, verdict: red.length ? 'RED' : 'GREEN' }
}
