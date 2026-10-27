export async function requestJson(baseUrl, method, path, body = undefined, { timeoutMs = 5000 } = {}) {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)
  try {
    const response = await fetch(`${baseUrl}${path}`, {
      method,
      headers: { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: controller.signal,
    })
    const text = await response.text()
    const parsed = text ? JSON.parse(text) : null
    if (!response.ok) {
      const error = new Error(`HTTP ${response.status} ${method} ${path}: ${text}`)
      error.status = response.status
      error.body = parsed
      throw error
    }
    return parsed
  } finally {
    clearTimeout(timer)
  }
}

export async function waitForJsonHealth(baseUrl, path, predicate, { timeoutMs = 30000, pollMs = 250 } = {}) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const value = await requestJson(baseUrl, 'GET', path, undefined, { timeoutMs: Math.min(2000, pollMs + 1500) })
      if (predicate(value)) return value
      lastError = new Error(`health predicate failed for ${JSON.stringify(value)}`)
    } catch (error) {
      lastError = error
    }
    await new Promise((resolve) => setTimeout(resolve, pollMs))
  }
  throw new Error(`timed out waiting for ${baseUrl}${path}: ${lastError?.message ?? 'unknown error'}`)
}
