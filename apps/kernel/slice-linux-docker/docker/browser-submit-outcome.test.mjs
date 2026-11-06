import assert from 'node:assert/strict';
import test from 'node:test';
import { awaitSubmitOutcome } from './browser-submit-outcome.mjs';

const clock = () => {
  let time = 0;
  return { now: () => time, sleep: async (ms) => { time += ms; } };
};

test('a document replaced after the submit reports where the page landed once it is loaded', async () => {
  const { now, sleep } = clock();
  let polls = 0, reads = 0;
  const outcome = await awaitSubmitOutcome({
    now, sleep, pageHandled: false,
    replaced: async () => ++polls > 3,
    page: async () => ({ url: 'https://site.test/home', title: 'Home', ready: ++reads > 2 }),
  });
  assert.deepEqual(outcome, { navigated: true, url: 'https://site.test/home', title: 'Home' });
  assert.ok(reads > 2, 'waits for the new document to load');
});

test('a page that handles the submit itself is reported after a short wait, not as a navigation', async () => {
  const { now, sleep } = clock();
  let waited = 0;
  const outcome = await awaitSubmitOutcome({
    now, sleep: async (ms) => { waited += ms; await sleep(ms); }, pageHandled: true,
    replaced: async () => false,
    page: async () => ({ url: 'https://site.test/app', title: 'App', ready: true }),
  });
  assert.deepEqual(outcome, { navigated: false, url: 'https://site.test/app', title: 'App' });
  assert.ok(waited <= 1100, `waited ${waited} ms`);
});

test('a native submit that never navigates is reported as not navigated after a bounded wait', async () => {
  const { now, sleep } = clock();
  let waited = 0;
  const outcome = await awaitSubmitOutcome({
    now, sleep: async (ms) => { waited += ms; await sleep(ms); }, pageHandled: false,
    replaced: async () => false,
    page: async () => ({ url: 'https://site.test/login', title: 'Login', ready: true }),
  });
  assert.equal(outcome.navigated, false);
  assert.ok(waited >= 8000 && waited <= 8100);
});

test('an unreadable page after the submit is an error, not a silent success', async () => {
  const { now, sleep } = clock();
  await assert.rejects(awaitSubmitOutcome({
    now, sleep, pageHandled: true,
    replaced: async () => false,
    page: async () => { throw new Error('gone'); },
  }), /page unavailable after submit/);
});
