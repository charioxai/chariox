import assert from "node:assert/strict"
import test from "node:test"

import {
  getProviderActivityLabel,
  getToolActivityLabel,
  isProviderIdleStatus,
  normalizeProviderActivityLabel,
  shouldRenderProviderStatus,
  chooseVisibleActivityLabel,
  deriveVisibleActivityLabel,
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
  assert.equal(shouldRenderProviderStatus("OpenCode is idle."), false)
  assert.equal(shouldRenderProviderStatus("OpenCode is thinking..."), false)
  assert.equal(shouldRenderProviderStatus("OpenCode status: reconnecting"), true)
})

test("provider status helpers normalize activity label vocabulary", () => {
  assert.equal(normalizeProviderActivityLabel(" Writing "), "writing")
  assert.equal(normalizeProviderActivityLabel(""), null)
  assert.equal(toProviderPresentParticiplePhrase("run"), "running")
  assert.equal(toProviderPresentParticiplePhrase("die"), "dying")
  assert.equal(toProviderPresentParticiplePhrase("compile"), "compiling")
})

test("provider status helpers derive tool activity labels", () => {
  assert.equal(getToolActivityLabel("bash"), "bashing")
  assert.equal(getToolActivityLabel("grep"), "grepping")
  assert.equal(getToolActivityLabel("glob"), "globbing")
  assert.equal(getToolActivityLabel("read"), "reading")
  assert.equal(getToolActivityLabel("apply_patch"), "patching")
  assert.equal(getToolActivityLabel("webfetch"), "webfetching")
  assert.equal(getToolActivityLabel("todowrite"), "todowriting")
})

test("provider status helpers prefer visible tool activity over provider activity", () => {
  assert.equal(chooseVisibleActivityLabel("reading", "grepping"), "grepping")
  assert.equal(chooseVisibleActivityLabel("reconnecting", null), "reconnecting")
  assert.equal(chooseVisibleActivityLabel(null, "writing"), "writing")
  assert.equal(chooseVisibleActivityLabel(null, null), null)
})

test("provider status helpers derive visible activity from active tool labels", () => {
  assert.equal(
    deriveVisibleActivityLabel({
      providerActivityLabel: "thinking",
      activeToolLabels: ["reading", "patching"],
    }),
    "patching",
  )
  assert.equal(
    deriveVisibleActivityLabel({
      providerActivityLabel: "reconnecting",
      activeToolLabels: [],
    }),
    "reconnecting",
  )
  assert.equal(
    deriveVisibleActivityLabel({
      providerActivityLabel: null,
      activeToolLabels: [],
    }),
    null,
  )
})
