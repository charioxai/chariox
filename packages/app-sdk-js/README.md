# JS/TS App SDK

`defineApp` exports all R1 handlers, registers the tool/event/lifecycle names,
and gives each concurrent invocation its own cancellation signal and resource
scope. The [Todo template](template/app.ts) uses durable state with optimistic
checks. Input schemas and capabilities still belong in the signed package.

```ts
import { defineApp } from '@chariox/app-sdk-wasm';
export const handlers = defineApp({ tools: {
  async read(input: { url: string }, ctx) {
    const response = await ctx.fetch(input.url);
    return { status: response.status, text: await response.text() };
  },
} });
```

Use `ctx.fetch` and `ctx.subscribe` when work belongs to a handler. They own late
responses as well as already returned streams. A `Promise.race` alone does not
release a response whose import is still pending. Raw interfaces are re-exported
for advanced use and require explicit ownership. Use `ctx.resources.own(promise,
dispose)` for another resource-valued import.

See [builds, polyfill limits and integration gates](../app-sdk-tools/README.md).
