export async function withCdpSocket({
  callback,
  commandTimeoutMs = 5_000,
  connectTimeoutMs = 5_000,
  settleMs = 0,
  url,
  WebSocketImpl = globalThis.WebSocket,
}) {
  const socket = new WebSocketImpl(url)
  const pending = new Map()
  let closed = false

  const failPending = (error) => {
    for (const [id, request] of pending) {
      pending.delete(id)
      request.reject(error)
    }
  }
  socket.addEventListener("message", (event) => {
    let message
    try {
      message = JSON.parse(event.data)
    } catch {
      return
    }
    const request = pending.get(message.id)
    if (!request) return
    pending.delete(message.id)
    request.resolve(message)
  })
  socket.addEventListener("error", () => {
    failPending(new Error("CDP socket failed"))
  })
  socket.addEventListener("close", () => {
    closed = true
    failPending(new Error("CDP socket closed"))
  })

  try {
    await waitForSocketOpen(socket, connectTimeoutMs)
    let nextId = 1
    const send = async (method, params = {}) => {
      const id = nextId++
      const response = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          pending.delete(id)
          reject(new Error(`${method}: CDP command timed out`))
        }, commandTimeoutMs)
        pending.set(id, {
          resolve(value) {
            clearTimeout(timer)
            resolve(value)
          },
          reject(error) {
            clearTimeout(timer)
            reject(error)
          },
        })
        try {
          socket.send(JSON.stringify({ id, method, params }))
        } catch (error) {
          const request = pending.get(id)
          pending.delete(id)
          request?.reject(error)
        }
      })
      if (response.error) {
        throw new Error(`${method}: ${response.error.message}`)
      }
      return response.result
    }

    const result = await callback(send)
    if (settleMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, settleMs))
    }
    return result
  } finally {
    failPending(new Error("CDP socket closed"))
    if (!closed) socket.close()
  }
}

async function waitForSocketOpen(socket, timeoutMs) {
  if (socket.readyState === 1) return
  await new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timer)
      socket.removeEventListener("open", onOpen)
      socket.removeEventListener("error", onError)
      socket.removeEventListener("close", onClose)
    }
    const onOpen = () => {
      cleanup()
      resolve()
    }
    const onError = () => {
      cleanup()
      reject(new Error("CDP socket failed before opening"))
    }
    const onClose = () => {
      cleanup()
      reject(new Error("CDP socket closed before opening"))
    }
    const timer = setTimeout(() => {
      cleanup()
      reject(new Error("CDP socket did not open in time"))
    }, timeoutMs)
    socket.addEventListener("open", onOpen)
    socket.addEventListener("error", onError)
    socket.addEventListener("close", onClose)
  })
}
