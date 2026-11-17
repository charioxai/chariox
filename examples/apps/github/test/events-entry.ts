// Only the event fan-out, as a component of its own: test/component-drill.rs runs it under Wasmtime with in-memory
// state and events (the whole App also imports HTTP, which that drill does not provide).
import { defineApp, state, events } from '@chariox/app-sdk-wasm';
import { createEventsApp } from '../src/events.mjs';

export const handlers = defineApp(createEventsApp({ state, events }));
