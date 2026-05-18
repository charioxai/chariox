import Fastify from "fastify"

export function installRawBodyParsers(app: ReturnType<typeof Fastify>) {
  app.removeContentTypeParser("application/json")
  app.addContentTypeParser("application/json", { parseAs: "string" }, (request: { raw: { arrobaRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    try {
      done(null, body ? JSON.parse(body) : {})
    } catch (error) {
      done(error as Error)
    }
  })

  app.addContentTypeParser("application/x-www-form-urlencoded", { parseAs: "string" }, (request: { raw: { arrobaRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    done(null, Object.fromEntries(new URLSearchParams(body)))
  })

  app.addContentTypeParser("text/plain", { parseAs: "string" }, (request: { raw: { arrobaRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    done(null, body)
  })
}

function setRawRequestBody(request: { raw: { arrobaRawBody?: string } }, body: string) {
  request.raw.arrobaRawBody = body
}
