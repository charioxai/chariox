// The read tools: pull requests and what surrounds them. Each call names an account (or the default) and
// every answer is cut down by normalize.mjs to what a window row or an agent needs.
import * as n from './normalize.mjs';

const fail = (code, message) => { throw { code, message, retryable: false }; };
const splitRepo = repo => { const [owner, name] = repo.split('/'); return { owner, repo: name }; };
const DIFF_DEFAULT = 256 * 1024;

/** The GitHub search that finds what a scope of the pull request list means. */
export function searchQuery({ scope, repos = [], org, state = 'open', draft = 'any', labels = [], author, review, query }) {
  const parts = ['is:pr'];
  if (scope === 'mine') parts.push('author:@me');
  else if (scope === 'review_requested') parts.push('review-requested:@me');
  else if (scope === 'all') parts.push('involves:@me');
  else if (scope === 'org') {
    if (!org) fail('INVALID_ARGUMENT', 'The org scope needs an organization.');
    parts.push(`org:${org}`);
  } else if (scope === 'repo') {
    if (!repos.length) fail('INVALID_ARGUMENT', 'The repo scope needs at least one repository.');
  } else fail('INVALID_ARGUMENT', `Unknown scope ${scope}`);
  for (const repo of repos) parts.push(`repo:${repo}`);
  if (state !== 'all') parts.push(`is:${state}`);
  if (draft === 'only') parts.push('draft:true'); else if (draft === 'hide') parts.push('draft:false');
  for (const name of labels) parts.push(`label:"${name.replace(/"/g, '')}"`);
  if (author) parts.push(`author:${author}`);
  if (review) parts.push(`review:${review}`);
  if (query) parts.push(query.trim());
  parts.push('sort:updated-desc');
  return parts.join(' ');
}

