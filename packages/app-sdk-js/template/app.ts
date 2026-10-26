import { defineApp, state } from '@chariox/app-sdk-wasm';

export const handlers = defineApp({
  tools: {
    async list() {
      const record = await state.get('todos');
      return record ? JSON.parse(record.value) : [];
    },
    async add(input: { title: string }, ctx) {
      if (typeof input?.title !== 'string' || !input.title) throw { code: 'INVALID_ARGUMENT', message: 'title is required' };
      const record = await ctx.resources.own(state.get('todos'), () => {});
      const todos = record ? JSON.parse(record.value) : [];
      todos.push({ title: input.title });
      ctx.signal.throwIfAborted();
      await state.transact({
        schemaVersion: 0, checks: [{ key: 'todos', version: record?.version ?? null }],
        writes: [{ tag: 'put', val: ['todos', JSON.stringify(todos)] }], occurrences: [], wakes: [],
      });
      return todos;
    },
  },
});
