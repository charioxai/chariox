import type { IncomingMessage } from "node:http"
import type WebSocket from "ws"

// ws otherwise discards the HTTP diagnostic. Read a bounded 401 body so a
// terminal can find its token and an external agent can find the Unix socket.
export function kernelUpgradeRejectionHandler(
  socket: WebSocket,
  reject: (message: string, authenticationFailed: boolean) => void,
) {
  return (_request: unknown, response: IncomingMessage) => {
    const status = response.statusCode ?? 0
    if (status !== 401) {
      reject(`Unexpected server response: ${status}`, status === 403)
      response.destroy()
      socket.terminate()
      return
    }
    let body = ""
    let finished = false
    const finish = () => {
      if (finished) return
      finished = true
      clearTimeout(timer)
      reject(body.trim() || "Kernel authentication required (HTTP 401). Check the local token file or request access over the Unix socket.", true)
      response.destroy()
      socket.terminate()
    }
    const timer = setTimeout(finish, 1000)
    response.setEncoding("utf8")
    response.on("data", (chunk: string) => {
      body += chunk.slice(0, 8192 - body.length)
      if (body.length === 8192) finish()
    })
    response.once("end", finish)
    response.once("error", finish)
    response.once("aborted", finish)
  }
}
