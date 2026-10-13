import assert from "node:assert/strict"
import test from "node:test"
import { CodeRenderable, MarkdownRenderable, SyntaxStyle, TextRenderable, getLinkId } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { prepareTranscriptMarkdown } from "./transcript-rich-markdown.js"
import { applyTranscriptTextContent } from "./transcript-text-render.js"
import type { TranscriptEntry } from "./cli-types.js"
import { createTranscriptMarkdownNodeRenderer, linkifyTranscriptChunks } from "./transcript-hyperlinks.js"

test("MP-08/MP-10/MP-11 code-fence URLs retain native links while the language streams in", async () => {
  const harness = await createTestRenderer({width:80,height:5,useThread:false})
  const syntax = SyntaxStyle.fromStyles({})
  const node = new MarkdownRenderable(harness.renderer,{
    content:"```t\nhttps://github.com/charioxai/chariox",syntaxStyle:syntax,streaming:true,
    renderNode:createTranscriptMarkdownNodeRenderer(harness.renderer),
  })
  harness.renderer.root.add(node)
  const waitForLink = async () => {
    for (let i=0;i<100;i++) {
      await harness.renderOnce()
      if ([...harness.renderer.currentRenderBuffer.buffers.attributes].some(value=>getLinkId(value)>0)) return
      await new Promise(resolve=>setTimeout(resolve,50))
    }
    assert.fail("streamed code fence lost native link metadata")
  }
  try {
    await harness.renderOnce()
    node.content="```text\nhttps://github.com/charioxai/chariox\n```"
    await waitForLink()
    assert.ok(harness.captureCharFrame().includes("https://github.com/charioxai/chariox"))
    assert.ok([...harness.renderer.currentRenderBuffer.buffers.attributes].some(value=>getLinkId(value)>0))
    node.content="```text\nUPDATED https://github.com/charioxai/chariox\n```"
    await waitForLink()
    assert.ok(harness.captureCharFrame().includes("UPDATED https://github.com/charioxai/chariox"))
    assert.ok([...harness.renderer.currentRenderBuffer.buffers.attributes].some(value=>getLinkId(value)>0))
  } finally { harness.renderer.destroy(); syntax.destroy() }
})

test("MP-08/MP-10/MP-11 Markdown bare URLs spanning styled chunks receive one exact native target", () => {
  const url = "https://github.com/charioxai/chariox"
  const input = [{ __isChunk: true as const, text: "Read https://github.com/", attributes: 1 }, { __isChunk: true as const, text: "charioxai/chariox, then reply." }]
  const output = linkifyTranscriptChunks(input)
  assert.equal(output.map(chunk => chunk.text).join(""), input.map(chunk => chunk.text).join(""))
  assert.deepEqual(output.filter(chunk => chunk.link).map(chunk => chunk.link!.url), [url, url])
  assert.equal(output[1]!.attributes, 1)
  assert.ok(!linkifyTranscriptChunks([{__isChunk:true,text:"https://user:fake@example.org/"}]).some(chunk => chunk.link))
})

test("MP-08/MP-10/MP-11 transcript links keep exact targets and clean text, including trailing query punctuation", async () => {
  const harness = await createTestRenderer({width:80,height:8,useThread:false})
  const text = new TextRenderable(harness.renderer,{wrapMode:"char"})
  harness.renderer.root.add(text)
  try {
    const urls = ["https://github.com/charioxai/chariox", "https://example.org/authorize?state=abc!", "https://example.org/" + "x".repeat(150)]
    const content = `Read ${urls[0]}, then reply.\n` + urls.join("\n") + "\nhttps://user:fake@example.org/ file:///tmp/example"
    applyTranscriptTextContent(text,{id:1,role:"notice",text:content} as TranscriptEntry)
    await harness.renderOnce()
    const chunks = text.textNode.toChunks()
    assert.equal(chunks.map(value => value.text).join(""),content)
    assert.deepEqual(chunks.filter(value => value.link).map(value => value.link!.url),[urls[0],...urls])
    assert.ok(!chunks.some(value => value.link?.url.includes("fake")))
  } finally { harness.renderer.destroy() }
})


test("MP-08/MP-10/MP-11 visible URLs stay clickable while syntax highlighting is pending", async () => {
  const harness = await createTestRenderer({width:80,height:4,useThread:false})
  const syntax = SyntaxStyle.fromStyles({})
  const url = "https://github.com/charioxai/chariox"
  let settle!: (result:{highlights:[]}) => void
  const pending = new Promise<{highlights:[]}>(resolve => {settle = resolve})
  const worker = {highlightOnce: () => pending} as unknown as NonNullable<ConstructorParameters<typeof CodeRenderable>[1]["treeSitterClient"]>
  const node = new CodeRenderable(harness.renderer, {
    content:"NATIVE COPY READY\n"+url,filetype:"markdown",syntaxStyle:syntax,
    streaming:true,drawUnstyledText:true,treeSitterClient:worker,
  })
  const renderNode = createTranscriptMarkdownNodeRenderer(harness.renderer)
  renderNode({type:"paragraph",raw:node.content,text:node.content} as Parameters<typeof renderNode>[0], {
    syntaxStyle:syntax,conceal:false,concealCode:false,treeSitterClient:worker,defaultRender:()=>node,
  })
  harness.renderer.root.add(node)
  try {
    for (const width of [80,81,80]) {
      harness.resize(width,4)
      await harness.renderOnce()
      const native = harness.renderer as unknown as {rendererPtr:number,lib:{getCurrentBuffer(ptr:number):{buffers:{char:Uint32Array,attributes:Uint32Array}},linkGetUrl(id:number):string}}
      const {char,attributes} = native.lib.getCurrentBuffer(native.rendererPtr).buffers
      const index = [...char].map(value=>String.fromCodePoint(value)).join("").indexOf(url)
      assert.ok(index>=0,"repository URL is visible before the worker finishes")
      const id = getLinkId(attributes[index]!)
      assert.ok(id>0,"a visible fallback URL must already carry native metadata")
      assert.equal(native.lib.linkGetUrl(id),url)
    }
  } finally { settle({highlights:[]}); await pending; harness.renderer.destroy(); syntax.destroy() }
})


