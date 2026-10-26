// MP-08 / MP-10 (PANE-09): terminal approximation of LaTeX math. The same
// strict delimiters as the web renderer: `$$…$$` and `\[…\]` display, `\(…\)`
// inline, `$…$` only when unambiguous (no space just inside the delimiters,
// no digit after the closing `$`, no word character before the opening `$`,
// one line). Supported TeX becomes Unicode (Greek, operators, sub/superscripts,
// fractions as a/b, roots as √); anything else keeps its raw source.

const symbols: Record<string, string> = {
  alpha: "α", beta: "β", gamma: "γ", delta: "δ", epsilon: "ϵ", varepsilon: "ε", zeta: "ζ", eta: "η",
  theta: "θ", vartheta: "ϑ", iota: "ι", kappa: "κ", lambda: "λ", mu: "μ", nu: "ν", xi: "ξ", pi: "π",
  varpi: "ϖ", rho: "ρ", varrho: "ϱ", sigma: "σ", varsigma: "ς", tau: "τ", upsilon: "υ", phi: "ϕ",
  varphi: "φ", chi: "χ", psi: "ψ", omega: "ω",
  Gamma: "Γ", Delta: "Δ", Theta: "Θ", Lambda: "Λ", Xi: "Ξ", Pi: "Π", Sigma: "Σ", Upsilon: "Υ",
  Phi: "Φ", Psi: "Ψ", Omega: "Ω",
  cdot: "⋅", times: "×", div: "÷", pm: "±", mp: "∓", ast: "∗", star: "⋆", circ: "∘", bullet: "∙",
  le: "≤", leq: "≤", ge: "≥", geq: "≥", ne: "≠", neq: "≠", approx: "≈", equiv: "≡", sim: "∼",
  simeq: "≃", cong: "≅", propto: "∝", ll: "≪", gg: "≫",
  infty: "∞", partial: "∂", nabla: "∇", sum: "∑", prod: "∏", coprod: "∐", int: "∫", iint: "∬",
  iiint: "∭", oint: "∮", sqrt: "√",
  to: "→", rightarrow: "→", leftarrow: "←", gets: "←", Rightarrow: "⇒", Leftarrow: "⇐",
  implies: "⇒", iff: "⇔", Leftrightarrow: "⇔", leftrightarrow: "↔", mapsto: "↦", uparrow: "↑",
  downarrow: "↓", longrightarrow: "⟶",
  in: "∈", notin: "∉", ni: "∋", subset: "⊂", subseteq: "⊆", supset: "⊃", supseteq: "⊇", cup: "∪",
  cap: "∩", setminus: "∖", emptyset: "∅", varnothing: "∅", forall: "∀", exists: "∃", nexists: "∄",
  neg: "¬", lnot: "¬", land: "∧", wedge: "∧", lor: "∨", vee: "∨", oplus: "⊕", otimes: "⊗",
  ldots: "…", dots: "…", cdots: "⋯", vdots: "⋮", ddots: "⋱", prime: "′", degree: "°", angle: "∠",
  perp: "⊥", parallel: "∥", mid: "∣", hbar: "ℏ", ell: "ℓ", Re: "ℜ", Im: "ℑ", aleph: "ℵ",
  langle: "⟨", rangle: "⟩", lfloor: "⌊", rfloor: "⌋", lceil: "⌈", rceil: "⌉", vert: "|", Vert: "‖",
  "|": "‖", "{": "{", "}": "}", "%": "%", "$": "$", "&": "&", "#": "#", "_": "_",
  ",": " ", ";": " ", ":": " ", "!": "", " ": " ", quad: " ", qquad: "  ",
  sin: "sin", cos: "cos", tan: "tan", cot: "cot", sec: "sec", csc: "csc", arcsin: "arcsin",
  arccos: "arccos", arctan: "arctan", sinh: "sinh", cosh: "cosh", tanh: "tanh", log: "log", ln: "ln",
  lg: "lg", exp: "exp", lim: "lim", max: "max", min: "min", sup: "sup", inf: "inf", det: "det",
  gcd: "gcd", deg: "deg", dim: "dim", ker: "ker", arg: "arg", Pr: "Pr", mod: "mod", bmod: "mod",
}
const ignored = new Set(["left", "right", "big", "Big", "bigg", "Bigg", "bigl", "bigr", "Bigl", "Bigr", "displaystyle", "textstyle", "limits", "nolimits"])
const textCommands = new Set(["text", "textrm", "textit", "textbf", "mathrm", "mathit", "mathbf", "mathsf", "mathtt", "operatorname", "mbox", "boldsymbol", "bm", "mathcal", "mathscr"])
const blackboard: Record<string, string> = { N: "ℕ", Z: "ℤ", Q: "ℚ", R: "ℝ", C: "ℂ", P: "ℙ", H: "ℍ", E: "𝔼" }
const superscripts: Record<string, string> = {
  0: "⁰", 1: "¹", 2: "²", 3: "³", 4: "⁴", 5: "⁵", 6: "⁶", 7: "⁷", 8: "⁸", 9: "⁹", "+": "⁺", "-": "⁻", "−": "⁻",
  "=": "⁼", "(": "⁽", ")": "⁾", a: "ᵃ", b: "ᵇ", c: "ᶜ", d: "ᵈ", e: "ᵉ", f: "ᶠ", g: "ᵍ", h: "ʰ", i: "ⁱ",
  j: "ʲ", k: "ᵏ", l: "ˡ", m: "ᵐ", n: "ⁿ", o: "ᵒ", p: "ᵖ", r: "ʳ", s: "ˢ", t: "ᵗ", u: "ᵘ", v: "ᵛ", w: "ʷ",
  x: "ˣ", y: "ʸ", z: "ᶻ", T: "ᵀ", "′": "′", "∗": "*",
}
const subscripts: Record<string, string> = {
  0: "₀", 1: "₁", 2: "₂", 3: "₃", 4: "₄", 5: "₅", 6: "₆", 7: "₇", 8: "₈", 9: "₉", "+": "₊", "-": "₋", "−": "₋",
  "=": "₌", "(": "₍", ")": "₎", a: "ₐ", e: "ₑ", h: "ₕ", i: "ᵢ", j: "ⱼ", k: "ₖ", l: "ₗ", m: "ₘ", n: "ₙ",
  o: "ₒ", p: "ₚ", r: "ᵣ", s: "ₛ", t: "ₜ", u: "ᵤ", v: "ᵥ", x: "ₓ",
}

