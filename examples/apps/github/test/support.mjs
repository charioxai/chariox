// A kernel's state and a loopback api.github.com for the App's tests. The answers are recordings of the sandbox
// repository (charioxai/chariox-github-app-drill) with their URLs trimmed; the fake checks what the real kernel
// enforces: no Authorization header from the App, an account connection id, conditional requests.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

export const fixture = name => JSON.parse(readFileSync(new URL(`./fixtures/${name}.json`, import.meta.url), 'utf8'));
export const heads = fixture('heads');
export const FAILING_CHECK = 132710337474;
export const REPO = 'charioxai/chariox-github-app-drill';

export function memoryState() {
  const saved = new Map();
  return {
    saved,
    get: async key => saved.get(key) ?? null,
    transact: async ({ schemaVersion, checks, writes }) => {
      assert.equal(schemaVersion, 0);
      for (const check of checks) {
        if ((saved.get(check.key)?.version ?? null) !== check.version) throw { code: 'CONFLICT', message: `${check.key} changed`, retryable: true };
      }
      for (const write of writes) {
        if (write.tag === 'delete') saved.delete(write.val);
        else saved.set(write.val[0], { value: write.val[1], version: (saved.get(write.val[0])?.version ?? 0n) + 1n });
      }
      return { revision: 1n, receipts: [] };
    },
  };
}

const pageOf = url => Number(url.searchParams.get('page') ?? 1);
const FIXTURE_OF = [
  [/^\/user$/, () => 'user'],
  [/^\/rate_limit$/, () => 'rate_limit'],
  [/^\/(user|orgs\/[^/]+)\/repos$/, () => 'user_repos'],
  [/^\/search\/issues$/, url => {
    const q = url.searchParams.get('q');
    return q.includes('is:merged') ? 'search_merged' : q.includes('review-requested') ? 'search_review_requested' : 'search_open';
  }],
  [/^\/repos\/[^/]+\/[^/]+\/pulls\/(\d+)$/, (url, match, headers) => headers.accept?.includes('diff') ? 'pull_3_diff' : `pull_${match[1]}`],
  [/\/pulls\/[134-9]\/(files|commits|reviews|comments)$|\/issues\/[134-9]\/comments$/, () => 'empty_list'],
  [/\/pulls\/2\/files$/, () => 'pull_2_files'],
  [/\/pulls\/2\/commits$/, () => 'pull_2_commits'],
  [/\/pulls\/2\/reviews$/, () => 'pull_2_reviews'],
  [/\/pulls\/2\/comments$/, () => 'pull_2_line_comments'],
  [/\/issues\/2\/comments$/, () => 'pull_2_comments'],
  [/\/check-runs\/\d+\/annotations$/, () => 'check_run_annotations'],
  [/\/check-runs\/\d+$/, () => 'check_run_failing'],
  [/\/commits\/[^/]+\/check-runs$/, () => 'pull_2_check_runs'],
  [/\/commits\/[^/]+\/status$/, () => 'pull_2_status'],
  [/\/branches$/, () => 'branches'],
  [/\/branches\/[^/]+\/protection$/, () => 'protection'],
  [/\/compare\/[^/]+$/, () => 'compare_2'],
];

/** `override(url, init)` may answer first with `{status, headers, body}`; otherwise the recording answers. */
export function fakeGitHub({ override, unbound = [] } = {}) {
  const calls = [];
  const fetch = async (urlText, init = {}) => {
    const url = new URL(urlText);
    const headers = Object.fromEntries(Object.entries(init.headers ?? {}).map(([k, v]) => [k.toLowerCase(), v]));
    calls.push({ url: urlText, path: url.pathname, query: url.searchParams, headers, connectionId: init.connectionId });
    assert.equal(url.origin, 'https://api.github.com');
    assert.equal(headers.authorization, undefined, 'the App never sets Authorization');
    assert.match(init.connectionId ?? '', /^account:[a-z0-9-]+$/, 'every call is made as an account');
    if (unbound.includes(init.connectionId.slice('account:'.length))) throw { payload: { code: 'APP_HTTP_ACCOUNT_UNKNOWN', message: 'no binding' } };
    let answer = override?.(url, init);
    if (!answer) {
      const hit = FIXTURE_OF.map(([pattern, pick]) => [pattern.exec(url.pathname), pick]).find(([match]) => match);
      assert.ok(hit, `the loopback double has no answer for ${url.pathname}`);
      const recorded = fixture(hit[1](url, hit[0], headers));
      answer = { status: recorded.status, headers: recorded.headers, body: recorded.body };
    }
    const etag = answer.headers?.etag;
    const matched = etag && headers['if-none-match'] === etag;
    const out = matched ? { status: 304, headers: { etag }, body: '' } : answer;
    return {
      status: out.status,
      headers: { get: name => out.headers?.[name.toLowerCase()] ?? null },
      text: async () => out.body ?? '',
    };
  };
  return { fetch, calls };
}
