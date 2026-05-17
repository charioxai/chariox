export type TranscriptParserRegistrationDeps<TParser> = {
  parsers: readonly TParser[]
  addDefaultParsers: (parsers: readonly TParser[]) => void
}

export function createTranscriptParserRegistration<TParser>(deps: TranscriptParserRegistrationDeps<TParser>) {
  let registered = false

  return {
    ensureRegistered() {
      if (registered) {
        return false
      }
      deps.addDefaultParsers(deps.parsers)
      registered = true
      return true
    },
  }
}
