import { defineApp, state, events } from '@chariox/app-sdk-wasm';
import { createApp } from './handler.mjs';

export const handlers = defineApp(createApp({ state, events }));
