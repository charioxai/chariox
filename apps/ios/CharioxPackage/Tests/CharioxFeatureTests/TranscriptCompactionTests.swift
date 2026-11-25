import Foundation
import Testing
@testable import CharioxFeature

@MainActor
@Test func compactionUpdatesReplaceOneTranscriptRowAndPresentProductText() throws {
    let suite = "TranscriptCompactionTests.\(UUID())"
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let model = CharioxAppModel(client: MockKernelClient(response: .unknown("fixture")), defaults: defaults)
    func record(_ text: String, key: String = "run:compaction:0") -> TerminalOutputRecord {
        TerminalOutputRecord(agentID: "agent-1", kind: "context_compacted", mergeKey: key, bytes: Array(text.utf8))
    }

    model.appendTerminalOutput([record(#"{"state":"running","started_at_ms":1000}"#)])
    let running = try #require(model.transcriptEntries.last)
    #expect(running.kind.rawValue == "compaction")
    #expect(running.compaction?.title == "Compacting context…")
    #expect(running.compaction?.startedAt == Date(timeIntervalSince1970: 1))

    model.appendTerminalOutput([
        record(#"{"state":"running","started_at_ms":2000}"#, key: "run:compaction:1"),
        record(#"{"state":"done","summary":"Saved context"}"#),
        record(#"{"state":"failed","reason":"The turn was cancelled."}"#, key: "run:compaction:1"),
    ])
    #expect(model.transcriptEntries.count == 2)
    #expect(model.transcriptEntries.first?.id == running.id)
    #expect(model.transcriptEntries.map { $0.compaction?.title } == ["Context compacted", "Context compaction failed"])
    #expect(model.transcriptEntries.map { $0.compaction?.detail } == ["Saved context", "The turn was cancelled."])

    model.appendTerminalOutput([record(#"{"state":"done","summary":""}"#, key: "run:compaction:2")])
    #expect(model.transcriptEntries.last?.compaction?.detail == "Summary unavailable")
}
