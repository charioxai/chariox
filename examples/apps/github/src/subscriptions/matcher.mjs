// The pure part of a subscription: does a delivered event satisfy it. Nothing here reads state or calls GitHub,
// so the polling sources of a later lane feed the same function with the same normalized event.
import { kindsOf } from '../event-types.mjs';

const escapeRegExp = text => text.replace(/[.+^${}()|[\]\\]/g, '\\$&');
const globCache = new Map();
/** `*` stays inside a path segment, `**` crosses segments, `?` is one character. */
export function globToRegExp(glob) {
  let compiled = globCache.get(glob);
  if (!compiled) {
    const source = [...glob].reduce((text, char, index, chars) => {
      if (char === '*') return chars[index - 1] === '*' ? text : text + (chars[index + 1] === '*' ? '.*' : '[^/]*');
      return text + (char === '?' ? '[^/]' : escapeRegExp(char));
    }, '');
    compiled = new RegExp(`^${source}$`);
    globCache.set(glob, compiled);
  }
  return compiled;
}

/** A list with `!` entries: it holds when some plain entry matches (or there are none) and no `!` entry matches. */
export function listMatches(patterns, values, test = (pattern, value) => pattern === value) {
  const include = patterns.filter(pattern => !pattern.startsWith('!'));
  const exclude = patterns.filter(pattern => pattern.startsWith('!')).map(pattern => pattern.slice(1));
  const hit = patterns => values.some(value => patterns.some(pattern => test(pattern, value)));
  return (include.length === 0 || hit(include)) && !hit(exclude);
}
const same = (pattern, value) => pattern.toLowerCase() === value.toLowerCase();
const globs = (pattern, value) => globToRegExp(pattern).test(value);
const foldedGlobs = (pattern, value) => globs(pattern.toLowerCase(), value.toLowerCase());

export function inScope(scope, repo) {
  const lower = repo.toLowerCase();
  if (scope.kind === 'all') return true;
  if (scope.kind === 'org') return lower.startsWith(`${scope.org.toLowerCase()}/`);
  return scope.repos.some(known => known.toLowerCase() === lower);
}

/** The filters that can be decided from the delivery alone; `needsPaths` says the changed files are still needed. */
export function filterReason(subscription, event, { logins = [] } = {}) {
  const { filters = {} } = subscription;
  if (filters.branches?.length && !(event.branch !== null && listMatches(filters.branches, [event.branch], globs))) return `branch ${event.branch ?? '(none)'} is not selected`;
  if (filters.authors?.length && !(event.author !== null && listMatches(filters.authors, [event.author], same))) return `author ${event.author ?? '(none)'} is not selected`;
  if (filters.labels?.length && !listMatches(filters.labels, event.labels, foldedGlobs)) return 'no selected label';
  if (filters.draft === 'only' && event.draft !== true) return 'not a draft';
  if (filters.draft === 'hide' && event.draft === true) return 'a draft';
  if (filters.conclusion?.length && event.conclusion !== null && !filters.conclusion.includes(event.conclusion)) return `conclusion ${event.conclusion ?? '(none)'} is not selected`;
  if (filters.excludeSelf && event.sender !== null && logins.some(login => same(login, event.sender))) return 'caused by the account itself';
  return null;
}

export const needsPaths = subscription => Boolean(subscription.filters?.paths?.length);
export const pathsMatch = (subscription, files) => listMatches(subscription.filters.paths, files, globs);

/** Which of the subscription's picked names this event satisfies; empty when the repository or event does not fit. */
export function matchedKinds(subscription, event, { logins = [] } = {}) {
  if (!inScope(subscription.scope, event.repo)) return [];
  const satisfied = kindsOf(event.sourceType, event.metadata);
  // A mention counts when the body names one of the account's own logins.
  if (!event.mentions.some(mention => logins.some(login => same(login, mention)))) satisfied.delete('mention');
  return subscription.events.filter(kind => satisfied.has(kind));
}

/** The facts of a delivery the matcher and the targets use, read once from the generator's payload. */
export function normalize(sourceType, payload) {
  const metadata = payload.metadata ?? {};
  const derived = metadata.chariox ?? {};
  const pull = metadata.pull_request;
  const issue = metadata.issue;
  const run = metadata.check_run ?? metadata.check_suite ?? metadata.workflow_run;
  const number = pull?.number ?? issue?.number ?? derived.pr_numbers?.[0] ?? null;
  return {
    sourceType, metadata, text: payload.text, occurredAt: payload.occurred_at,
    repo: metadata.repository?.full_name ?? '',
    number,
    prNumbers: derived.pr_numbers ?? (pull ? [pull.number] : []),
    title: pull?.title ?? issue?.title ?? metadata.release?.name ?? run?.name ?? null,
    url: pull?.html_url ?? issue?.html_url ?? metadata.release?.html_url ?? run?.html_url ?? null,
    author: pull?.user?.login ?? issue?.user?.login ?? null,
    sender: derived.sender_login ?? metadata.sender?.login ?? null,
    labels: derived.labels ?? [],
    branch: derived.branch ?? null,
    baseRef: derived.base_ref ?? null,
    headSha: derived.head_sha ?? null,
    draft: pull ? pull.draft === true : null,
    merged: derived.merged === true,
    conclusion: run?.conclusion ?? null,
    mentions: derived.body_mentions ?? [],
  };
}
