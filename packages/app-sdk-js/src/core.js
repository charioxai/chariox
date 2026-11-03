export const appError = thrown => {
  const value = thrown?.payload ?? thrown;
  return {
    code: typeof value?.code === 'string' ? value.code : 'HANDLER_FAILED',
    message: String(value?.message ?? value).slice(0, 4096),
    retryable: typeof value?.retryable === 'boolean' ? value.retryable : null,
  };
};

const cancelled = reason => ({ code: 'CANCELLED', message: String(reason), retryable: false });
const encode = value => {
  try {
    const text = JSON.stringify(value);
    if (typeof text !== 'string') throw new TypeError('handler result is not JSON');
    return text;
  } catch (error) {
    throw { code: 'INVALID_RESULT', message: String(error.message), retryable: null };
  }
};

/** An invocation owns returned resources, including results of late imports.
 * Keep the completion continuation after Promise.race: cancellation may race
 * with a host result, and a subsequently delivered stream must still be dropped.
 */
export class ResourceScope {
  #resources = new Map();
  #pending = new Set();
  #closed = false;
  #closing;
  #controller = new AbortController();
  get signal() { return this.#controller.signal; }

  async own(pending, dispose) {
    this.#pending.add(pending);
    let value;
    try { value = await pending; }
    finally { this.#pending.delete(pending); }
    if (this.#closed) {
      await dispose(value);
      throw cancelled(this.signal.reason);
    }
    this.#resources.set(value, dispose);
    return value;
  }

  async release(value) {
    const dispose = this.#resources.get(value);
    this.#resources.delete(value);
    if (dispose) await dispose(value);
  }

  close(reason = 'invocation finished') {
    if (this.#closing) return this.#closing;
    let resolve, reject;
    this.#closing = new Promise((yes, no) => { resolve = yes; reject = no; });
    this.#closed = true;
    this.#finishClose(reason).then(resolve, reject);
    return this.#closing;
  }

  async #finishClose(reason) {
    const failures = [];
    try { this.#controller.abort(reason); } catch (error) { failures.push(error); }
    for (const pending of this.#pending) {
      try { pending.cancel?.(); } catch (error) { failures.push(error); }
    }
    const releases = [...this.#resources].map(([value, dispose]) => {
      this.#resources.delete(value);
      return Promise.resolve().then(() => dispose(value));
    });
    const results = await Promise.allSettled(releases);
    const failed = results.find(result => result.status === 'rejected');
    if (failed) failures.push(failed.reason);
    if (failures.length) throw failures[0];
  }
}

export function createHandlers(app, host) {
  const dispatch = async (fn, input, context, cancel) => {
    const resources = new ResourceScope();
    let wasCancelled = false, cancelReason;
    let readSettled = false;
    // Both outcomes have rejection handlers, even after the other wins.
    const cancelRead = cancel.read().then(value => {
      readSettled = true; return value;
    }, error => { readSettled = true; throw error; });
    const cancellation = cancelRead.then(reason => {
      wasCancelled = true; cancelReason = reason;
      throw cancelled(reason);
    });
    try {
      const ctx = { ...context, signal: resources.signal, resources };
      if (host.context) Object.assign(ctx, host.context(resources));
      const work = Promise.resolve().then(() => {
        if (wasCancelled) throw cancelled(cancelReason);
        return fn(input, ctx);
      });
      return await Promise.race([work, cancellation]);
    } catch (error) {
      throw appError(error);
    } finally {
      try {
        try { await host.finishInvocation?.(); }
        finally { await resources.close(); }
      } catch (error) {
        throw { code: 'RESOURCE_CLOSE_FAILED', message: appError(error).message, retryable: false };
      } finally {
        // An endpoint cannot be dropped while its read is in flight. Await its
        // cancellation acknowledgement even when the handler completed normally.
        if (typeof cancel.cancelRead === 'function') {
          try {
            if (!readSettled) cancel.cancelRead();
            await cancelRead.catch(() => {});
            cancel.drop();
          } catch (error) {
            throw { code: 'RESOURCE_CLOSE_FAILED', message: appError(error).message, retryable: false };
          }
        }
      }
    }
  };
  const handler = (table, name) => {
    const fn = Object.hasOwn(table ?? {}, name) ? table[name] : null;
    return fn ?? (() => { throw { code: 'HANDLER_NOT_FOUND', message: name }; });
  };
  return {
    register() {
      try {
        for (const name of Object.keys(app.tools ?? {})) host.registry.tool(name);
        for (const name of Object.keys(app.events ?? {})) host.registry.event(name);
        for (const name of Object.keys(app.lifecycle ?? {})) host.registry.lifecycle(name);
      } catch (error) { throw appError(error); }
    },
    async callTool(name, input, context, cancel) {
      return encode(await dispatch((raw, ctx) => handler(app.tools, name)(JSON.parse(raw), ctx), input, context, cancel));
    },
    async deliverEvent(name, occurrenceId, payload, context, cancel) {
      return encode(await dispatch((raw, ctx) => handler(app.events, name)(JSON.parse(raw), { ...ctx, occurrenceId }), payload, context, cancel));
    },
    async lifecycle(event, data, context, cancel) {
      const result = await dispatch((raw, ctx) => Object.hasOwn(app.lifecycle ?? {}, event)
        ? app.lifecycle[event](raw == null ? null : JSON.parse(raw), ctx) : undefined, data, context, cancel);
      return result === undefined ? null : encode(result);
    },
    async deliverWake(wake, overdue, context, cancel) {
      await dispatch((_, ctx) => {
        if (!app.wake) throw { code: 'WAKE_NOT_FOUND', message: wake.id };
        return app.wake(wake, { ...ctx, overdue });
      }, null, context, cancel);
    },
  };
}
