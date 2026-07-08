import Foundation

@MainActor
extension ArrobaAppModel {
  func perform(_ label: String, operation: () async throws -> String) async {
    saveDraftConfiguration()
    connectionState = .working(label)
    do {
      statusMessage = try await operation()
      connectionState = .connected
    } catch {
      connectionState = .failed
      statusMessage = Self.describe(error)
    }
  }

  func endpointURL() throws -> URL {
    guard let url = URL(string: kernelURLText.trimmingCharacters(in: .whitespacesAndNewlines)),
      url.scheme == "ws" || url.scheme == "wss"
    else {
      throw KernelClientError.invalidEndpoint
    }
    return url
  }

  static func describe(_ error: Error) -> String {
    if let transportError = error as? KernelTransportError {
      return "\(transportError.code): \(transportError.message)"
    }
    if let clientError = error as? KernelClientError {
      switch clientError {
      case .invalidEndpoint:
        return "Kernel URL must start with ws:// or wss://."
      case .invalidRequestEncoding:
        return "Could not encode kernel request."
      case .unsupportedMessage:
        return "Kernel returned an unsupported WebSocket message."
      case .emptyResponse:
        return "Kernel response was empty."
      case .unexpectedResponse(let name):
        return "Unexpected kernel response: \(name)."
      }
    }
    return error.localizedDescription
  }

  func startEventStream(
    sessionID: String,
    attachmentID: String,
    endpoint: URL,
    resumeFromEventID: Int64?
  ) {
    stopEventStream(resetCursor: false)
    eventStreamStartedAt = Date()
    eventStreamState = .connecting
    startHeartbeatMonitor()
    eventTask = Task { @MainActor [weak self] in
      guard let self else { return }
      var nextResumeFromEventID = resumeFromEventID
      while !Task.isCancelled {
        eventStreamState = .connecting
        let currentStream = client.events(
          sessionID: sessionID,
          attachmentID: attachmentID,
          endpoint: endpoint,
          resumeFromEventID: nextResumeFromEventID
        )
        do {
          for try await frame in currentStream {
            handle(eventFrame: frame)
            nextResumeFromEventID = lastEventID
          }
          if !Task.isCancelled {
            eventStreamState = .disconnected
          }
          return
        } catch is CancellationError {
          return
        } catch {
          eventStreamState = .disconnected
          if case .working = connectionState {
            // Keep the active command's outcome visible; the stream loop will keep retrying.
          } else {
            statusMessage = "Event stream interrupted. Reconnecting with replay cursor."
          }
          try? await Task.sleep(for: .seconds(1))
          nextResumeFromEventID = lastEventID
        }
      }
    }
  }

  func evaluateHeartbeatStaleness(now: Date = Date()) {
    guard heartbeatStaleAfterSeconds > 0,
      activeAttachment != nil,
      subscribedSessionID != nil,
      eventStreamState == .live || eventStreamState == .connecting
    else {
      return
    }
    guard let referenceDate = lastHeartbeatAt ?? eventStreamStartedAt else {
      return
    }
    let elapsed = now.timeIntervalSince(referenceDate)
    guard elapsed >= heartbeatStaleAfterSeconds else {
      return
    }
    eventStreamState = .stale
    statusMessage =
      "No kernel heartbeat for \(Int(elapsed.rounded()))s. Waiting for stream recovery."
  }

  func startHeartbeatMonitor() {
    heartbeatMonitorTask?.cancel()
    let interval = max(0.25, min(heartbeatStaleAfterSeconds / 3, 5))
    let sleepNanoseconds = UInt64(interval * 1_000_000_000)
    heartbeatMonitorTask = Task { @MainActor [weak self] in
      while !Task.isCancelled {
        try? await Task.sleep(nanoseconds: sleepNanoseconds)
        guard !Task.isCancelled else { return }
        self?.evaluateHeartbeatStaleness()
      }
    }
  }

  func stopEventStream(resetCursor: Bool) {
    eventTask?.cancel()
    eventTask = nil
    heartbeatMonitorTask?.cancel()
    heartbeatMonitorTask = nil
    eventStreamStartedAt = nil
    if resetCursor {
      lastEventID = nil
      lastHeartbeatAt = nil
    }
  }

  func handle(eventFrame: KernelEventFrame) {
    lastEventID = eventFrame.eventID
    switch eventFrame.event {
    case .terminalOutput(let records):
      appendTerminalOutput(records)
    case .runtimeNotices(let notices):
      for notice in notices {
        appendTranscript(kind: .notice, agentID: nil, text: notice.message)
      }
    case .assistantMessageCompleted(_, _, let agentID, let messageID, _):
      appendTranscript(
        kind: .completion,
        agentID: agentID,
        text: "assistant message completed: \(messageID)"
      )
    case .sessionSnapshot(let session):
      upsert(session)
      selectedSessionID = session.id
      eventStreamState = .live
      statusMessage = "Session snapshot received."
    case .heartbeat(let sessionID):
      if sessionID == subscribedSessionID {
        lastHeartbeatAt = Date()
        eventStreamState = .live
      }
    case .sessionUnavailable(let sessionID, let message):
      if selectedSessionID == sessionID {
        eventStreamState = .failed
        statusMessage = message
      }
    case .transportResumed(let sessionID, _):
      if sessionID == subscribedSessionID {
        eventStreamState = .live
        lastHeartbeatAt = Date()
        statusMessage = "Kernel event stream resumed."
      }
    case .replayGap(let gap):
      eventStreamState = .failed
      statusMessage = gap.message ?? "Replay gap detected. Refresh sessions before continuing."
    case .unknown(let name):
      statusMessage = "Kernel event received: \(name)."
    }
  }

  func appendTerminalOutput(_ records: [TerminalOutputRecord]) {
    var pendingKind: TranscriptEntry.Kind?
    var pendingAgentID: String?
    var pendingText = ""

    func flushPending() {
      guard let kind = pendingKind, !pendingText.isEmpty else { return }
      appendTranscript(kind: kind, agentID: pendingAgentID, text: pendingText)
      pendingKind = nil
      pendingAgentID = nil
      pendingText = ""
    }

    for record in records where !record.text.isEmpty {
      let kind = TranscriptEntry.Kind(terminalKind: record.kind)
      if pendingKind == kind, pendingAgentID == record.agentID {
        pendingText += record.text
      } else {
        flushPending()
        pendingKind = kind
        pendingAgentID = record.agentID
        pendingText = record.text
      }
    }
    flushPending()
  }

  func appendTranscript(kind: TranscriptEntry.Kind, agentID: String?, text: String) {
    guard !text.isEmpty else { return }
    if kind.mergesAdjacentOutput,
      let lastIndex = transcriptEntries.indices.last,
      transcriptEntries[lastIndex].kind == kind,
      transcriptEntries[lastIndex].agentID == agentID
    {
      transcriptEntries[lastIndex].text += text
      return
    }
    transcriptEntries.append(
      TranscriptEntry(kind: kind, agentID: agentID, text: text)
    )
    if transcriptEntries.count > 500 {
      transcriptEntries.removeFirst(transcriptEntries.count - 500)
    }
  }

  func appendCommandNotice(_ message: String) {
    statusMessage = message
    appendTranscript(kind: .notice, agentID: nil, text: message)
  }

  func clearCommandDraftOnSuccess() {
    if connectionState == .connected || connectionState == .idle {
      promptDraft = ""
    }
  }
}
