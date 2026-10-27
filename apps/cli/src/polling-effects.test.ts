import test from "node:test"
import assert from "node:assert/strict"

import { evaluateConnectionHealth, runPollingLoop } from "./polling-effects.js"

test("evaluateConnectionHealth resets when detached or idle", () => {
  assert.deepEqual(
    evaluateConnectionHealth({
      attached: false,
      working: true,
      now: 1000,
      lastDaemonActivityAt: 0,
      consecutiveSilentPolls: 4,
      silentThreshold: 8,
      silenceWindowMs: 2000,
    }),
    {
      nextConsecutiveSilentPolls: 0,
      shouldRecover: false,
      timeSinceLastActivityMs: 0,
    },
  )
})

test("evaluateConnectionHealth triggers recovery after enough silent polls", () => {
  assert.deepEqual(
    evaluateConnectionHealth({
      attached: true,
      working: true,
      now: 5000,
      lastDaemonActivityAt: 0,
      consecutiveSilentPolls: 7,
      silentThreshold: 8,
      silenceWindowMs: 2000,
    }),
    {
      nextConsecutiveSilentPolls: 8,
      shouldRecover: true,
      timeSinceLastActivityMs: 5000,
    },
  )
})

test("runPollingLoop retries transient failures and then recovers", async () => {
  const events: string[] = []
  let attempt = 0
  let closing = false

  await runPollingLoop({
    operation: "polling test",
    intervalMs: 10,
    isClosing: () => closing,
    task: async () => {
      attempt += 1
      if (attempt === 1) {
        throw new Error("temporary transport failure")
      }
      closing = true
      events.push("task")
    },
    onSessionUnavailable: () => events.push("session-unavailable"),
    onMarkRecovered: (_operation, failures) => events.push(`recovered:${failures}`),
    onMarkDegraded: (_operation, message) => events.push(`degraded:${message}`),
    onFatalError: () => events.push("fatal"),
    formatError: (error) => (error instanceof Error ? error.message : String(error)),
    isSessionUnavailableError: () => false,
    getPollRecoveryDecision: () => ({
      retry: true,
      delayMs: 1,
      message: "retrying",
    }),
    sleep: async () => undefined,
    logger: null,
  })

  assert.deepEqual(events, ["degraded:retrying", "task", "recovered:1"])
})

test("runPollingLoop routes session-unavailable errors separately", async () => {
  const events: string[] = []
  let step = 0
  let closing = false

  await runPollingLoop({
    operation: "polling test",
    intervalMs: 10,
    isClosing: () => closing,
    task: async () => {
      step += 1
      if (step === 1) {
        throw new Error("session not found")
      }
      closing = true
    },
    onSessionUnavailable: () => events.push("session-unavailable"),
    onMarkRecovered: () => events.push("recovered"),
    onMarkDegraded: () => events.push("degraded"),
    onFatalError: () => events.push("fatal"),
    formatError: (error) => (error instanceof Error ? error.message : String(error)),
    isSessionUnavailableError: (error) =>
      error instanceof Error && /session not found/i.test(error.message),
    getPollRecoveryDecision: () => ({
      retry: false,
      delayMs: 0,
      message: "fatal",
    }),
    sleep: async () => undefined,
    logger: null,
  })

  assert.deepEqual(events, ["session-unavailable", "recovered"])
})