class UnsupportedTex extends Error {}

export function renderTranscriptMath(text: string): string {
  let output = ""
  let index = 0
  while (index < text.length) {
    const found = findMath(text, index)
    if (!found) break
    output += text.slice(index, found.start)
    const unicode = texToUnicode(found.tex)
    output += unicode === null
      ? text.slice(found.start, found.end)
      : found.display && !inMarkdownContainer(text, found.start)
        ? `\n\n${escapeMarkdown(unicode)}\n\n` : escapeMarkdown(unicode)
    index = found.end
  }
  return (output + text.slice(index)).replace(/\n{3,}/g, "\n\n")
}

export function texToUnicode(tex: string): string | null {
  try {
    const parser = { tex, index: 0 }
    const result = parseSequence(parser, false)
    return result.replace(/\s+/g, " ").trim() || null
  } catch (error) {
    if (error instanceof UnsupportedTex) return null
    throw error
  }
}

function findMath(text: string, from: number): { start: number; end: number; tex: string; display: boolean } | null {
  for (let index = from; index < text.length; index += 1) {
    const char = text[index]
    if (char === "\\") {
      const next = text[index + 1]
      if (next === "(" || next === "[") {
        const close = next === "(" ? "\\)" : "\\]"
        const end = text.indexOf(close, index + 2)
        const tex = end < 0 ? "" : text.slice(index + 2, end)
        if (tex.trim()) return { start: index, end: end + 2, tex, display: next === "[" }
      }
      index += 1
      continue
    }
    if (char !== "$") continue
    if (text[index + 1] === "$") {
      const end = text.indexOf("$$", index + 2)
      const tex = end < 0 ? "" : text.slice(index + 2, end)
      if (tex.trim()) return { start: index, end: end + 2, tex, display: true }
      index += 1
      continue
    }
    const before = index > 0 ? text[index - 1]! : ""
    if (/[\p{L}\p{N}$]/u.test(before)) continue
    const line = text.slice(index + 1, text.indexOf("\n", index) < 0 ? text.length : text.indexOf("\n", index))
    let close = -1
    for (let cursor = 0; cursor < line.length; cursor += 1) {
      if (line[cursor] === "\\") {
        cursor += 1
        continue
      }
      if (line[cursor] === "$") {
        close = cursor
        break
      }
    }
    if (close <= 0) continue
    const tex = line.slice(0, close)
    const after = line[close + 1] ?? ""
    if (/^\s/.test(tex) || /\s$/.test(tex) || /\d/.test(after)) continue
    return { start: index, end: index + close + 2, tex, display: false }
  }
  return null
}

