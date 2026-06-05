import net from "node:net"

const host = process.argv[2] ?? "127.0.0.1"
const port = Number(process.argv[3] ?? 43118)
const timeoutMs = Number(process.argv[4] ?? 20_000)
const deadline = Date.now() + timeoutMs
let lastError = null

while (Date.now() < deadline) {
  try {
    await connect(host, port)
    process.exit(0)
  } catch (error) {
    lastError = error
    await sleep(200)
  }
}

console.error(`timed out waiting for ${host}:${port}: ${lastError?.message ?? "not reachable"}`)
process.exit(1)

function connect(host, port) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host, port })
    const timeout = setTimeout(() => {
      socket.destroy()
      reject(new Error("timeout"))
    }, 1_000)
    socket.once("connect", () => {
      clearTimeout(timeout)
      socket.end()
      resolve()
    })
    socket.once("error", (error) => {
      clearTimeout(timeout)
      reject(error)
    })
  })
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
