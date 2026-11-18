// ship_changes: a Room's changed files to a pull request in steps, as the person approved them. The kernel runs the
// git steps (the account's token never reaches the App); the pull request is opened over GitHub's API as the same
// account. A step that fails stops the rest and says why, so the call can be run again with only the steps left.
const STEPS = ['commit', 'push', 'pr'];
const fail = (code, message) => { throw { code, message, retryable: false }; };
const words = error => (error?.payload ?? error)?.message ?? String(error);

export function createShip({ accounts, workspace, writes, links }) {
  return {
    async link_pull_request({ repo, number }, ctx) {
      if (!ctx.roomId) fail('NO_ROOM', 'Only a call made from a Room can link a pull request to it.');
      return { linked: await links.put(ctx, { repo, number }), repo, number };
    },

    async ship_changes(input, ctx) {
      const steps = STEPS.filter(step => (input.steps ?? STEPS).includes(step));
      const message = input.message?.trim() ?? '';
      if (steps.includes('commit') && !(input.files?.length && message)) fail('INVALID_ARGUMENT', 'Committing needs the files to commit and a message that says what changed.');
      if (steps.includes('pr') && !(input.title?.trim() || message)) fail('INVALID_ARGUMENT', 'Opening a pull request needs a title or a commit message to take it from.');

      const state = JSON.parse(await workspace.changes(input.worktree));
      if (!state.remote) fail('NO_GITHUB_REMOTE', 'This worktree has no GitHub repository to ship to: its origin must be an https://github.com/<owner>/<repo> remote.');
      const label = await accounts.resolve({ account: input.account, repo: state.remote });
      const result = { ok: true, repo: state.remote, branch: state.branch, steps: [] };
      const run = async (step, action) => {
        if (!result.ok) { result.steps.push({ step, state: 'not_run' }); return; }
        try {
          result.steps.push({ step, state: 'done', ...(await action()) });
        } catch (error) {
          result.ok = false;
          result.steps.push({ step, state: 'failed', message: words(error) });
        }
      };

      if (steps.includes('commit')) {
        await run('commit', async () => {
          const done = JSON.parse(await workspace.commit(input.worktree, input.files, message, input.branch ?? null, label));
          result.branch = done.branch;
          return { commit: done.commit, branch: done.branch, createdBranch: done.createdBranch };
        });
      }
      if (steps.includes('push')) {
        await run('push', async () => {
          const done = JSON.parse(await workspace.push(input.worktree, label));
          result.branch = done.branch;
          return { commit: done.commit, branch: done.branch };
        });
      }
      if (steps.includes('pr')) {
        await run('pr', async () => {
          const { pull } = await writes.open_pull_request({
            account: label, repo: state.remote, head: result.branch,
            title: input.title?.trim() || message.split('\n')[0], body: input.body ?? '', draft: Boolean(input.draft),
            ...(input.base ? { base: input.base } : {}),
          });
          result.pull = pull;
          await links.put(ctx, { repo: state.remote, number: pull.number, branch: result.branch }).catch(() => {});
          return { number: pull.number, url: pull.url };
        });
      }
      return result;
    },
  };
}
