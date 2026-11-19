import { defineApp, state, events, schedule, host } from '@chariox/app-sdk-wasm';
import { eventRouting } from '@chariox/app-sdk-wasm/event-routing';
import { workspace } from '@chariox/app-sdk-wasm/workspace';
import { createApp } from './src/app.mjs';

const app = createApp({ state, events, routing: eventRouting, workspace, host, schedule });
export const handlers = defineApp({ tools: app.tools, events: app.events, wake: app.wake });
