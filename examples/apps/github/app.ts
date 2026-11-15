import { defineApp, state, events } from '@chariox/app-sdk-wasm';
import { createApp } from './src/app.mjs';

export const handlers = defineApp(createApp({ state, events }));
