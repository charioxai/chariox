// The window's write controls in headless Chrome: the built page, the App's real code, a loopback GitHub. Skipped
// where no Chrome is installed (set CHARIOX_TEST_CHROME to one).
import assert from 'node:assert/strict';
import test, { after, before } from 'node:test';
import { fixture, heads, REPO } from './support.mjs';
import { buildWindow, CHROME, launch, open } from './window-support.mjs';

const skip = CHROME ? false : 'no Chrome to run the window in';
let built, browser;
before(async () => { if (skip) return; built = buildWindow(); browser = await launch(); });
after(async () => { await browser?.close(); built?.remove(); });

const json = (status, body) => ({ status, headers: {}, body: JSON.stringify(body) });
const recorded = name => JSON.parse(fixture(name).body);
const rel = path => path.replace(`/repos/${REPO}`, '');
/** The changes the App made, as `METHOD /path body`. */
const made = session => session.changes().map(call => [call.method, rel(call.path), call.body]);

/** A window on a loopback GitHub that accepts every change; `more(url, init)` answers first. */
async function window(more = () => undefined, options = {}) {
  const session = await open(browser, built, {
    ...options,
    override: (url, init) => {
      const answer = more(url, init);
      if (answer) return answer;
      if (init.method === 'POST' && url.pathname.endsWith('/issues/2/comments')) return json(201, { id: 99, user: { login: 'mgutierrez09' }, body: JSON.parse(init.body).body, created_at: '2026-11-15T12:00:00Z' });
      if (init.method === 'POST' && url.pathname.endsWith('/reviews')) return json(200, { id: 7, user: { login: 'mgutierrez09' }, state: 'COMMENTED', body: '', submitted_at: '2026-11-15T12:00:00Z', commit_id: heads[2] });
      if (init.method === 'POST' && url.pathname.endsWith('/pulls')) return json(201, { ...recorded('pull_2'), number: 12, title: JSON.parse(init.body).title });
      if (init.method === 'PUT' && url.pathname.endsWith('/merge')) return json(200, { merged: true, sha: 'cafe0000cafe0000cafe0000cafe0000cafe0000', message: 'merged' });
      if (init.method !== 'GET') return json(200, {});
      return undefined;
    },
  });
  await session.page.waitForSelector('[data-pull]');
  return session;
}
const openPull = async (page, number) => { await page.click(`[data-pull$="#${number}"]`); await page.waitForSelector('text=Add a comment'); await page.waitForSelector('.Box >> text=Merge', { timeout: 15000 }).catch(() => {}); };
const settled = page => page.waitForTimeout(400);

test('the header\'s Pull requests tab goes back to the list', { skip }, async () => {
  const { page, errors } = await window();
  await page.click('[data-pull$="#2"]');
  await page.waitForSelector('text=Add a comment');
  await page.click('header a:has-text("Pull requests")');
  await page.waitForSelector('[data-pull]');
  assert.deepEqual(errors, []);
});

test('a comment is posted as typed; closing with a comment posts it first, then closes; a refusal is shown above the control', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await openPull(page, 2);
  assert.equal(await page.isDisabled('button:text-is("Comment")'), true, 'nothing to comment yet');
  await page.fill('textarea[aria-label="Leave a comment"]', 'Thanks, looks fine.');
  await page.click('button:text-is("Comment")');
  await settled(page);
  assert.deepEqual(made(session), [['POST', '/issues/2/comments', { body: 'Thanks, looks fine.' }]]);
  await page.fill('textarea[aria-label="Leave a comment"]', 'Not needed any more.');
  assert.equal(await page.textContent('button:has-text("Close with comment")'), 'Close with comment');
  await page.click('button:text-is("Close with comment")');
  await settled(page);
  assert.deepEqual(made(session).slice(1), [['POST', '/issues/2/comments', { body: 'Not needed any more.' }], ['PATCH', '/pulls/2', { state: 'closed' }]]);
});

