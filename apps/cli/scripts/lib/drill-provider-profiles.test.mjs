import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_PROVIDER_IDS,
  applyProviderAccountAlias,
  applyProviderModelOverride,
  codexCliModel,
  isKnownDrillProvider,
  parseProviderAccountAlias,
  parseProviderList,
  parseProviderModelOverride,
  providerProfileMetadata,
  resolveProviderModel,
} from "./drill-provider-profiles.mjs"

test("defines known drill provider ids", () => {
  assert.deepEqual(DRILL_PROVIDER_IDS, ["claude", "claude-headless", "claude-p", "codex", "dev-stub", "opencode", "opencode-zen"])
  assert.equal(isKnownDrillProvider("codex"), true)
  assert.equal(isKnownDrillProvider("cdoex"), false)
})

test("parses provider lists", () => {
  assert.deepEqual(parseProviderList("codex, opencode,,claude"), ["codex", "opencode", "claude"])
  assert.deepEqual(parseProviderList("dev-stub, claude-headless, opencode-zen"), ["dev-stub", "claude-headless", "opencode-zen"])
  assert.throws(() => parseProviderList(" , "), /at least one provider/)
  assert.throws(() => parseProviderList("codex,cdoex"), /provider list\[1\] has unknown provider "cdoex"/)
})

test("parses provider model overrides", () => {
  assert.deepEqual(parseProviderModelOverride("codex=gpt-5.5"), { provider: "codex", model: "gpt-5.5" })
  assert.deepEqual(applyProviderModelOverride({}, "opencode=opencode/gpt-5.2"), { opencode: "opencode/gpt-5.2" })
  assert.throws(() => parseProviderModelOverride("codex"), /provider=model/)
})

test("parses provider account aliases without accepting secrets", () => {
  assert.deepEqual(parseProviderAccountAlias("codex=work-account"), { provider: "codex", alias: "work-account" })
  assert.deepEqual(applyProviderAccountAlias({}, "opencode=zen_profile"), { opencode: "zen_profile" })
  assert.throws(() => parseProviderAccountAlias("codex"), /provider=alias/)
  assert.throws(() => parseProviderAccountAlias("codex=sk-this-should-not-persist"), /provider account alias/)
  assert.throws(() => parseProviderAccountAlias("codex=user@example.test"), /provider account alias/)
})

test("resolves provider-specific model defaults", () => {
  assert.equal(codexCliModel("gpt-5.2"), "gpt-5.2-codex")
  assert.equal(codexCliModel("gpt-5.2-codex"), "gpt-5.2-codex")
  assert.equal(resolveProviderModel("codex", { defaultModel: "gpt-5.2" }), "gpt-5.2-codex")
  assert.equal(resolveProviderModel("opencode", { defaultModel: "gpt-5.2" }), "opencode/gpt-5.2")
  assert.equal(resolveProviderModel("claude", { defaultModel: "sonnet" }), "sonnet")
  assert.equal(resolveProviderModel("codex", {
    defaultModel: "gpt-5.2",
    providerModels: { codex: "gpt-5.5" },
  }), "gpt-5.5")
})

test("summarizes provider profile metadata without account secrets", () => {
  assert.deepEqual(providerProfileMetadata({
    providers: ["codex", "opencode"],
    defaultModel: "gpt-5.2",
    providerModels: { opencode: "opencode/gpt-5.4", codex: "gpt-5.5" },
    providerAccounts: { opencode: "zen", codex: "work" },
  }), {
    providerCount: 2,
    providers: "codex,opencode",
    defaultModel: "gpt-5.2",
    providerModelOverrides: "codex,opencode",
    providerAccountAliases: "codex=work,opencode=zen",
  })
  assert.throws(() => providerProfileMetadata({
    providers: ["codex", "opencode"],
    defaultModel: "gpt-5.2",
    providerModels: { cdoex: "gpt-5.5" },
  }), /provider model override provider has unknown provider "cdoex"/)
  assert.throws(() => providerProfileMetadata({
    providers: ["codex", "opencode"],
    defaultModel: "gpt-5.2",
    providerAccounts: { cdoex: "work" },
  }), /provider account alias provider has unknown provider "cdoex"/)
})
