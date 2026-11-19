// The one list of what this App offers. The signed tool and UI action declarations
// and the REST routes the manifest allows are generated from it (scripts/generate.mjs),
// so the three cannot drift: a test fails when a committed file differs from its output.
import { OUTGOING, incoming, kinds } from './event-types.mjs';
import { TARGET_KINDS } from './subscriptions/model.mjs';

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

const kindName = { enum: Object.keys(kinds) };
const globs = { type: 'array', maxItems: 20, uniqueItems: true, items: string({ minLength: 1, maxLength: 200 }), description: 'Names or globs; an entry starting with ! excludes.' };
const scope = object({ kind: { enum: ['repos', 'org', 'all'] }, repos: { type: 'array', minItems: 1, maxItems: 50, uniqueItems: true, items: repo }, org: string({ pattern: '^[A-Za-z0-9_.-]{1,100}$' }) }, ['kind']);
const filters = object({ branches: globs, authors: globs, labels: globs, paths: globs, conclusion: { type: 'array', maxItems: 8, items: string() }, draft: { enum: ['any', 'only', 'hide'] }, excludeSelf: { type: 'boolean' } });
const target = object({ kind: { enum: TARGET_KINDS }, agentId: string(), roomId: string(), sessionId: string(), publicationRef: string(), queueRef: string() }, ['kind']);
const once = object({ pr: object({ repo, number }, ['repo', 'number']), expiresAt: string({ format: 'date-time', description: 'At most 7 days ahead; 4 hours when absent.' }) });
const subscriptionBody = { name: string({ maxLength: 100 }), account, scope, events: { type: 'array', minItems: 1, maxItems: 30, uniqueItems: true, items: kindName }, filters, target, once };
const subscriptionId = object({ id: string({ minLength: 1, maxLength: 64 }) }, ['id']);
const creating = 'Notify a target when GitHub events match: repositories, an organization or all, event names, filters, and a target (the person, an agent, a Room\'s agent or a workflow). The kernel delivers the events; nothing polls.';

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
  { name: 'get_check_output', effect: 'read', description: 'What one check run reports: its title, summary, annotations (the failure messages with file and line) and, for a GitHub Actions job, its steps with their result and, for a job, the end of its log (the last 200 lines; a log over 1 MiB is reported as unavailable).', input: object({ account, repo, check_run_id: integer(1) }, ['repo', 'check_run_id']) },
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

  { name: 'list_subscriptions', effect: 'read', description: 'The subscriptions of this App: what each watches and where it notifies.', input: object() },
  { name: 'get_subscription', effect: 'read', description: 'One subscription.', input: subscriptionId },
  { name: 'list_deliveries', effect: 'read', description: 'The last deliveries (newest first): delivered, filtered with the reason, failed or expired.', input: object({ subscription: string(), limit: integer(1, 200) }) },
  { name: 'get_event_sources', effect: 'read', description: 'Whether each watched repository has a live webhook source, and the state of the events connection.', input: object() },
  { name: 'list_connection_repositories', effect: 'read', description: 'Repositories the granted events connection delivers for.', input: object({ query: string({ maxLength: 100 }), cursor: string(), limit: integer(1, 100) }) },
  { name: 'connect_github_events', effect: 'write', description: 'Start the GitHub authorization for live events; returns the address where the person approves on GitHub.', input: object({ returnUrl: string() }) },
  { name: 'create_subscription', effect: 'write', description: `${creating} An agent subscribes itself with this; for any other target use create_subscription_for_others. A one-shot (once) ends after its first delivery or at its expiry, which also notifies once. "ci.finished" fires when every check of the pull request's head commit completed.`, input: object(subscriptionBody, ['events']) },
  { name: 'update_subscription', effect: 'write', description: 'Change a subscription you made (the person may change any).', input: object({ id: subscriptionId.properties.id, ...subscriptionBody }, ['id']) },
  { name: 'pause_subscription', effect: 'write', description: 'Stop a subscription without deleting it.', input: subscriptionId },
  { name: 'resume_subscription', effect: 'write', description: 'Start a paused subscription again.', input: subscriptionId },
  { name: 'delete_subscription', effect: 'write', description: 'Delete a subscription and the routes and targets only it used.', input: subscriptionId },
  { name: 'watch_pull_request', effect: 'write', description: 'Tell this agent when checks fail or pass, when someone else comments or reviews, until the pull request is merged or closed. No polling: the agent may end its turn.', input: object({ repo, number, account, on: { type: 'array', minItems: 1, uniqueItems: true, items: { enum: ['checks_failed', 'checks_passed', 'comment', 'review'] } }, until: { enum: ['merged_or_closed'] }, expiresAt: string({ format: 'date-time' }) }, ['repo', 'number']) },

  // Visible to others, one approval by the person (once, for the session, or denied).
  { name: 'approve_pull_request', effect: 'approval', approvalReason: 'Approves the pull request in the name of the GitHub account', standing: { class: 'review', repo: 'repo', account: 'account' }, description: 'Approve a pull request, with the comments on lines of the diff collected for the review.', input: object({ ...pull, body: body(), comments: lineComments, sha }, ['repo', 'number']) },
  { name: 'request_changes', effect: 'approval', approvalReason: 'Asks for changes on the pull request in the name of the GitHub account', standing: { class: 'review', repo: 'repo', account: 'account' }, description: 'Review a pull request asking for changes, with comments on lines of the diff.', input: object({ ...pull, body: body({ minLength: 1 }), comments: lineComments, sha }, ['repo', 'number', 'body']) },
  { name: 'open_pull_request', effect: 'approval', approvalReason: 'Opens a pull request on GitHub', description: 'Open a pull request from a branch; the base is the default branch when none is given.', input: object({ account, repo, title: string({ minLength: 1, maxLength: 256 }), head: string({ minLength: 1, maxLength: 255 }), base: string({ minLength: 1, maxLength: 255 }), body: body(), draft: { type: 'boolean' } }, ['repo', 'title', 'head']) },
  { name: 'ship_changes', effect: 'approval', approvalReason: 'Commits the named files of a worktree, pushes the branch and opens a pull request as the GitHub account', description: 'Take the changed files of a Room worktree to a pull request in steps (commit, push, pr; all three when none are named). The commit holds exactly the files named; work on the default branch moves to a new branch first. Never pushes the default branch or forces. A step that fails stops the rest and says why: run it again with only the steps left.', input: object({ worktree: string({ minLength: 1, description: 'The worktree path.' }), files: { type: 'array', minItems: 1, maxItems: 500, uniqueItems: true, items: string({ minLength: 1 }) }, message: string({ maxLength: 4096 }), branch: string({ minLength: 1, maxLength: 200, description: 'Commit to this branch instead of a new one.' }), steps: { type: 'array', minItems: 1, uniqueItems: true, items: { enum: ['commit', 'push', 'pr'] } }, title: string({ minLength: 1, maxLength: 256 }), body: body(), draft: { type: 'boolean' }, base: string({ minLength: 1, maxLength: 255 }), account }, ['worktree']) },
  { name: 'link_pull_request', effect: 'write', description: 'Record that this Room made a pull request, so the pull request shows which Room and agent it came from.', input: object({ repo, number }, ['repo', 'number']) },
  { name: 'mark_ready_for_review', effect: 'approval', approvalReason: 'Takes the pull request out of draft so reviewers are notified', description: 'Mark a draft pull request ready for review.', input: object(pull, ['repo', 'number']) },
  { name: 'convert_to_draft', effect: 'approval', approvalReason: 'Turns the pull request back into a draft', description: 'Convert a pull request to a draft.', input: object(pull, ['repo', 'number']) },
  { name: 'update_branch', effect: 'approval', approvalReason: 'Merges the base branch into the pull request branch on GitHub', description: 'Update the branch of a pull request with the latest changes of its base branch.', input: object({ ...pull, sha }, ['repo', 'number']) },
  { name: 'reopen_pull_request', effect: 'approval', approvalReason: 'Reopens the pull request', description: 'Reopen a closed pull request.', input: object(pull, ['repo', 'number']) },
  { name: 'enable_auto_merge', effect: 'approval', approvalReason: 'Lets GitHub merge the pull request by itself once its requirements are met', description: 'Merge a pull request automatically when its requirements are met.', input: object({ ...pull, method: { enum: MERGE_METHODS } }, ['repo', 'number', 'method']) },
  { name: 'disable_auto_merge', effect: 'approval', approvalReason: 'Stops GitHub from merging the pull request by itself', description: 'Turn automatic merging of a pull request off.', input: object(pull, ['repo', 'number']) },
  { name: 'create_subscription_for_others', effect: 'approval', approvalReason: 'Sends GitHub notifications to the person, another agent, a Room or a workflow', description: creating, input: object(subscriptionBody, ['events']) },
  { name: 'update_subscription_for_others', effect: 'approval', approvalReason: 'Changes where GitHub notifications go', description: 'Change a subscription, including its target.', input: object({ id: subscriptionId.properties.id, ...subscriptionBody }, ['id']) },

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
  // GitHub answers a job log with a redirect to its storage; the kernel follows it only to these hosts.
  ['GET', '/repos/{owner}/{repo}/actions/jobs/{job_id}/logs', 'read', ['*.blob.core.windows.net']],
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
].map(([method, path, safety = 'read', redirectHosts]) => ({ method, path, safety, ...(redirectHosts ? { redirectHosts } : {}) }));

