// The event types the generator delivers (docs/fixtures/event-generators/github) and what a subscription calls them.
// `incoming` is the App's declared event name for each generator type; `kinds` are the names a person picks.
export const incoming = {
  pull_request_opened: 'pull_request.opened',
  pull_request_updated: 'pull_request.synchronize',
  review_requested: 'pull_request.review_requested',
  pull_request_closed: 'pull_request.closed',
  pull_request_ready_for_review: 'pull_request.ready_for_review',
  pull_request_reopened: 'pull_request.reopened',
  pull_request_labeled: 'pull_request.labeled',
  pull_request_review_submitted: 'pull_request_review.submitted',
  pull_request_review_comment_created: 'pull_request_review_comment.created',
  issue_comment_created: 'issue_comment.created',
  issue_opened: 'issues.opened',
  issues_closed: 'issues.closed',
  issues_labeled: 'issues.labeled',
  check_run_completed: 'check_run.completed',
  check_suite_completed: 'check_suite.completed',
  workflow_run_completed: 'workflow_run.completed',
  release_published: 'release.published',
  push: 'push',
};
export const OUTGOING = 'github_notification';

/** What a subscription may ask for, and the generator types that must be routed for it. */
const COMMENTS = ['issue_comment.created', 'pull_request_review_comment.created'];
const MENTIONS = [...COMMENTS, 'pull_request_review.submitted', 'issues.opened', 'pull_request.opened'];
export const kinds = {
  'pull_request.opened': ['pull_request.opened'],
  'pull_request.updated': ['pull_request.synchronize'],
  'pull_request.review_requested': ['pull_request.review_requested'],
  'pull_request.review_submitted': ['pull_request_review.submitted'],
  'pull_request.merged': ['pull_request.closed'],
  'pull_request.closed': ['pull_request.closed'],
  'pull_request.ready_for_review': ['pull_request.ready_for_review'],
  'pull_request.reopened': ['pull_request.reopened'],
  'pull_request.labeled': ['pull_request.labeled'],
  'pull_request.comment': COMMENTS,
  'check.completed': ['check_run.completed', 'check_suite.completed'],
  'ci.finished': ['check_run.completed', 'check_suite.completed', 'workflow_run.completed'],
  'workflow_run.completed': ['workflow_run.completed'],
  'issue.opened': ['issues.opened'],
  'issue.closed': ['issues.closed'],
  'issue.labeled': ['issues.labeled'],
  'issue.comment': ['issue_comment.created'],
  'release.published': ['release.published'],
  push: ['push'],
  mention: MENTIONS,
};

const CHECKS = new Set(['check_run.completed', 'check_suite.completed', 'workflow_run.completed']);
export const finishesChecks = sourceType => CHECKS.has(sourceType);

/** The picked names one delivered event satisfies (before filters). */
export function kindsOf(sourceType, metadata) {
  const out = new Set(Object.entries(kinds).filter(([name, types]) => name !== 'mention' && name !== 'ci.finished' && types.includes(sourceType)).map(([name]) => name));
  // A merge is a merge: `pull_request.closed` is a pull request closed without merging, so one event is one delivery.
  if (sourceType === 'pull_request.closed') out.delete(metadata.chariox?.merged === true ? 'pull_request.closed' : 'pull_request.merged');
  // An issue comment on a pull request is a pull request comment, and only that.
  if (sourceType === 'issue_comment.created') out.delete(metadata.issue?.pull_request ? 'issue.comment' : 'pull_request.comment');
  if (metadata.chariox?.body_mentions?.length) out.add('mention');
  return out;
}
