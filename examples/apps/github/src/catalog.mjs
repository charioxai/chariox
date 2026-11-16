// The one list of what this App offers. The signed tool and UI action declarations
// and the REST routes the manifest allows are generated from it (scripts/generate.mjs),
// so the three cannot drift: a test fails when a committed file differs from its output.
import { eventTypes } from './events.mjs';

const string = (extra = {}) => ({ type: 'string', ...extra });
const integer = (minimum, maximum) => ({ type: 'integer', minimum, ...(maximum ? { maximum } : {}) });
const object = (properties = {}, required = []) => ({
  type: 'object', additionalProperties: false, properties, ...(required.length ? { required } : {}),
});

export const ACCOUNT_PATTERN = '^[a-z0-9-]{1,32}$';
export const REPO_PATTERN = '^[A-Za-z0-9_.-]{1,100}/[A-Za-z0-9_.-]{1,100}$';
const account = string({ pattern: ACCOUNT_PATTERN, description: 'The account to use; the default account when absent.' });
const repo = string({ pattern: REPO_PATTERN, description: 'owner/name' });
const number = integer(1, 2147483647);
const page = integer(1, 1000);
const perPage = integer(1, 100);
const pull = { account, repo, number };
const sha = string({ pattern: '^[0-9a-f]{40}$', description: 'The full commit sha the person saw; GitHub refuses the call when the head moved.' });
const body = (extra = {}) => string({ maxLength: 65536, ...extra });
const names = (maximum, extra = {}) => ({ type: 'array', minItems: 1, maxItems: maximum, uniqueItems: true, items: string({ minLength: 1, maxLength: 100 }), ...extra });
const logins = { type: 'array', maxItems: 15, uniqueItems: true, items: string({ pattern: '^[A-Za-z0-9-]{1,39}$' }) };
const lineComments = { type: 'array', maxItems: 50, items: object({
  path: string({ minLength: 1, maxLength: 1024 }), line: integer(1), side: { enum: ['LEFT', 'RIGHT'] },
  start_line: integer(1), start_side: { enum: ['LEFT', 'RIGHT'] }, body: body({ minLength: 1 }),
}, ['path', 'line', 'body']) };
export const MERGE_METHODS = ['merge', 'squash', 'rebase'];

export const SCOPES = ['mine', 'review_requested', 'repo', 'org', 'all'];
export const PULL_STATES = ['open', 'closed', 'merged', 'all'];

const branch = string({ minLength: 1, maxLength: 255 });
const mergeInput = object({ ...pull, method: { enum: MERGE_METHODS }, sha, commit_title: string({ maxLength: 256 }), commit_message: body() }, ['repo', 'number', 'method', 'sha']);