test('merging asks which method, shows GitHub\'s confirmation step, and merges the head that is on screen with that method and message', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await openPull(page, 2);
  await page.click('summary[aria-label="Select merge method"]');
  assert.deepEqual(await page.locator('.SelectMenu-item .text-bold').allTextContents(), ['Create a merge commit', 'Squash and merge', 'Rebase and merge']);
  await page.click('.SelectMenu-item:has-text("Squash and merge")');
  await page.click('button:text-is("Squash and merge")');
  assert.equal(await page.inputValue('input[aria-label="Commit message"]'), 'Add hello, notes and a large data file (#2)');
  assert.deepEqual(made(session), [], 'one click does not merge: there is a confirmation step');
  await page.click('button:text-is("Cancel")');
  assert.deepEqual(made(session), []);
  await page.click('button:text-is("Squash and merge")');
  await page.fill('textarea[aria-label="Extended description"]', 'Because.');
  await page.click('button:text-is("Confirm squash and merge")');
  await settled(page);
  assert.deepEqual(made(session), [['PUT', '/pulls/2/merge', { merge_method: 'squash', sha: heads[2], commit_title: 'Add hello, notes and a large data file (#2)', commit_message: 'Because.' }]]);
});

test('a head that moved after the page was read is refused with the reason and nothing is merged', { skip }, async () => {
  let moved = false;
  const session = await window((url, init) => {
    if (moved && init.method === 'GET' && url.pathname.endsWith('/pulls/2')) return json(200, { ...recorded('pull_2'), head: { ...recorded('pull_2').head, sha: heads[1] } });
    return undefined;
  });
  const { page } = session;
  await openPull(page, 2);
  moved = true;
  await page.click('button:text-is("Merge pull request")');
  await page.click('button:text-is("Confirm merge")');
  await page.waitForSelector('.flash-error');
  assert.match(await page.textContent('.flash-error'), /The branch moved after it was read: it is now at a4b9d7c/);
  assert.deepEqual(made(session), []);
});

test('the page is not reloaded under the merge confirmation or an open menu when something changes on GitHub meanwhile', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await openPull(page, 2);
  await page.click('button:text-is("Merge pull request")');
  await page.waitForSelector('button:text-is("Confirm merge")');
  // another actor comments: the App's change feed tells the open page, which would reload what it shows
  await page.evaluate(() => window.chariox.call('comment', { repo: 'charioxai/chariox-github-app-drill', number: 2, body: 'From elsewhere.' }));
  await page.waitForTimeout(7500);
  assert.equal(await page.locator('button:text-is("Confirm merge")').count(), 1, 'the confirmation step is still there');
  await page.click('button:text-is("Cancel")');
  await page.click('summary[aria-label="Select merge method"]');
  await page.waitForTimeout(7500);
  assert.equal(await page.locator('.SelectMenu-item:has-text("Rebase and merge")').isVisible(), true, 'the open menu is still open');
});

test('GitHub\'s own refusal of a merge is shown as GitHub words it', { skip }, async () => {
  const session = await window((url, init) => init.method === 'PUT' ? json(405, { message: 'Required status check "ci" is expected.' }) : undefined);
  const { page } = session;
  await openPull(page, 2);
  await page.click('button:text-is("Merge pull request")');
  await page.click('button:text-is("Confirm merge")');
  await page.waitForSelector('.flash-error');
  assert.match(await page.textContent('.flash-error'), /Required status check "ci" is expected/);
  assert.equal(await page.isEnabled('button:text-is("Confirm merge")'), true, 'the person can cancel or try again');
});

test('a draft offers Ready for review; a merged pull request offers Delete branch for its own branch', { skip }, async () => {
  const draft = await window((url, init) => url.pathname === '/graphql' ? json(200, { data: { ok: true } }) : undefined);
  await openPull(draft.page, 3);
  assert.equal(await draft.page.locator('button:text-is("Merge pull request")').count(), 0, 'a draft is not merged');
  await draft.page.click('button:text-is("Ready for review")');
  await settled(draft.page);
  assert.match(draft.double.calls.find(call => call.path === '/graphql').body.query, /markPullRequestReadyForReview/);

  const search = JSON.parse(fixture('search_merged').body);
  const merged = await window((url, init) => url.pathname === '/search/issues' ? json(200, search) : url.pathname.endsWith('/branches/drill%2Fgha4-merged') ? json(200, { ...recorded('branch_rich'), name: 'drill/gha4-merged', protected: false }) : undefined);
  await openPull(merged.page, 5);
  await merged.page.click('button:text-is("Delete branch")');
  await settled(merged.page);
  assert.deepEqual(made(merged), [['DELETE', '/git/refs/heads/drill%2Fgha4-merged', undefined]]);
});