const viewTools = [
  { name: 'view_actions', description: 'The actions this backend registers for its view', input: object() },
  { name: 'view_state', description: 'The state of this backend\'s view', input: object() },
  { name: 'view_account', description: 'The account a call of this window runs as: the one it names, the repository\'s, the default, else the first known', input: object({ account: { type: 'string' }, repo: { type: 'string' } }) },
  { name: 'view_changes', description: 'The changes this backend\'s view recorded after `since`', input: object({ since: integer(0) }) },
];

export const toolsDocument = () => ({
  tools: [
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

const eventPayload = type => object({
  source: object({ generator_id: { type: 'string', enum: ['dev.chariox.github'] }, connection_id: string(), event_type: { type: 'string', enum: [type] }, event_type_version: { type: 'integer', enum: [1] } },
    ['generator_id', 'connection_id', 'event_type', 'event_type_version']),
  occurred_at: string({ maxLength: 64 }), text: string({ maxLength: 65536 }), metadata: {}, artifacts: { type: 'array', maxItems: 32 }, reply_context: {},
}, ['source', 'occurred_at', 'text']);

/** One incoming event per generator type the routes deliver, and the one outgoing event every target is bound on. */
export const eventsDocument = () => ({
  events: [
    ...Object.entries(incoming).map(([name, type]) => ({ name, schemaVersion: 1, direction: 'incoming', payloadSchema: eventPayload(type) })),
    { name: OUTGOING, schemaVersion: 1, direction: 'outgoing', payloadSchema: object({
      subscription: object({ id: string(), name: string() }, ['id']), event: string(), repo: string(),
      number: { type: ['integer', 'null'] }, title: { type: ['string', 'null'] }, url: { type: ['string', 'null'] },
      conclusion: { type: ['string', 'null'] }, headSha: { type: ['string', 'null'] },
      outcome: { enum: ['expired', 'no_checks'] }, checks: object({ total: integer(0), failed: { type: 'array', items: string() } }),
    }, ['subscription', 'event', 'repo']) },
  ],
});

export const workspaceCapability = () => ({ credential: 'github' });
export const eventRoutingCapability = () => ({ generator: 'dev.chariox.github', incoming: Object.keys(incoming).sort(), maxRoutes: 200 });
