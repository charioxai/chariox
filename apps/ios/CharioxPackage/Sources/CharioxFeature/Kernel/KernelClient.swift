import Foundation

public protocol KernelClientProtocol: Sendable {
    func send(_ request: LocalDaemonRequest, to endpoint: URL) async throws -> LocalDaemonResponse
    func events(
        sessionID: String,
        attachmentID: String,
        endpoint: URL,
        resumeFromEventID: Int64?
    ) -> AsyncThrowingStream<KernelEventFrame, Error>
}

public struct KernelClient: KernelClientProtocol {
    private let session: URLSession

    public init(session: URLSession = .shared) {
        self.session = session
    }

    public func send(_ request: LocalDaemonRequest, to endpoint: URL) async throws -> LocalDaemonResponse {
        let task = session.webSocketTask(with: endpoint)
        task.resume()
        defer {
            task.cancel(with: .normalClosure, reason: nil)
        }

        let requestID = UUID().uuidString
        let frame = KernelRequestFrame(requestID: requestID, request: request)
        let requestData = try KernelProtocolCodec.encodeRequestFrame(frame)
        guard let requestText = String(data: requestData, encoding: .utf8) else {
            throw KernelClientError.invalidRequestEncoding
        }

        try await task.send(.string(requestText))

        while true {
            let message = try await task.receive()
            let responseData: Data
            switch message {
            case let .string(text):
                responseData = Data(text.utf8)
            case let .data(data):
                responseData = data
            @unknown default:
                throw KernelClientError.unsupportedMessage
            }

            let responseFrame = try KernelProtocolCodec.decodeResponseFrame(responseData)
            guard responseFrame.requestID == requestID else {
                continue
            }
            if let error = responseFrame.error {
                throw error
            }
            guard let response = responseFrame.response else {
                throw KernelClientError.emptyResponse
            }
            return response
        }
    }

    public func events(
        sessionID: String,
        attachmentID: String,
        endpoint: URL,
        resumeFromEventID: Int64?
    ) -> AsyncThrowingStream<KernelEventFrame, Error> {
        AsyncThrowingStream { continuation in
            let task = session.webSocketTask(with: endpoint)
            task.resume()

            let streamTask = Task {
                let subscribeRequestID = UUID().uuidString
                do {
                    try await sendSubscribe(
                        requestID: subscribeRequestID,
                        sessionID: sessionID,
                        attachmentID: attachmentID,
                        resumeFromEventID: resumeFromEventID,
                        task: task
                    )

                    while !Task.isCancelled {
                        let frame = try KernelProtocolCodec.decodeTransportFrame(
                            try await receiveData(from: task)
                        )
                        switch frame {
                        case let .response(responseFrame):
                            guard responseFrame.requestID == subscribeRequestID else {
                                continue
                            }
                            if let error = responseFrame.error {
                                throw error
                            }
                        case let .event(eventFrame):
                            continuation.yield(eventFrame)
                        }
                    }
                    continuation.finish()
                } catch is CancellationError {
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }

            continuation.onTermination = { @Sendable _ in
                streamTask.cancel()
                Task {
                    await sendUnsubscribe(task: task)
                }
            }
        }
    }

    private func sendSubscribe(
        requestID: String,
        sessionID: String,
        attachmentID: String,
        resumeFromEventID: Int64?,
        task: URLSessionWebSocketTask
    ) async throws {
        let frame = KernelSubscribeFrame(
            requestID: requestID,
            sessionID: sessionID,
            attachmentID: attachmentID,
            resumeFromEventID: resumeFromEventID
        )
        let data = try KernelProtocolCodec.encodeSubscribeFrame(frame)
        guard let text = String(data: data, encoding: .utf8) else {
            throw KernelClientError.invalidRequestEncoding
        }
        try await task.send(.string(text))
    }

    private func sendUnsubscribe(task: URLSessionWebSocketTask) async {
        defer {
            task.cancel(with: .normalClosure, reason: nil)
        }
        let frame = KernelUnsubscribeFrame()
        guard let data = try? KernelProtocolCodec.encodeUnsubscribeFrame(frame),
              let text = String(data: data, encoding: .utf8)
        else {
            return
        }
        try? await task.send(.string(text))
    }

    private func receiveData(from task: URLSessionWebSocketTask) async throws -> Data {
        let message = try await task.receive()
        switch message {
        case let .string(text):
            return Data(text.utf8)
        case let .data(data):
            return data
        @unknown default:
            throw KernelClientError.unsupportedMessage
        }
    }
}

public enum KernelClientError: Error, Equatable, Sendable {
    case invalidEndpoint
    case invalidRequestEncoding
    case unsupportedMessage
    case emptyResponse
    case unexpectedResponse(String)
}