test('reviewers and labels are chosen from the gear of their section', { skip }, async () => {
  const session = await window((url, init) => {
    if (init.method === 'GET' && url.pathname.endsWith('/assignees')) return json(200, [{ login: 'octo', type: 'User' }, { login: 'hubot', type: 'User' }]);
    if (init.method === 'POST' && url.pathname.endsWith('/requested_reviewers')) return json(201, { requested_reviewers: [{ login: 'octo' }], requested_teams: [] });
    if (init.method === 'POST' && url.pathname.endsWith('/labels')) return json(200, [{ name: 'bug', color: 'd73a4a' }, { name: 'documentation', color: '0075ca' }, { name: 'help wanted', color: '008672' }]);
    return undefined;
  });
  const { page } = session;
  await openPull(page, 2);
  await page.click('summary[aria-label="Request up to 15 reviewers"]');
  await page.waitForSelector('.SelectMenu-item:has-text("octo")');
  await page.click('.SelectMenu-item:has-text("octo")');
  await settled(page);
  await page.click('summary[aria-label="Apply labels to this pull request"]');
  await page.waitForSelector('.SelectMenu-item:has-text("help wanted")');
  await page.click('.SelectMenu-item:has-text("help wanted")');
  await settled(page);
  assert.deepEqual(made(session), [['POST', '/pulls/2/requested_reviewers', { reviewers: ['octo'], team_reviewers: [] }], ['POST', '/issues/2/labels', { labels: ['help wanted'] }]]);
});

test('a comment on a line starts a review; the review is finished with a verdict and sends all its comments at once; the author cannot approve', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await openPull(page, 2);
  await page.click('a:has-text("Files changed")');
  await page.waitForSelector('.diff-file');
  await page.click('[data-file="src/hello.js"] button[aria-label="Add a comment on line 2"]', { force: true });
  await page.fill('.diff-composer textarea', 'Rename this please.');
  await page.click('button:text-is("Start a review")');
  await page.waitForSelector('[data-pending]');
  assert.equal((await page.textContent('summary:has-text("Review changes") .Counter')).trim(), '1');
  await page.click('[data-file="src/hello.js"] button[aria-label="Add a comment on line 4"]', { force: true });
  await page.fill('.diff-composer textarea', 'And this.');
  assert.equal(await page.locator('.diff-composer button:text-is("Add review comment")').count(), 1, 'a second comment joins the review');
  await page.click('.diff-composer button:text-is("Add review comment")');
  await page.click('summary:has-text("Review changes")');
  assert.equal(await page.isDisabled('input[value="approve"]'), true);
  assert.equal(await page.isDisabled('input[value="request_changes"]'), true);
  assert.match(await page.textContent('label:has(input[value="approve"])'), /can't approve their own pull request/);
  await page.fill('.Popover textarea', 'Two small things.');
  await page.click('button:text-is("Submit review")');
  await settled(page);
  assert.deepEqual(made(session), [['POST', '/pulls/2/reviews', {
    event: 'COMMENT', commit_id: heads[2], body: 'Two small things.',
    comments: [{ path: 'src/hello.js', line: 2, side: 'RIGHT', body: 'Rename this please.' }, { path: 'src/hello.js', line: 4, side: 'RIGHT', body: 'And this.' }],
  }]]);
});