/** Every tool is also the action of the same name that the window and agents run through the view. */
export const tools = [
  { name: 'list_accounts', effect: 'read', description: 'The GitHub accounts this App uses: label, login, status and API rate limit. Accounts are bound by the owner in the kernel; this lists the ones the App knows.', input: object() },
  { name: 'add_account', effect: 'write', description: 'Make the App use a GitHub account the owner bound in the kernel (label as bound). The App checks it by reading its user.', input: object({ account: string({ pattern: ACCOUNT_PATTERN }) }, ['account']) },
  { name: 'remove_account', effect: 'write', description: 'Stop using an account in this App. The owner\'s binding in the kernel is not changed.', input: object({ account: string({ pattern: ACCOUNT_PATTERN }) }, ['account']) },
  { name: 'set_default_account', effect: 'write', description: 'Use this account when a call names none.', input: object({ account: string({ pattern: ACCOUNT_PATTERN }) }, ['account']) },
  { name: 'set_repo_account', effect: 'write', description: 'Use this account for one repository, or the default again when account is absent.', input: object({ repo, account: string({ pattern: ACCOUNT_PATTERN }) }, ['repo']) },
  { name: 'get_status', effect: 'read', description: 'API rate limits of the accounts (core and search) and which accounts work.', input: object({ account }) },
  { name: 'list_repositories', effect: 'read', description: 'Repositories the account can see, most recently pushed first.', input: object({ account, org: string({ pattern: '^[A-Za-z0-9_.-]{1,100}$' }), query: string({ maxLength: 100 }), page, perPage }) },
  { name: 'list_pull_requests', effect: 'read', description: 'Pull requests: created by the account (mine), waiting for its review (review_requested), in repositories (repo), in an organization (org) or involving the account (all).', input: object({
    account,
    scope: { enum: SCOPES },
    repos: { type: 'array', maxItems: 20, uniqueItems: true, items: repo },
    org: string({ pattern: '^[A-Za-z0-9_.-]{1,100}$' }),
    state: { enum: PULL_STATES },
    draft: { enum: ['any', 'only', 'hide'] },
    labels: { type: 'array', maxItems: 10, items: string({ minLength: 1, maxLength: 100 }) },
    author: string({ pattern: '^[A-Za-z0-9-]{1,39}$' }),
    review: { enum: ['approved', 'changes_requested', 'required', 'none'] },
    query: string({ maxLength: 200 }),
    page, perPage,
  }, ['scope']) },
  { name: 'get_pull_request', effect: 'read', description: 'One pull request: title, description, branches, labels, reviewers, size and merge state.', input: object(pull, ['repo', 'number']) },
  { name: 'get_pull_request_summary', effect: 'read', description: 'What a list row shows beside the title: size, check state and review decision.', input: object(pull, ['repo', 'number']) },
  { name: 'get_pull_request_files', effect: 'read', description: 'The files of a pull request with their patches (a file without a patch is too large to show).', input: object({ ...pull, page, perPage }, ['repo', 'number']) },
  { name: 'get_pull_request_diff', effect: 'read', description: 'The unified diff of a pull request, or of one file, at most maxBytes (default 262144).', input: object({ ...pull, path: string({ maxLength: 1024 }), maxBytes: integer(1024, 1048576) }, ['repo', 'number']) },
  { name: 'list_commits', effect: 'read', description: 'The commits of a pull request, oldest first.', input: object({ ...pull, page, perPage }, ['repo', 'number']) },
  { name: 'get_checks', effect: 'read', description: 'Check runs and commit statuses of the pull request head, with the overall state.', input: object(pull, ['repo', 'number']) },
  { name: 'get_check_output', effect: 'read', description: 'What one check run reports: its title, summary, annotations (the failure messages with file and line) and, for a GitHub Actions job, its steps with their result. The text of the job log lives on GitHub storage, which this App does not reach.', input: object({ account, repo, check_run_id: integer(1) }, ['repo', 'check_run_id']) },
  { name: 'list_reviews', effect: 'read', description: 'Reviews of a pull request, who was asked to review, and the review decision.', input: object(pull, ['repo', 'number']) },
  { name: 'list_comments', effect: 'read', description: 'Comments of a pull request: conversation comments and comments on lines of the diff.', input: object({ ...pull, page, perPage }, ['repo', 'number']) },
  { name: 'get_mergeability', effect: 'read', description: 'Whether a pull request can be merged: conflicts, how far behind the base branch it is, protection of the base branch, auto-merge.', input: object(pull, ['repo', 'number']) },
  { name: 'list_branches', effect: 'read', description: 'Branches of a repository.', input: object({ account, repo, page, perPage }, ['repo']) },
  { name: 'get_compare', effect: 'read', description: 'How far two refs of a repository differ.', input: object({ account, repo, base: string({ minLength: 1, maxLength: 255 }), head: string({ minLength: 1, maxLength: 255 }) }, ['repo', 'base', 'head']) },

  { name: 'list_labels', effect: 'read', description: 'The labels of a repository.', input: object({ account, repo, page, perPage }, ['repo']) },
  { name: 'list_assignees', effect: 'read', description: 'The people who can be asked to review in a repository.', input: object({ account, repo, page, perPage }, ['repo']) },
  { name: 'preview_merge_pull_request', effect: 'read', description: 'What merging this pull request with this method and head sha would do; refuses what can never be merged.', input: mergeInput },
  { name: 'preview_close_pull_request', effect: 'read', description: 'What closing this pull request would do.', input: object(pull, ['repo', 'number']) },
  { name: 'preview_delete_branch', effect: 'read', description: 'What deleting this branch would do; refuses the default and protected branches.', input: object({ account, repo, branch }, ['repo', 'branch']) },

  // Low risk, no card.
  { name: 'comment', effect: 'write', description: 'Add a comment to the conversation of a pull request.', input: object({ ...pull, body: body({ minLength: 1 }) }, ['repo', 'number', 'body']) },
  { name: 'reply_to_review_comment', effect: 'write', description: 'Reply to a comment on a line of the diff.', input: object({ ...pull, comment_id: integer(1), body: body({ minLength: 1 }) }, ['repo', 'number', 'comment_id', 'body']) },
  { name: 'submit_review', effect: 'write', description: 'Review a pull request without a verdict: a summary and comments on lines of the diff. Approving or requesting changes are the actions of those names.', input: object({ ...pull, body: body(), comments: lineComments, sha }, ['repo', 'number']) },
  { name: 'add_labels', effect: 'write', description: 'Add labels to a pull request.', input: object({ ...pull, labels: names(20) }, ['repo', 'number', 'labels']) },
  { name: 'remove_labels', effect: 'write', description: 'Remove labels from a pull request.', input: object({ ...pull, labels: names(20) }, ['repo', 'number', 'labels']) },
  { name: 'request_reviewers', effect: 'write', description: 'Ask people or teams to review a pull request.', input: object({ ...pull, reviewers: logins, team_reviewers: { type: 'array', maxItems: 15, uniqueItems: true, items: string({ minLength: 1, maxLength: 100 }) } }, ['repo', 'number']) },
  { name: 'remove_reviewers', effect: 'write', description: 'Withdraw a request for review.', input: object({ ...pull, reviewers: logins, team_reviewers: { type: 'array', maxItems: 15, uniqueItems: true, items: string({ minLength: 1, maxLength: 100 }) } }, ['repo', 'number']) },
  { name: 'rerun_failed_checks', effect: 'write', description: 'Run the failed jobs of the checks of a pull request again.', input: object(pull, ['repo', 'number']) },

  // Visible to others, one approval by the person (once, for the session, or denied).
  { name: 'approve_pull_request', effect: 'approval', approvalReason: 'Approves the pull request in the name of the GitHub account', standing: { class: 'review', repo: 'repo', account: 'account' }, description: 'Approve a pull request, with the comments on lines of the diff collected for the review.', input: object({ ...pull, body: body(), comments: lineComments, sha }, ['repo', 'number']) },
  { name: 'request_changes', effect: 'approval', approvalReason: 'Asks for changes on the pull request in the name of the GitHub account', standing: { class: 'review', repo: 'repo', account: 'account' }, description: 'Review a pull request asking for changes, with comments on lines of the diff.', input: object({ ...pull, body: body({ minLength: 1 }), comments: lineComments, sha }, ['repo', 'number', 'body']) },
  { name: 'open_pull_request', effect: 'approval', approvalReason: 'Opens a pull request on GitHub', description: 'Open a pull request from a branch; the base is the default branch when none is given.', input: object({ account, repo, title: string({ minLength: 1, maxLength: 256 }), head: string({ minLength: 1, maxLength: 255 }), base: string({ minLength: 1, maxLength: 255 }), body: body(), draft: { type: 'boolean' } }, ['repo', 'title', 'head']) },
  { name: 'mark_ready_for_review', effect: 'approval', approvalReason: 'Takes the pull request out of draft so reviewers are notified', description: 'Mark a draft pull request ready for review.', input: object(pull, ['repo', 'number']) },
  { name: 'convert_to_draft', effect: 'approval', approvalReason: 'Turns the pull request back into a draft', description: 'Convert a pull request to a draft.', input: object(pull, ['repo', 'number']) },
  { name: 'update_branch', effect: 'approval', approvalReason: 'Merges the base branch into the pull request branch on GitHub', description: 'Update the branch of a pull request with the latest changes of its base branch.', input: object({ ...pull, sha }, ['repo', 'number']) },
  { name: 'reopen_pull_request', effect: 'approval', approvalReason: 'Reopens the pull request', description: 'Reopen a closed pull request.', input: object(pull, ['repo', 'number']) },
  { name: 'enable_auto_merge', effect: 'approval', approvalReason: 'Lets GitHub merge the pull request by itself once its requirements are met', description: 'Merge a pull request automatically when its requirements are met.', input: object({ ...pull, method: { enum: MERGE_METHODS } }, ['repo', 'number', 'method']) },
  { name: 'disable_auto_merge', effect: 'approval', approvalReason: 'Stops GitHub from merging the pull request by itself', description: 'Turn automatic merging of a pull request off.', input: object(pull, ['repo', 'number']) },

  // Hard to undo: the person confirms exactly this input on a card.
  { name: 'merge_pull_request', effect: 'person_confirmed', confirmationView: 'preview_merge_pull_request', standing: { class: 'merge', repo: 'repo', account: 'account' }, description: 'Merge a pull request with a method, only if its head is still the commit given.', input: mergeInput },
  { name: 'close_pull_request', effect: 'person_confirmed', confirmationView: 'preview_close_pull_request', standing: { class: 'close', repo: 'repo', account: 'account' }, description: 'Close a pull request without merging it.', input: object(pull, ['repo', 'number']) },
  { name: 'delete_branch', effect: 'person_confirmed', confirmationView: 'preview_delete_branch', standing: { class: 'delete_branch', repo: 'repo', account: 'account' }, description: 'Delete a branch of a repository. The default branch and protected branches are never deleted.', input: object({ account, repo, branch }, ['repo', 'branch']) },
];

