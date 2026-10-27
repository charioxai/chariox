import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import vm from "node:vm"

import {
  createPairingToken,
  pairCloudMachineDirect,
} from "./live-hosted-cloud-relay-drill-helpers.mjs"

function pairingFetch(calls) {
  return async (url, request) => {
    const body = JSON.parse(request.body)
    calls.push({ url, body, headers: request.headers })
    if (url.endsWith("/pairing-tokens")) {
      assert.deepEqual({ ...request.headers }, {
        "content-type": "application/json",
        authorization: "Bearer synthetic-operator-session",
      })
      assert.deepEqual(body, { accountId: "account-1", createdByUserId: "user-1", subjectKind: "machine" })
      return { ok: true, json: async () => ({ token: "synthetic-pairing-token" }) }
    }
    assert.ok(url.endsWith("/machines/pair"))
    assert.deepEqual(body, {
      accountId: "account-1", userId: "user-1", machineId: "worker-1",
      token: "synthetic-pairing-token", alias: "worker",
    })
    return { ok: true, json: async () => ({ machineId: "worker-1" }) }
  }
}

test("hosted pairing token authenticates the existing operator session", async (t) => {
  const calls = []
  t.mock.method(globalThis, "fetch", pairingFetch(calls))
  const token = await createPairingToken({
    accountId: "account-1", userId: "user-1", subjectKind: "machine",
    cloudSessionToken: "synthetic-operator-session",
  })
  assert.equal(token, "synthetic-pairing-token")
  assert.equal(calls.length, 1)
})

test("hosted direct machine pairing passes its profile operator session to token creation", async (t) => {
  const calls = []
  t.mock.method(globalThis, "fetch", pairingFetch(calls))
  const paired = await pairCloudMachineDirect({
    profile: { accountId: "account-1", userId: "user-1", cloudSessionToken: "synthetic-operator-session" },
    machineId: "worker-1", alias: "worker",
  })
  assert.equal(paired.machineId, "worker-1")
  assert.equal(calls.length, 2)
})

test("hosted pairing callers reject missing operator sessions before fetch", async (t) => {
  for (const cloudSessionToken of [undefined, "", "   "]) {
    await t.test(`session=${JSON.stringify(cloudSessionToken)}`, async (t) => {
      const calls = []
      t.mock.method(globalThis, "fetch", async (...args) => {
        calls.push(args)
        return { ok: true, json: async () => ({ token: "synthetic-pairing-token", machineId: "worker-1" }) }
      })
      await assert.rejects(
        createPairingToken({ accountId: "account-1", userId: "user-1", subjectKind: "machine", cloudSessionToken }),
        /requires an authenticated Cloud session/,
      )
      await assert.rejects(
        pairCloudMachineDirect({
          profile: { accountId: "account-1", userId: "user-1", cloudSessionToken, machineCredential: "synthetic-machine-only-credential" },
          machineId: "worker-1", alias: "worker",
        }),
        /requires an authenticated Cloud session/,
      )
      assert.deepEqual(calls, [])
    })
  }
})

async function remoteOwnerPairMachine(fetch) {
  // Load the actual two HTTP/pairing functions without invoking the live drill's
  // main entry point, which reads a private profile and starts remote processes.
  const source = await readFile(new URL("../live-tui-remote-owner-cloud-hetzner-drill.mjs", import.meta.url), "utf8")
  const start = source.indexOf("async function postJson(")
  const end = source.indexOf("async function issueMachineToken(", start)
  assert.ok(start >= 0 && end > start, "remote owner pairing function boundary changed")
  return vm.runInNewContext(`${source.slice(start, end)}; pairMachine`, { fetch })
}

test("remote owner machine pairing authenticates its snake-case profile operator session", async () => {
  const calls = []
  const pairMachine = await remoteOwnerPairMachine(pairingFetch(calls))
  await pairMachine({
    api_url: "https://cloud.example", account_id: "account-1", user_id: "user-1",
    cloud_session_token: "synthetic-operator-session",
  }, "worker-1", "worker")
  assert.deepEqual(calls.map((call) => call.url), [
    "https://cloud.example/pairing-tokens", "https://cloud.example/machines/pair",
  ])
})

test("remote owner machine pairing refuses missing operator sessions before fetch", async (t) => {
  for (const cloud_session_token of [undefined, "", "   "]) {
    await t.test(`session=${JSON.stringify(cloud_session_token)}`, async () => {
      const calls = []
      const pairMachine = await remoteOwnerPairMachine(async (...args) => {
        calls.push(args)
        return { ok: true, json: async () => ({ token: "synthetic-pairing-token", machineId: "worker-1" }) }
      })
      await assert.rejects(
        pairMachine({
          api_url: "https://cloud.example", account_id: "account-1", user_id: "user-1",
          cloud_session_token, machine_credential: "synthetic-machine-only-credential",
        }, "worker-1", "worker"),
        /requires an authenticated Cloud session/,
      )
      assert.deepEqual(calls, [])
    })
  }
})
