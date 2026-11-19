import { defineApp, createFeed, state, events, connections } from '@chariox/app-sdk-wasm';
import { createApp } from './handler.mjs';

const app = createApp({ state, events, connections, feed: createFeed(state) });
export const handlers = defineApp({ tools: app.tools, events: app.events });
