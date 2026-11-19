// What a kind of GitHub event is called in the window.
export const EVENT_TEXT = {
  'pull_request.opened': 'Pull request opened', 'pull_request.updated': 'Pull request updated', 'pull_request.review_requested': 'Review requested',
  'pull_request.review_submitted': 'Review submitted', 'pull_request.merged': 'Pull request merged', 'pull_request.closed': 'Pull request closed',
  'pull_request.ready_for_review': 'Ready for review', 'pull_request.reopened': 'Pull request reopened', 'pull_request.labeled': 'Pull request labeled',
  'pull_request.comment': 'Comment on a pull request', 'check.completed': 'Check completed', 'ci.finished': 'Checks finished', 'workflow_run.completed': 'Workflow run completed',
  'issue.opened': 'Issue opened', 'issue.closed': 'Issue closed', 'issue.labeled': 'Issue labeled', 'issue.comment': 'Comment on an issue',
  'release.published': 'Release published', push: 'Push', mention: 'Mention of you', expired: 'Expired',
};
export const eventText = kind => EVENT_TEXT[kind] ?? kind;
