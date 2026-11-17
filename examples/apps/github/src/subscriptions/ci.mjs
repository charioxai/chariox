// `ci.finished` is derived: a check, suite or workflow run of a commit completed, and the commit's check runs now
// say that at least one exists and none is pending. One conditional request per delivery (the ETag makes a repeat free).
const FAILED = new Set(['failure', 'timed_out', 'cancelled', 'action_required', 'startup_failure', 'stale']);
/** How long a pull request may show no checks before a one-shot hears "no checks reported". */
export const NO_CHECKS_MS = 15 * 60_000;

export async function checkState(github, account, repo, sha) {
  const [owner, name] = repo.split('/');
  const { data } = await github.get(account, '/repos/{owner}/{repo}/commits/{ref}/check-runs', { owner, repo: name, ref: sha }, { query: { per_page: 100 } });
  const runs = data.check_runs ?? [];
  const failed = runs.filter(run => FAILED.has(run.conclusion)).map(run => run.name);
  return {
    total: runs.length, finished: runs.length > 0 && runs.every(run => run.status === 'completed'),
    failed, conclusion: failed.length ? 'failure' : 'success',
    line: failed.length ? `CI failed (${failed.length} of ${runs.length} checks): ${failed.join(', ')}` : `CI passed (${runs.length} checks)`,
  };
}
