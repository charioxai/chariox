import { defineApp, http } from '@chariox/app-sdk-wasm';
let timerHits = 0;
export const handlers = defineApp({ tools: {
  async abandon(_input, ctx) {
    const response = await ctx.fetch('https://example.com/');
    return response.status;
  },
  async consume(_input, ctx) {
    const response = await ctx.fetch('https://example.com/');
    for await (const chunk of response.body) break;
    return { done: true };
  },
  async late(_input, ctx) {
    // Deliberately remove QuickJS's optional cancel property to exercise the
    // ownership continuation when a result arrives after the handler returns.
    const pending = http.fetch({ url: 'https://example.com/late', method: 'GET', headers: [], connectionId: null, operationId: null }, null).then(value => value);
    const [head] = await ctx.resources.own(pending, ([, body]) => body.return());
    return head.status;
  },
  async encoding() {
    const message = 'Todo 🍋 日本語';
    return new TextDecoder().decode(new TextEncoder().encode(message));
  },
  async timer() {
    await new Promise(resolve => setTimeout(resolve, 20));
    setTimeout(() => { timerHits++; }, 20);
    return timerHits;
  },
  async timerHits() { return timerHits; },
  async random() { return Math.random(); },
} });
