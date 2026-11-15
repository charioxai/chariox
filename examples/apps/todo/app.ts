import { defineApp, createFeed, state, events } from '@chariox/app-sdk-wasm';
import { createApp } from './handler.mjs';

const app = createApp({ state, events, feed: createFeed(state) });
export const handlers = defineApp({ tools: app.tools, events: app.events, wake: app.wake });
