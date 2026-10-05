// MP-08 / MP-10: only a proven pre-action harness failure admits a retry.
export function canRetryUnactedHarnessFailure(row) {
  return row.red === true && row.agentActed === false
    && !row.submissionRequestedAt && !row.promptId && !row.excluded
}
