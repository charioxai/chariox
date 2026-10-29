// MP-08 / MP-10 (PANE-09): shared machinery for content that needs a lazily
// loaded renderer (code highlighting, math). Rendering stays a pure string
// function: a miss emits a fallback tagged with a key and queues the input;
// after a short debounce the renderer module is loaded once, visible tagged
// elements are patched in place, and the result is cached so the next string
// render of the same input is complete. `null` results (unsupported input,
// invalid input) are cached too. Transport errors retry up to three times so a
// transient chunk failure cannot permanently freeze a visible fallback.

export type LazyRenderer<TInput> = {
  readonly key: (input: TInput) => string
  readonly cached: (key: string) => string | null | undefined
  readonly request: (key: string, input: TInput) => void
  readonly flush: () => Promise<void>
  readonly setRenderForTests: (render: ((input: TInput) => Promise<string | null>) | null) => void
}

const maxCached = 400
const delayMs = 120
export function createLazyRenderer<TInput>(options: {
  readonly attribute: string
  readonly available: () => boolean
  readonly load: () => Promise<((input: TInput) => Promise<string | null>) | null>
  readonly apply: (element: HTMLElement, html: string | null) => void
  readonly maxInputChars?: number
}): LazyRenderer<TInput> {
  const cache = new Map<string, { readonly input: string | null; readonly html: string | null }>()
  // Keep the identity as long as its rendered result. Streaming inputs that
  // never reach flush may churn the shared registry without expiring a result.
  const cachedKeys = new Map<string, string>()
  const pending = new Map<string, { readonly input: TInput; readonly attempt: number; readonly failures: number }>()
  let renderPromise: Promise<((input: TInput) => Promise<string | null>) | null> | null = null
  let override: ((input: TInput) => Promise<string | null>) | null = null
  let timer: ReturnType<typeof setTimeout> | null = null

  const schedule = () => {
    timer ??= setTimeout(() => {
      timer = null
      void flush()
    }, delayMs)
  }
  const loadRender = () => {
    if (override) return Promise.resolve(override)
    renderPromise ??= options.load().catch(() => {
      renderPromise = null
      return null
    })
    return renderPromise
  }
  const remember = (key: string, input: string | null, html: string | null) => {
    cache.set(key, { input, html })
    if (input !== null) cachedKeys.set(input, key)
    if (cache.size > maxCached) {
      const oldest = cache.keys().next().value
      if (oldest !== undefined) {
        const input = cache.get(oldest)!.input
        if (input !== null && cachedKeys.get(input) === oldest) cachedKeys.delete(input)
        cache.delete(oldest)
      }
    }
  }
  const retry = (key: string, input: TInput, attempt: number, failures: number) => {
    if (failures < 2) {
      pending.set(key, { input, attempt, failures: failures + 1 })
      schedule()
    } else {
      remember(key, JSON.stringify(input), null)
      if (typeof document !== "undefined") {
        for (const element of document.querySelectorAll<HTMLElement>(`[${options.attribute}="${key}"]`)) options.apply(element, null)
      }
    }
  }
  const flush = async () => {
    const batch = [...pending]
    pending.clear()
    if (batch.length === 0) return
    const render = await loadRender()
    if (!render) {
      for (const [key, { input, attempt, failures }] of batch) retry(key, input, attempt, failures)
      return
    }
    for (const [key, { input, attempt, failures }] of batch) {
      const elements = typeof document === "undefined"
        ? []
        : [...document.querySelectorAll<HTMLElement>(`[${options.attribute}="${key}"]`)]
      // Streaming content changes on every delta; skip inputs no longer on
      // screen (after one retry for markup rendered but not yet attached).
      if (typeof document !== "undefined" && elements.length === 0) {
        if (attempt === 0) {
          pending.set(key, { input, attempt: 1, failures })
          schedule()
        }
        continue
      }
      let html: string | null
      try { html = await render(input) }
      catch { retry(key, input, attempt, failures); continue }
      remember(key, JSON.stringify(input), html)
      for (const element of elements) options.apply(element, html)
    }
  }

  return {
    key: (input) => {
      const serialized = JSON.stringify(input)
      if (options.maxInputChars !== undefined && serialized.length > options.maxInputChars) return "oversized"
      return cachedKeys.get(serialized) ?? contentKey(serialized)
    },
    cached: (key) => key === "oversized" ? null : cache.get(key)?.html,
    request: (key, input) => {
      if (key === "oversized") return
      if (cache.has(key) || pending.has(key)) return
      if (options.maxInputChars !== undefined && JSON.stringify(input).length > options.maxInputChars) {
        remember(key, null, null)
        return
      }
      if (!override && !options.available()) return
      pending.set(key, { input, attempt: 0, failures: 0 })
      schedule()
    },
    flush,
    setRenderForTests: (render) => {
      override = render
      renderPromise = null
      cache.clear()
      cachedKeys.clear()
      pending.clear()
    },
  }
}

// MP-08 / MP-10: compare exact inputs and never reuse an ID. Share the
// registry across duplicate bundles in the same document so separate renderers
// cannot patch another bundle's slots with different content.
const keyRegistrySymbol = Symbol.for("chariox.markdown.lazy-input-keys.v1")
type KeyRegistry = { next: number; inputs: Map<string, string> }
const keyGlobal = globalThis as typeof globalThis & { [keyRegistrySymbol]?: KeyRegistry }
const keyRegistry = keyGlobal[keyRegistrySymbol] ??= { next: 0, inputs: new Map() }

export function contentKey(input: string): string {
  const previous = keyRegistry.inputs.get(input)
  if (previous !== undefined) {
    keyRegistry.inputs.delete(input)
    keyRegistry.inputs.set(input, previous)
    return previous
  }
  const key = (++keyRegistry.next).toString(36)
  keyRegistry.inputs.set(input, key)
  if (keyRegistry.inputs.size > maxCached) {
    const oldest = keyRegistry.inputs.keys().next().value
    if (oldest !== undefined) keyRegistry.inputs.delete(oldest)
  }
  return key
}
