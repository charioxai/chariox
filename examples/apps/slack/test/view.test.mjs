// The page against the window's own surface: `window.chariox` holds `call` and `changes` and nothing else, and the sandbox has no forms.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const later = () => new Promise(resolve => setImmediate(resolve));

async function view(notifications) {
  const make = () => {
    const value = {
      value: '', dataset: {}, children: [], handlers: {}, hidden: false, textContent: '', attributes: {},
      classList: { toggle() {}, contains: () => false },
      addEventListener(type, handler) { this.handlers[type] = handler; },
      setAttribute(name, attribute) { this.attributes[name] = attribute; },
      append(...children) { this.children.push(...children); },
      replaceChildren(...children) { this.children = children; },
      querySelectorAll: () => [],
      contains: () => false,
    };
    return value;
  };
  const elements = new Map(['notifications', 'status', 'kind', 'empty', 'clear'].map(id => [id, make()]));
  const root = make();
  const calls = [];
  const feed = [];
  const chariox = Object.freeze({
    call: async (method, input) => {
      calls.push([method, input]);
      if (method === 'list_notifications') return { notifications };
      return method === 'reply' ? { posted: true } : { cleared: 1 };
    },
    changes: async (since, waitMs) => {
      if (waitMs === undefined) return { latest: 1, changes: [], dropped: false };
      return new Promise(resolve => feed.push(resolve));
    },
  });
  const context = {
    window: { chariox }, crypto, Date, setTimeout,
    document: { getElementById: id => id === 'app' ? root : elements.get(id), createElement: make },
  };
  vm.runInNewContext(await readFile(new URL('../view/app.js', import.meta.url), 'utf8'), context);
  await later();
  const rows = () => elements.get('notifications').children;
  return { calls, rows, root, elements, feed };
}

const one = [{ id: 'one', kind: 'mentioned', occurred_at: '2026-10-28T10:00:00Z', can_reply: true, text: 'Hello', channel: 'C1', user: 'U1' }];
const replyOf = row => { const [input, button] = row.children.at(-1).children; return { input, button }; };

test('the list is read once at the start and shows each notification with its reply row', async () => {
  const { calls, rows } = await view(one);
  assert.equal(JSON.stringify(calls), JSON.stringify([['list_notifications', { limit: 50 }]]));
  assert.equal(rows().length, 1);
  assert.ok(replyOf(rows()[0]).button, 'a notification that can be answered has a Reply button');
});

test('Reply sends the text with one request id per reply, from the button and from Enter', async () => {
  const { calls, rows } = await view(one);
  calls.length = 0;
  const { input, button } = replyOf(rows()[0]);
  input.value = 'Deploying now';
  await button.handlers.click();
  input.value = 'Deploying now';
  await input.handlers.keydown({ key: 'a' });
  await input.handlers.keydown({ key: 'Enter' });
  await later();
  const sent = calls.filter(([method]) => method === 'reply').map(([, input]) => input);
  assert.equal(sent.length, 2, 'a key other than Enter sends nothing');
  assert.deepEqual(sent.map(({ id, text }) => [id, text]), [['one', 'Deploying now'], ['one', 'Deploying now']]);
  assert.match(sent[0].request_id, /^[0-9a-f-]{36}$/);
});

test('an empty reply is not sent', async () => {
  const { calls, rows } = await view(one);
  calls.length = 0;
  await replyOf(rows()[0]).button.handlers.click();
  assert.deepEqual(calls.map(([method]) => method), []);
});

test('a notification arriving on the change feed is read again', async () => {
  const { calls, feed } = await view(one);
  calls.length = 0;
  feed.shift()({ latest: 2, changes: [{ seq: 2, delta: { id: 'two' } }], dropped: false });
  await later();
  assert.deepEqual(calls.map(([method]) => method), ['list_notifications']);
});

test('the page has no form, which the sandbox would not submit', async () => {
  const { root } = await view(one);
  assert.doesNotMatch(root.innerHTML, /<form/);
});