type Parser = { readonly tex: string; index: number }

function parseSequence(parser: Parser, inGroup: boolean): string {
  let output = ""
  while (parser.index < parser.tex.length) {
    const char = parser.tex[parser.index]!
    if (char === "}") {
      if (!inGroup) throw new UnsupportedTex()
      parser.index += 1
      return output
    }
    if (char === "^" || char === "_") {
      parser.index += 1
      skipSpaces(parser)
      output = output.trimEnd()
      output += script(parseAtom(parser), char === "^" ? superscripts : subscripts, char)
      continue
    }
    output += parseAtom(parser)
  }
  if (inGroup) throw new UnsupportedTex()
  return output
}

function parseAtom(parser: Parser): string {
  const tex = parser.tex
  const char = tex[parser.index]
  if (char === undefined) throw new UnsupportedTex()
  if (char === "{") {
    parser.index += 1
    return parseSequence(parser, true)
  }
  if (char === "\\") return parseCommand(parser)
  if (char === "&" || char === "^" || char === "_") throw new UnsupportedTex()
  parser.index += 1
  if (char === "~") return " "
  if (char === "'") return "′"
  if (char === "-") return "−"
  return char
}

function parseCommand(parser: Parser): string {
  const tex = parser.tex
  parser.index += 1
  const match = /^(?:[A-Za-z]+|.)/.exec(tex.slice(parser.index))
  if (!match) throw new UnsupportedTex()
  const name = match[0]
  parser.index += name.length
  if (name === "frac" || name === "dfrac" || name === "tfrac") {
    skipSpaces(parser)
    const numerator = parseAtom(parser)
    skipSpaces(parser)
    const denominator = parseAtom(parser)
    return `${wrap(numerator)}/${wrap(denominator)}`
  }
  if (name === "sqrt") {
    skipSpaces(parser)
    let index = ""
    if (tex[parser.index] === "[") {
      const end = tex.indexOf("]", parser.index)
      if (end < 0) throw new UnsupportedTex()
      index = script(tex.slice(parser.index + 1, end), superscripts, "^")
      parser.index = end + 1
    }
    skipSpaces(parser)
    return `${index}√${wrap(parseAtom(parser))}`
  }
  if (name === "mathbb" || textCommands.has(name)) skipSpaces(parser)
  if (name === "mathbb") {
    const letter = parseAtom(parser)
    return blackboard[letter] ?? letter
  }
  if (textCommands.has(name)) return parseAtom(parser)
  if (ignored.has(name)) {
    if (tex[parser.index] === ".") parser.index += 1
    return ""
  }
  const symbol = symbols[name]
  if (symbol === undefined) throw new UnsupportedTex()
  return /^[a-z]{2,}$/.test(symbol) ? ` ${symbol} ` : symbol
}

function script(value: string, map: Record<string, string>, marker: string): string {
  const compact = value.replace(/\s+/g, "")
  const mapped = [...compact].map((char) => map[char])
  if (compact && mapped.every((char) => char !== undefined)) return mapped.join("")
  return compact.length === 1 ? `${marker}${compact}` : `${marker}(${compact})`
}

function wrap(value: string): string {
  const trimmed = value.trim()
  return /^(?:\p{N}+(?:\.\p{N}+)?|\p{L}[′⁰¹²³⁴⁵⁶⁷⁸⁹₀₁₂₃₄₅₆₇₈₉]*)$/u.test(trimmed) ? trimmed : `(${trimmed})`
}

function skipSpaces(parser: Parser): void {
  while (parser.tex[parser.index] === " ") parser.index += 1
}

function escapeMarkdown(value: string): string {
  return value.replace(/[\\`*_[\]<>|~]/g, "\\$&")
}

function inMarkdownContainer(text: string, start: number): boolean {
  const prefix = text.slice(text.lastIndexOf("\n", start - 1) + 1, start)
  return /^(?:\s*(?:>\s*)+|\s*(?:[-+*]|\d+[.)])\s+|[ \t]{2,})$/.test(prefix)
}