test('a single comment on a line is sent at once; someone else\'s pull request can be approved and asked for changes', { skip }, async () => {
  const session = await window((url, init) => {
    if (init.method === 'GET' && url.pathname === '/user') return json(200, { login: 'reviewer' });
    return undefined;
  });
  const { page } = session;
  await openPull(page, 2);
  await page.click('a:has-text("Files changed")');
  await page.waitForSelector('.diff-file');
  await page.click('[data-file="docs/notes.md"] button[aria-label="Add a comment on line 1"]', { force: true });
  await page.fill('.diff-composer textarea', 'Title?');
  await page.click('button:text-is("Add single comment")');
  await settled(page);
  assert.deepEqual(made(session), [['POST', '/pulls/2/reviews', { event: 'COMMENT', commit_id: heads[2], comments: [{ path: 'docs/notes.md', line: 1, side: 'RIGHT', body: 'Title?' }] }]]);
  await page.waitForSelector('summary:has-text("Review changes")');
  await page.click('summary:has-text("Review changes")');
  assert.equal(await page.isEnabled('input[value="approve"]'), true, 'another account may approve');
  await page.click('input[value="request_changes"]');
  assert.equal(await page.isDisabled('button:text-is("Submit review")'), true, 'asking for changes needs a message');
  await page.fill('.Popover textarea', 'Please add a test.');
  await page.click('button:text-is("Submit review")');
  await settled(page);
  assert.deepEqual(made(session).at(-1), ['POST', '/pulls/2/reviews', { event: 'REQUEST_CHANGES', body: 'Please add a test.', commit_id: heads[2] }]);
});

test('failed checks can be run again from the Checks tab', { skip }, async () => {
  const session = await window((url, init) => init.method === 'POST' && url.pathname.includes('/rerun-failed-jobs') ? json(201, {}) : undefined);
  const { page } = session;
  await openPull(page, 2);
  await page.click('a:has-text("Checks")');
  await page.click('button:has-text("Re-run failed jobs")');
  await settled(page);
  assert.deepEqual(made(session).map(([method, path]) => `${method} ${path}`), ['POST /actions/runs/43680658254/rerun-failed-jobs', 'POST /actions/runs/43680655184/rerun-failed-jobs']);
});

test('a failed check shows its annotations and the steps of its job, the failing one marked', { skip }, async () => {
  const { page } = await window();
  await openPull(page, 2);
  await page.click('a:has-text("Checks")');
  await page.click('[data-check="failing"] button[aria-label^="Details of"] >> nth=0');
  await page.waitForSelector('[data-step]');
  assert.deepEqual(await page.locator('[data-step]').evaluateAll(rows => rows.map(row => row.dataset.step)), ['Set up job', 'Run echo "e2e: expected a green banner"', 'Complete job']);
  assert.match(await page.textContent('[data-check="failing"]'), /Process completed with exit code 1/);
});

test('a pull request is opened from two branches: the comparison is shown, the title comes from the branch, and the page goes to the new pull request', { skip }, async () => {
  // GitHub numbers the new pull request 12; the loopback answers it with the recording of #2 under that number
  const session = await window((url, init) => {
    if (init.method !== 'GET' || !/\/(pulls|issues)\/12(\/|$)/.test(url.pathname)) return undefined;
    return url.pathname.endsWith('/12') ? fixture('pull_2') : json(200, []);
  });
  const { page } = session;
  await page.click('a:has-text("New pull request")');
  await page.waitForSelector('text=Open a pull request');
  await page.click('summary:has-text("compare:")');
  await page.click('.SelectMenu-item:has-text("drill/gha4-rich")');
  await page.waitForSelector('text=Able to create a pull request');
  assert.equal(await page.inputValue('input[aria-label="Add a title"]'), 'Drill: gha4 rich');
  await page.fill('input[aria-label="Add a title"]', 'Add hello and notes');
  await page.fill('textarea[aria-label="Add a description"]', 'Why.');
  await page.click('button:text-is("Create draft pull request")');
  await page.waitForSelector('text=Add a comment');
  assert.deepEqual(made(session), [['POST', '/pulls', { title: 'Add hello and notes', head: 'drill/gha4-rich', base: 'main', body: 'Why.', draft: true }]]);
});

test('what is being written survives the page reloading behind it', { skip }, async () => {
  const session = await window();
  const { page } = session;
  await openPull(page, 2);
  await page.fill('textarea[aria-label="Leave a comment"]', 'Half written');
  await page.click('a:has-text("Checks")');
  await page.click('a:has-text("Conversation")');
  await page.waitForSelector('textarea[aria-label="Leave a comment"]');
  assert.equal(await page.inputValue('textarea[aria-label="Leave a comment"]'), 'Half written');
});
