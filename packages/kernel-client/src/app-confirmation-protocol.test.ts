// APP-CONFIRM-01: the client reads a person-confirmed action's card and catalog entry at the current pair, from the
// fixture the kernel pins (`protocol_shapes/app_confirmation.rs`).
import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { LOCAL_DAEMON_PROTOCOL_VERSION, type AppViewAction, type RuntimeInteraction } from "./kernel-types.js"
import { formatAppViewInspection } from "./shell-app-command.js"

type Fixture = { local: number; interaction: RuntimeInteraction; action: AppViewAction }
const fixture = async () => JSON.parse(await readFile(new URL("../../../fixtures/app-confirmation-protocol.json", import.meta.url), "utf8")) as Fixture

test("the client reads the kernel's App and publisher frame and a person_confirmed catalog action", async () => {
  const shapes = await fixture()
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, shapes.local)
  assert.deepEqual(shapes.interaction.app_confirmation, { installation_id: "campaign", app_id: "com.chariox.campaign", app_name: "Campaign", publisher_id: "com.chariox", publisher_name: "Chariox", action_name: "pay" })
  assert.equal(shapes.action.effect, "person_confirmed")
  const catalog = { actions: [shapes.action], registrations: [], state: { ok: true, value: {} }, warnings: [] }
  const listed = formatAppViewInspection({ installation_id: "campaign", target_id: "tab-1", generation: "1", development: false, catalog_json: JSON.stringify(catalog) })
  assert.match(listed, /pay · person_confirmed · Pay for the campaign/)
  assert.match(listed, /confirms with the person using: review_payment/)
})