export function createPulls({ accounts, github }) {
  const call = async (input, run) => {
    const account = await accounts.resolve(input);
    return run(account);
  };
  const where = input => splitRepo(input.repo);
  const pullPath = '/repos/{owner}/{repo}/pulls/{number}';
  const params = input => ({ ...where(input), number: input.number });
  const fetchPull = (account, input, options) => github.get(account, pullPath, params(input), options);

  async function checksOf(account, input, sha) {
    const base = { ...where(input), ref: sha };
    const [runs, statuses] = await Promise.all([
      github.get(account, '/repos/{owner}/{repo}/commits/{ref}/check-runs', base, { query: { per_page: 100 } }),
      github.get(account, '/repos/{owner}/{repo}/commits/{ref}/status', base, { query: { per_page: 100 } }),
    ]);
    return { runs: runs.data.check_runs ?? [], statuses: statuses.data.statuses ?? [] };
  }

  async function reviewsOf(account, input) {
    const { data } = await github.get(account, `${pullPath}/reviews`, params(input), { query: { per_page: 100 } });
    return data.map(n.review);
  }

  return {
    async list_repositories(input) {
      return call(input, async account => {
        const { items, page, hasNext } = input.org
          ? await github.list(account, '/orgs/{org}/repos', { org: input.org }, { query: { sort: 'pushed', type: 'all' }, page: input.page, perPage: input.perPage ?? 100 })
          : await github.list(account, '/user/repos', {}, { query: { sort: 'pushed', affiliation: 'owner,collaborator,organization_member' }, page: input.page, perPage: input.perPage ?? 100 });
        const query = input.query?.toLowerCase();
        const repos = items.filter(r => !query || r.full_name.toLowerCase().includes(query)).map(r => ({
          fullName: r.full_name, private: r.private, description: r.description ?? null, defaultBranch: r.default_branch,
          pushedAt: r.pushed_at, archived: r.archived, language: r.language ?? null,
        }));
        return { repos, page: page ?? 1, hasNext };
      });
    },

    async list_pull_requests(input) {
      const account = await accounts.resolve({ account: input.account, repo: input.repos?.length === 1 ? input.repos[0] : undefined });
      const q = searchQuery(input);
      const { data, link, stale } = await github.get(account, '/search/issues', {}, { query: { q, per_page: input.perPage ?? 25, page: input.page ?? 1 } });
      return {
        account, query: q, total: data.total_count, incomplete: data.incomplete_results,
        pulls: data.items.filter(item => item.pull_request).map(n.searchRow),
        page: input.page ?? 1, hasNext: Boolean(link.next), stale,
      };
    },

    async get_pull_request(input) {
      return call(input, async account => ({ account, pull: n.pullDetail((await fetchPull(account, input)).data) }));
    },

    /** A list row's extras: size from the pull request, check state from its head commit, the review decision. */
    async get_pull_request_summary(input) {
      return call(input, async account => {
        const { data: pull } = await fetchPull(account, input);
        const [checks, reviews] = await Promise.all([
          checksOf(account, input, pull.head.sha).then(({ runs, statuses }) => n.checksSummary(runs, statuses)),
          reviewsOf(account, input),
        ]);
        return {
          repo: input.repo, number: input.number, additions: pull.additions, deletions: pull.deletions, changedFiles: pull.changed_files,
          checks, review: n.reviewDecision(reviews, (pull.requested_reviewers ?? []).length + (pull.requested_teams ?? []).length),
        };
      });
    },

    async get_pull_request_files(input) {
      return call(input, async account => {
        const { items, page, hasNext } = await github.list(account, `${pullPath}/files`, params(input), { page: input.page, perPage: input.perPage ?? 100 });
        return { files: items.map(n.file), page, hasNext };
      });
    },

    async get_pull_request_diff(input) {
      return call(input, async account => {
        const limit = input.maxBytes ?? DIFF_DEFAULT;
        const size = text => new TextEncoder().encode(text).length;
        const answer = text => {
          const bytes = size(text);
          if (bytes <= limit) return { diff: text, tooLarge: false, truncated: false, bytes };
          const cut = text.slice(0, limit);
          return { diff: cut.slice(0, cut.lastIndexOf('\n') + 1), tooLarge: false, truncated: true, bytes };
        };
        if (input.path) {
          // One file's patch comes with the file list, which also works when another file makes the whole diff too large.
          for (let page = 1; page <= 30; page++) {
            const { items, hasNext } = await github.list(account, `${pullPath}/files`, params(input), { page, perPage: 100 });
            const file = items.find(item => item.filename === input.path);
            if (file) {
              if (file.patch) return answer(`diff --git a/${file.previous_filename ?? file.filename} b/${file.filename}\n${file.patch}\n`);
              break;
            }
            if (!hasNext) break;
          }
        }
        let text;
        try {
          ({ data: text } = await fetchPull(account, input, { accept: 'application/vnd.github.diff' }));
        } catch (error) {
          const code = (error?.payload ?? error)?.code;
          // GitHub refuses a diff of more than 20000 lines, and the kernel answers at most 1 MiB.
          if (code === 'UNPROCESSABLE' || code === 'APP_HTTP_LIMIT') return { diff: '', tooLarge: true, truncated: false, bytes: 0 };
          throw error;
        }
        if (input.path) {
          const sections = text.split(/^(?=diff --git )/m);
          text = sections.find(section => section.startsWith('diff --git ') && section.split('\n', 1)[0].endsWith(` b/${input.path}`)) ?? '';
        }
        return answer(text);
      });
    },

    async list_commits(input) {
      return call(input, async account => {
        const { items, page, hasNext } = await github.list(account, `${pullPath}/commits`, params(input), { page: input.page, perPage: input.perPage ?? 100 });
        return { commits: items.map(n.commit), page, hasNext };
      });
    },

    async get_checks(input) {
      return call(input, async account => {
        const { data: pull } = await fetchPull(account, input);
        const { runs, statuses } = await checksOf(account, input, pull.head.sha);
        return { sha: pull.head.sha, ...n.checksSummary(runs, statuses), checks: runs.map(n.checkRun), statuses: statuses.map(n.commitStatus) };
      });
    },

    async list_reviews(input) {
      return call(input, async account => {
        const [{ data: pull }, reviews] = await Promise.all([fetchPull(account, input), reviewsOf(account, input)]);
        const requested = (pull.requested_reviewers ?? []).map(u => ({ login: u.login, avatarUrl: u.avatar_url }));
        const teams = (pull.requested_teams ?? []).map(t => ({ slug: t.slug, name: t.name }));
        return { reviews, requestedReviewers: requested, requestedTeams: teams, decision: n.reviewDecision(reviews, requested.length + teams.length) };
      });
    },

    async list_comments(input) {
      return call(input, async account => {
        const paging = { page: input.page, perPage: input.perPage ?? 50 };
        const [conversation, lines] = await Promise.all([
          github.list(account, '/repos/{owner}/{repo}/issues/{number}/comments', params(input), paging),
          github.list(account, `${pullPath}/comments`, params(input), paging),
        ]);
        return {
          comments: conversation.items.map(n.issueComment), lineComments: lines.items.map(n.reviewComment),
          page: paging.page ?? 1, hasNext: conversation.hasNext || lines.hasNext,
        };
      });
    },

    async get_mergeability(input) {
      return call(input, async account => {
        const { data: pull } = await fetchPull(account, input);
        const base = { ...where(input) };
        const [behind, protection] = await Promise.all([
          github.get(account, '/repos/{owner}/{repo}/compare/{basehead}', { ...base, basehead: `${pull.base.ref}...${pull.head.sha}` })
            .then(({ data }) => ({ behindBy: data.behind_by, aheadBy: data.ahead_by }), () => ({ behindBy: null, aheadBy: null })),
          github.get(account, '/repos/{owner}/{repo}/branches/{branch}/protection', { ...base, branch: pull.base.ref })
            .then(({ data }) => ({
              state: 'protected',
              requiredApprovals: data.required_pull_request_reviews?.required_approving_review_count ?? 0,
              requiredChecks: data.required_status_checks?.contexts ?? [],
              requireUpToDate: Boolean(data.required_status_checks?.strict),
            }), error => {
              const cause = error?.payload ?? error;
              return { state: /not protected/i.test(cause?.github ?? '') ? 'unprotected' : 'unknown' };
            }),
        ]);
        const detail = n.pullDetail(pull);
        return {
          mergeable: detail.mergeable, mergeableState: detail.mergeableState, rebaseable: detail.rebaseable, draft: detail.state === 'draft',
          merged: detail.merged, autoMerge: detail.autoMerge, ...behind, protection,
        };
      });
    },

    async list_branches(input) {
      return call(input, async account => {
        const { items, page, hasNext } = await github.list(account, '/repos/{owner}/{repo}/branches', where(input), { page: input.page, perPage: input.perPage ?? 100 });
        return { branches: items.map(b => ({ name: b.name, protected: b.protected, sha: b.commit.sha })), page, hasNext };
      });
    },

    async get_compare(input) {
      return call(input, async account => {
        const { data } = await github.get(account, '/repos/{owner}/{repo}/compare/{basehead}', { ...where(input), basehead: `${input.base}...${input.head}` });
        return {
          status: data.status, aheadBy: data.ahead_by, behindBy: data.behind_by, totalCommits: data.total_commits,
          filesChanged: (data.files ?? []).length, url: data.html_url,
          commits: data.commits.slice(0, 20).map(c => ({ sha: c.sha, message: c.commit.message.split('\n', 1)[0], author: c.author?.login ?? c.commit.author?.name ?? null })),
        };
      });
    },
  };
}
