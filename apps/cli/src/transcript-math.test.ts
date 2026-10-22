import assert from "node:assert/strict"
import test from "node:test"

import { renderTranscriptMath, texToUnicode } from "./transcript-math.js"
import { prepareTranscriptMarkdown } from "./transcript-rich-markdown.js"

// MP-08 / MP-10 (PANE-09): LaTeX approximated as Unicode in the terminal.
test("common TeX becomes Unicode", () => {
  assert.equal(texToUnicode("E = mc^2"), "E = mc²")
  assert.equal(texToUnicode("\\int_0^1 x^2\\,dx = \\frac{1}{3}"), "∫₀¹ x² dx = 1/3")
  assert.equal(texToUnicode("\\sum_{i=1}^{n} x_i"), "∑ᵢ₌₁ⁿ xᵢ")
  assert.equal(texToUnicode("\\frac{a+b}{2c} \\le \\sqrt{x^2 + y^2}"), "(a+b)/(2c) ≤ √(x² + y²)")
  assert.equal(texToUnicode("\\alpha \\cdot \\beta \\neq \\Omega, x \\in \\mathbb{R}"), "α ⋅ β ≠ Ω, x ∈ ℝ")
  assert.equal(texToUnicode("\\lim_{n \\to \\infty} \\left( 1 + \\frac{1}{n} \\right)^n"), "lim_(n→∞) ( 1 + 1/n )ⁿ")
  assert.equal(texToUnicode("x_{\\text{max}}"), "xₘₐₓ")
  assert.equal(texToUnicode("x_{\\text{best}}"), "x_(best)")
})

test("unsupported TeX keeps the raw source", () => {
  assert.equal(texToUnicode("\\begin{pmatrix} a & b \\end{pmatrix}"), null)
  assert.equal(texToUnicode("\\frac{1"), null)
  assert.equal(renderTranscriptMath("see $\\unknowncmd{x}$ here"), "see $\\unknowncmd{x}$ here")
})

test("strict delimiters: prices, spaced and escaped dollars stay text", () => {
  for (const text of ["It costs $5 and $10.", "Between $10-$20", "US$5", "a $ b $ c", "pay \\$5 and \\$6", "$x$5"]) {
    assert.equal(renderTranscriptMath(text), text, text)
  }
  assert.equal(renderTranscriptMath("inline $x^2$ and \\(\\alpha\\)"), "inline x² and α")
})

test("display math gets its own paragraph and markdown specials are escaped", () => {
  assert.equal(renderTranscriptMath("and the display:\n\n$$\\int_0^1 x^2\\,dx = \\frac{1}{3}$$\n"), "and the display:\n\n∫₀¹ x² dx = 1/3\n\n")
  assert.equal(renderTranscriptMath("\\[a_{bc} * [x]\\]"), "\n\na\\_(bc) \\* \\[x\\]\n\n")
})

test("math in fences and code spans stays literal through the preprocessor", () => {
  assert.equal(prepareTranscriptMarkdown("`$x^2$` and $x^2$\n```tex\n$$y^2$$\n```"), "`$x^2$` and x²\n```tex\n$$y^2$$\n```")
})

// MP-08 / MP-10: display math must not leave its quote/list container.
test("display math keeps blockquote and list containers", () => {
  assert.equal(prepareTranscriptMarkdown("> $$x^2$$"), "> x²")
  assert.equal(prepareTranscriptMarkdown("- $$x^2$$"), "- x²")
  assert.equal(prepareTranscriptMarkdown("- result:\n  $$x^2$$"), "- result:\n  x²")
})