/** The REST calls the App may make: the signed route table of the manifest. The owner's binding must allow the safety class. */
export const routes = [
  ['GET', '/user'],
  ['GET', '/rate_limit'],
  ['GET', '/user/repos'],
  ['GET', '/orgs/{org}/repos'],
  ['GET', '/search/issues'],
  ['GET', '/repos/{owner}/{repo}'],
  ['GET', '/repos/{owner}/{repo}/labels'],
  ['GET', '/repos/{owner}/{repo}/assignees'],
  ['GET', '/repos/{owner}/{repo}/pulls'],
  ['GET', '/repos/{owner}/{repo}/pulls/{number}'],
  ['GET', '/repos/{owner}/{repo}/pulls/{number}/files'],
  ['GET', '/repos/{owner}/{repo}/pulls/{number}/commits'],
  ['GET', '/repos/{owner}/{repo}/pulls/{number}/reviews'],
  ['GET', '/repos/{owner}/{repo}/pulls/{number}/comments'],
  ['GET', '/repos/{owner}/{repo}/issues/{number}/comments'],
  ['GET', '/repos/{owner}/{repo}/commits/{ref}/check-runs'],
  ['GET', '/repos/{owner}/{repo}/commits/{ref}/status'],
  ['GET', '/repos/{owner}/{repo}/check-runs/{check_run_id}'],
  ['GET', '/repos/{owner}/{repo}/check-runs/{check_run_id}/annotations'],
  ['GET', '/repos/{owner}/{repo}/actions/runs'],
  ['GET', '/repos/{owner}/{repo}/actions/jobs/{job_id}'],
  ['GET', '/repos/{owner}/{repo}/branches'],
  ['GET', '/repos/{owner}/{repo}/branches/{branch}'],
  ['GET', '/repos/{owner}/{repo}/branches/{branch}/protection'],
  ['GET', '/repos/{owner}/{repo}/compare/{basehead}'],
  ['POST', '/repos/{owner}/{repo}/issues/{number}/comments', 'write'],
  ['POST', '/repos/{owner}/{repo}/pulls/{number}/comments/{comment_id}/replies', 'write'],
  ['POST', '/repos/{owner}/{repo}/pulls/{number}/reviews', 'write'],
  ['POST', '/repos/{owner}/{repo}/issues/{number}/labels', 'write'],
  ['DELETE', '/repos/{owner}/{repo}/issues/{number}/labels/{name}', 'write'],
  ['POST', '/repos/{owner}/{repo}/pulls/{number}/requested_reviewers', 'write'],
  ['DELETE', '/repos/{owner}/{repo}/pulls/{number}/requested_reviewers', 'write'],
  ['POST', '/repos/{owner}/{repo}/actions/runs/{run_id}/rerun-failed-jobs', 'write'],
  ['POST', '/repos/{owner}/{repo}/pulls', 'write'],
  ['PUT', '/repos/{owner}/{repo}/pulls/{number}/update-branch', 'write'],
  ['POST', '/graphql', 'write'],
  ['PUT', '/repos/{owner}/{repo}/pulls/{number}/merge', 'destructive'],
  ['PATCH', '/repos/{owner}/{repo}/pulls/{number}', 'destructive'],
  ['DELETE', '/repos/{owner}/{repo}/git/refs/heads/{branch}', 'destructive'],
].map(([method, path, safety = 'read']) => ({ method, path, safety }));