test("MP-08 already painted fallback URLs become linked while the syntax worker remains pending", async () => {
  const harness = await createTestRenderer({width:80,height:4,useThread:false})
  const syntax = SyntaxStyle.fromStyles({})
  const url = "https://github.com/serde-rs/json"
  let settle!: (value:{highlights:[]}) => void
  const pending = new Promise<{highlights:[]}>(resolve => { settle = resolve })
  const worker = {highlightOnce:()=>pending} as unknown as NonNullable<ConstructorParameters<typeof CodeRenderable>[1]["treeSitterClient"]>
  const node = new CodeRenderable(harness.renderer,{content:url,filetype:"markdown",streaming:true,drawUnstyledText:true,syntaxStyle:syntax,treeSitterClient:worker})
  harness.renderer.root.add(node)
  try {
    await harness.renderOnce()
    assert.equal(node.isHighlighting,true)
    const renderNode = createTranscriptMarkdownNodeRenderer(harness.renderer)
    renderNode({type:"paragraph",raw:url,text:url} as Parameters<typeof renderNode>[0],{syntaxStyle:syntax,conceal:false,concealCode:false,treeSitterClient:worker,defaultRender:()=>node})
    await harness.renderOnce()
    const native = harness.renderer as unknown as {rendererPtr:number,lib:{getCurrentBuffer(ptr:number):{buffers:{char:Uint32Array,attributes:Uint32Array}},linkGetUrl(id:number):string}}
    const {char,attributes} = native.lib.getCurrentBuffer(native.rendererPtr).buffers
    const index = [...char].map(value=>String.fromCodePoint(value)).join("").indexOf(url)
    assert.ok(index>=0)
    const id = getLinkId(attributes[index]!)
    assert.ok(id>0,"an already-visible fallback URL must become clickable without waiting for syntax")
    assert.equal(native.lib.linkGetUrl(id),url)
  } finally { settle({highlights:[]});await pending;harness.renderer.destroy();syntax.destroy() }
})

test("MP-08/MP-10/MP-11 sentence-ending punctuation stays visible outside the native URL", () => {
  for (const punctuation of [".", ",", ";", ":"]) {
    const url = "https://github.com/charioxai/chariox"
    const prose = `Read ${url}${punctuation}`
    const linked = linkifyTranscriptChunks([{ __isChunk: true, text: prose }])
    assert.equal(linked.map(chunk => chunk.text).join(""), prose)
    assert.deepEqual(linked.filter(chunk => chunk.link).map(chunk => chunk.link!.url), [url])
    assert.equal(linked.at(-1)!.text, punctuation)
    assert.equal(linked.at(-1)!.link, undefined)
    const bare = linkifyTranscriptChunks([{ __isChunk: true, text: `${url}${punctuation}` }])
    assert.equal(bare[0]!.link!.url, `${url}${punctuation}`, "a standalone URL retains exact bytes")
  }
})

test("MP-08 / MP-11 Markdown targets obey URL admission and cannot spoof visible URLs", () => {
  for (const url of ["file:///etc/passwd", "javascript:alert(1)", "https://user:pass@example.org", "https://example.org/\u001b[31m"]) {
    const chunks = linkifyTranscriptChunks([{__isChunk:true, text:"docs", link:{url}}])
    assert.equal(chunks[0]!.link, undefined)
  }
  const spoof = linkifyTranscriptChunks([{__isChunk:true, text:"https://github.com/charioxai/chariox", link:{url:"https://attacker.example"}}])
  assert.equal(spoof[0]!.link?.url, "https://github.com/charioxai/chariox")
  const safe = linkifyTranscriptChunks([{__isChunk:true, text:"docs", link:{url:"https://developer.mozilla.org"}}])
  assert.equal(safe[0]!.link?.url, "https://developer.mozilla.org")
})

// MP-08 / MP-10: assert the displayed frame, not escaped preprocessor bytes.
test("rich math and image placeholders conceal escapes while code keeps exact bytes", async () => {
  const harness = await createTestRenderer({width:100,height:14,useThread:false})
  const syntax = SyntaxStyle.fromStyles({})
  const node = new MarkdownRenderable(harness.renderer, {
    content: prepareTranscriptMarkdown("$0 < x < 1$ · $|x|$ · $a * b$ · $x_{\\text{best}}$\n\n![plot](./out/plot[1].png)\n\n`literal \\* \\_`"),
    syntaxStyle:syntax,conceal:true,streaming:true,
    renderNode:createTranscriptMarkdownNodeRenderer(harness.renderer),
  })
  harness.renderer.root.add(node)
  try {
    let captured = ""
    for (let i=0;i<100;i++) {
      await harness.renderOnce()
      captured = harness.captureCharFrame()
      if (captured.includes("inline; open") || captured.includes("literal \\*")) break
      await new Promise(resolve=>setTimeout(resolve,20))
    }
    assert.match(captured, /0 < x < 1 · \|x\| · a \* b · x_\(best\)/)
    assert.ok(captured.includes("./out/plot[1].png"),captured)
    assert.ok(captured.includes("literal \\* \\_"),"code span must remain literal")
  } finally {harness.renderer.destroy();syntax.destroy()}
})
