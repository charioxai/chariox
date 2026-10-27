import process from "node:process"

let input = ""
for await (const chunk of process.stdin) input += chunk
const document = JSON.parse(input)
delete document.serialNumber
if (document.metadata && typeof document.metadata === "object") {
  delete document.metadata.timestamp
}

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(
    Object.keys(value).sort().map((key) => [key, canonical(value[key])]),
  )
}

process.stdout.write(`${JSON.stringify(canonical(document))}\n`)
