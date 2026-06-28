import assert from "node:assert/strict"
import test from "node:test"

import {
  getProviderActivityLabel,
  isProviderIdleStatus,
  normalizeProviderActivityLabel,
  toProviderPresentParticiplePhrase,
} from "./provider-status.js"

test("provider status helpers map OpenCode status text to activity labels", () => {
  assert.equal(getProviderActivityLabel("OpenCode is idle."), null)
  assert.equal(getProviderActivityLabel("OpenCode is thinking..."), "thinking")
  assert.equal(getProviderActivityLabel("OpenCode status: reconnecting"), "reconnecting")
  assert.equal(getProviderActivityLabel("OpenCode status: retry_auth"), "retry authing")
  assert.equal(getProviderActivityLabel("OpenCode status: compile"), "compiling")
  assert.equal(getProviderActivityLabel("OpenCode is writing."), "writing")
  assert.equal(getProviderActivityLabel("OpenCode is compile."), "compile")
  assert.equal(getProviderActivityLabel("unrecognized"), null)
})

test("provider status helpers identify idle status without treating activity as idle", () => {
  assert.equal(isProviderIdleStatus("OpenCode is idle."), true)
  assert.equal(isProviderIdleStatus("OpenCode is idle"), true)
  assert.equal(isProviderIdleStatus("OpenCode is thinking..."), false)
})

test("provider status helpers normalize activity label vocabulary", () => {
  assert.equal(normalizeProviderActivityLabel(" Writing "), "writing")
  assert.equal(normalizeProviderActivityLabel(""), null)
  assert.equal(toProviderPresentParticiplePhrase("run"), "running")
  assert.equal(toProviderPresentParticiplePhrase("die"), "dying")
  assert.equal(toProviderPresentParticiplePhrase("compile"), "compiling")
})