const viewTools = [
  { name: 'view_actions', description: 'The actions this backend registers for its view', input: object() },
  { name: 'view_state', description: 'The state of this backend\'s view', input: object() },
  { name: 'view_changes', description: 'The changes this backend\'s view recorded after `since`', input: object({ since: integer(0) }) },
];

const own = [
  { name: 'get_automations', description: 'Read the outgoing event to automation names setting.', input: object() },
  { name: 'configure_automations', description: 'Replace the outgoing event to automation names setting. Configure each named automation in the kernel first. An empty list explicitly disables forwarding for that event.', input: 'automations' },
];

const automationNames = {
  type: 'object', additionalProperties: false,
  properties: Object.fromEntries(Object.keys(eventTypes).map(name => [`${name}_notification`, {
    type: 'array', uniqueItems: true, items: { type: 'string', minLength: 1, maxLength: 128 },
  }])),
};

export const toolsDocument = () => ({
  tools: [
    ...own.map(({ name, description, input }) => ({
      name, description, inputSchema: input === 'automations' ? object({ automations: automationNames }, ['automations']) : input,
    })),
    ...tools.map(({ name, description, input }) => ({ name, description, inputSchema: input })),
    ...viewTools.map(({ name, description, input }) => ({ name, description, inputSchema: input })),
  ],
});

export const uiActionsDocument = () => ({
  uiActions: tools.map(({ name, description, effect, input, approvalReason, confirmationView, standing }) => ({
    name, description, effect, ...(approvalReason ? { approvalReason } : {}), ...(confirmationView ? { confirmationView } : {}),
    ...(standing ? { standing } : {}), inputSchema: input,
  })),
});

export const networkCapability = () => ({
  origin: 'https://api.github.com',
  methods: [...new Set(routes.map(route => route.method))].sort(),
  credential: 'github',
  routes,
});
