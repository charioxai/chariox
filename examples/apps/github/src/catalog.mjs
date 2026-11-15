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

export const SCOPES = ['mine', 'review_requested', 'repo', 'org', 'all'];
export const PULL_STATES = ['open', 'closed', 'merged', 'all'];

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
  { name: 'get_check_output', effect: 'read', description: 'What one check run reports: its title, summary and annotations (the failure messages with file and line). The job log itself lives on GitHub storage, which this App does not reach.', input: object({ account, repo, check_run_id: integer(1) }, ['repo', 'check_run_id']) },
  { name: 'list_reviews', effect: 'read', description: 'Reviews of a pull request, who was asked to review, and the review decision.', input: object(pull, ['repo', 'number']) },
  { name: 'list_comments', effect: 'read', description: 'Comments of a pull request: conversation comments and comments on lines of the diff.', input: object({ ...pull, page, perPage }, ['repo', 'number']) },
  { name: 'get_mergeability', effect: 'read', description: 'Whether a pull request can be merged: conflicts, how far behind the base branch it is, protection of the base branch, auto-merge.', input: object(pull, ['repo', 'number']) },
  { name: 'list_branches', effect: 'read', description: 'Branches of a repository.', input: object({ account, repo, page, perPage }, ['repo']) },
  { name: 'get_compare', effect: 'read', description: 'How far two refs of a repository differ.', input: object({ account, repo, base: string({ minLength: 1, maxLength: 255 }), head: string({ minLength: 1, maxLength: 255 }) }, ['repo', 'base', 'head']) },
];

/** The REST calls the App may make: the signed route table of the manifest, read-only for now. */
export const routes = [
  ['GET', '/user'],
  ['GET', '/rate_limit'],
  ['GET', '/user/repos'],
  ['GET', '/orgs/{org}/repos'],
  ['GET', '/search/issues'],
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
  ['GET', '/repos/{owner}/{repo}/branches'],
  ['GET', '/repos/{owner}/{repo}/branches/{branch}/protection'],
  ['GET', '/repos/{owner}/{repo}/compare/{basehead}'],
].map(([method, path]) => ({ method, path, safety: 'read' }));

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
  uiActions: tools.map(({ name, description, effect, input }) => ({ name, description, effect, inputSchema: input })),
});

export const networkCapability = () => ({
  origin: 'https://api.github.com',
  methods: ['GET'],
  credential: 'github',
  routes,
});
