import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

test('Slack reply form uses the submit caller', async () => {
  const nodes = [];
  const node = () => {
    const value = { value: '', dataset: {}, children: [], classList: { toggle() {}, contains: () => false },
      addEventListener() {}, setAttribute() {}, append(...children) { this.children.push(...children); },
      replaceChildren(...children) { this.children = children; }, querySelectorAll: () => [] };
    nodes.push(value); return value;
  };
  const elements = new Map(['notifications','status','kind','empty','clear'].map(id => [id,node()]));
  let handler;
  const ambient = [], scoped = [];
  const chariox = {
    call: async method => { ambient.push(method); return { notifications: [{ id:'one',kind:'mentioned',occurred_at:'2026-10-28',can_reply:true,text:'Hello' }] }; },
    form: (_form, registered) => { handler = registered; },
  };
  vm.runInNewContext(await readFile(new URL('../bundle/ui/app.js', import.meta.url), 'utf8'), {
    window:{chariox}, document:{getElementById:id=>elements.get(id),createElement:node}, setInterval() {}, Date, crypto,
  });
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(typeof handler,'function','the async reply form explicitly registers its handler');
  ambient.length = 0;
  await handler({}, {call:async (method,input)=>{scoped.push({method,input});return {posted:true};}});
  assert.equal(scoped.length,1);
  assert.equal(scoped[0].method,'reply');
  assert.equal(scoped[0].input.id,'one');
  assert.deepEqual(ambient,[]);
});
