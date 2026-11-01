import Fastify from "fastify"

export function installRawBodyParsers(app: ReturnType<typeof Fastify>) {
  for (const type of ["application/json", "application/x-www-form-urlencoded", "text/plain"]) app.removeContentTypeParser(type)
  app.addContentTypeParser("application/json", { parseAs: "string" }, (request: { raw: { charioxRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    try {
      done(null, body ? JSON.parse(body) : {})
    } catch (error) {
      done(Object.assign(error as Error, { statusCode: 400 }))
    }
  })

  app.addContentTypeParser("application/x-www-form-urlencoded", { parseAs: "string" }, (request: { raw: { charioxRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    done(null, Object.fromEntries(new URLSearchParams(body)))
  })

  app.addContentTypeParser("text/plain", { parseAs: "string" }, (request: { raw: { charioxRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    done(null, body)
  })
}

function setRawRequestBody(request: { raw: { charioxRawBody?: string } }, body: string) {
  request.raw.charioxRawBody = body
}
