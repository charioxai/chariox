export function listPromptSettingsRequest() {
  return { ListPromptSettings: null }
}

export function resetPromptSettingRequest(
  id: string,
  expectedRevision: number,
  expectedSha256: string,
) {
  return {
    ResetPromptSetting: {
      id,
      expected_revision: expectedRevision,
      expected_sha256: expectedSha256,
    },
  }
}

export function resetAllPromptSettingsRequest(
  expected: Record<string, { revision: number; sha256: string }>,
) {
  return { ResetAllPromptSettings: { expected } }
}
