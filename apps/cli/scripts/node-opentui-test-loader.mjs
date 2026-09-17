const opentuiCoreStub = `data:text/javascript,${encodeURIComponent([
  "export const RGBA = { fromHex: (value) => value }",
  "export const SyntaxStyle = { fromStyles: (styles) => styles }",
].join("\n"))}`

export async function resolve(specifier, context, nextResolve) {
  if (specifier === "@opentui/core") {
    return { url: opentuiCoreStub, shortCircuit: true }
  }
  return nextResolve(specifier, context)
}
