import Foundation

struct TranscriptCompaction: Decodable, Equatable, Sendable {
    enum State: String, Decodable, Sendable {
        case running
        case done
        case failed
    }

    let state: State
    let startedAtMs: Int64?
    let summary: String?
    let reason: String?

    private enum CodingKeys: String, CodingKey {
        case state, summary, reason
        case startedAtMs = "started_at_ms"
    }

    var startedAt: Date? {
        startedAtMs.map { Date(timeIntervalSince1970: Double($0) / 1_000) }
    }

    var title: String {
        switch state {
        case .running: "Compacting context…"
        case .done: "Context compacted"
        case .failed: "Context compaction failed"
        }
    }

    var detail: String? {
        switch state {
        case .running: nil
        case .done: nonempty(summary) ?? "Summary unavailable"
        case .failed: nonempty(reason) ?? "Claude gave no reason."
        }
    }

    private func nonempty(_ text: String?) -> String? {
        guard let text = text?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else {
            return nil
        }
        return text
    }
}

extension TranscriptEntry {
    var compaction: TranscriptCompaction? {
        guard kind == .compaction else { return nil }
        return try? JSONDecoder().decode(TranscriptCompaction.self, from: Data(text.utf8))
    }
}
