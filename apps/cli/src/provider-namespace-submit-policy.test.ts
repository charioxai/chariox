import assert from "node:assert/strict"
import test from "node:test"

import { validateProviderNamespaceSubmit } from "./provider-namespace-submit-policy.js"

test("provider namespace submit policy accepts matching focused providers", () => {
  assert.deepEqual(
    validateProviderNamespaceSubmit({
      command: {
        raw: "/opencode compact",
        provider: "opencode",
        forwardedCommand: "/compact",
      },
      focusedProvider: "opencode",
      workflowScreenShowing: false,
      pendingAttachmentCount: 0,
    }),
    {
      ok: true,
      forwardedCommand: "/compact",
    },
  )
})

test("provider namespace submit policy rejects mismatched focused providers", () => {
  assert.deepEqual(
    validateProviderNamespaceSubmit({
      command: {
        raw: "/opencode compact",
        provider: "opencode",
        forwardedCommand: "/compact",
      },
      focusedProvider: "codex",
      workflowScreenShowing: false,
      pendingAttachmentCount: 0,
    }),
    {
      ok: false,
      message: "/opencode is unavailable while the focused agent uses codex",
    },
  )
})

test("provider namespace submit policy requires a focused provider", () => {
  assert.deepEqual(
    validateProviderNamespaceSubmit({
      command: {
        raw: "/codex compact",
        provider: "codex",
        forwardedCommand: "/compact",
      },
      focusedProvider: null,
      workflowScreenShowing: false,
      pendingAttachmentCount: 0,
    }),
    {
      ok: false,
      message: "provider-native commands require a focused OpenCode, Codex, or Claude Code agent",
    },
  )
})

test("provider namespace submit policy rejects empty provider commands", () => {
  assert.deepEqual(
    validateProviderNamespaceSubmit({
      command: {
        raw: "/codex",
        provider: "codex",
        forwardedCommand: "",
      },
      focusedProvider: "codex",
      workflowScreenShowing: false,
      pendingAttachmentCount: 0,
    }),
    {
      ok: false,
      message: "usage: /codex <provider-command>",
    },
  )
})

test("provider namespace submit policy rejects workflow prompt ownership", () => {
  assert.deepEqual(
    validateProviderNamespaceSubmit({
      command: {
        raw: "/opencode compact",
        provider: "opencode",
        forwardedCommand: "/compact",
      },
      focusedProvider: "opencode",
      workflowScreenShowing: true,
      pendingAttachmentCount: 0,
    }),
    {
      ok: false,
      message: "provider-native commands are unavailable while the workflow screen owns the prompt",
    },
  )
})

test("provider namespace submit policy rejects pending attachments", () => {
  assert.deepEqual(
    validateProviderNamespaceSubmit({
      command: {
        raw: "/opencode compact",
        provider: "opencode",
        forwardedCommand: "/compact",
      },
      focusedProvider: "opencode",
      workflowScreenShowing: false,
      pendingAttachmentCount: 1,
    }),
    {
      ok: false,
      message: "provider-native commands do not support attachments",
    },
  )
})
