import AppKit
import ScreenCaptureKit
import CoreImage

// Callback-owned raw pixels never leave this one-element mailbox. The consumer
// takes a strong immutable buffer reference; SCK may then offer the next sample.
final class NativeFrameInbox: @unchecked Sendable {
    struct Sample { let buffer: CVPixelBuffer; let timestamp: TimeInterval }
    private let lock = NSLock()
    private var pending: Sample?
    private var closed = false
    func offer(_ buffer: CVPixelBuffer?, complete: Bool, timestamp: TimeInterval) {
        lock.lock(); defer { lock.unlock() }
        guard !closed else { return }
        guard complete, timestamp.isFinite, let buffer,
              CVPixelBufferGetWidth(buffer) * CVPixelBufferGetHeight(buffer) <= 4 * 1024 * 1024 else {
            pending = nil; return
        }
        pending = Sample(buffer: buffer, timestamp: timestamp)
    }
    func take(after: TimeInterval) -> Sample? {
        lock.lock(); defer { lock.unlock() }
        defer { pending = nil }
        guard !closed, let pending, pending.timestamp >= after else { return nil }
        return pending
    }
    var isClosed: Bool { lock.lock(); defer { lock.unlock() }; return closed }
    func flush() { lock.lock(); pending = nil; lock.unlock() }
    func close() { lock.lock(); closed = true; pending = nil; lock.unlock() }
}

final class NativeCaptureStream: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    private let inbox = NativeFrameInbox()
    private let width: Int, height: Int
    private let scale: CGFloat
    private var stream: SCStream?
    init(surface: NativeSurface) { width = surface.width; height = surface.height; scale = surface.scale }
    @MainActor func start(filter: SCContentFilter, configuration: SCStreamConfiguration) async throws {
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: 30)
        configuration.queueDepth = 3
        configuration.pixelFormat = kCVPixelFormatType_32BGRA
        configuration.colorSpaceName = CGColorSpace.sRGB
        let stream = SCStream(filter: filter, configuration: configuration, delegate: self)
        try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: DispatchQueue(label: "ai.chariox.capture"))
        self.stream = stream
        do { try await stream.startCapture() }
        catch { stop(); throw tagged(.native, "capture-stream-start") }
    }
    @MainActor func stop() {
        inbox.close()
        stream?.stopCapture(completionHandler: { _ in })
        stream = nil
    }
    func flush() { inbox.flush() }
    func stream(_ stream: SCStream, didStopWithError error: Error) { inbox.close() }
    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen else { return }
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]]
        let metadata = attachments?.first
        let buffer = CMSampleBufferGetImageBuffer(sampleBuffer)
        // Dirty rects are hints only. Every accepted sample is compared in full
        // after protection; no skipped/incomplete sample can establish an exact base.
        let status = metadata?[.status] as? Int
        let content = (metadata?[.contentRect] as? NSDictionary).flatMap { CGRect(dictionaryRepresentation: $0) }
        let contentScale = metadata?[.contentScale] as? CGFloat
        let pixelScale = metadata?[.scaleFactor] as? CGFloat
        let complete = CMSampleBufferIsValid(sampleBuffer) && CMSampleBufferDataIsReady(sampleBuffer)
            && status == SCFrameStatus.complete.rawValue && contentScale == 1 && pixelScale == scale
            && content?.width == CGFloat(width) / scale && content?.height == CGFloat(height) / scale
            && buffer.map { CVPixelBufferGetWidth($0) == width && CVPixelBufferGetHeight($0) == height } == true
        inbox.offer(buffer, complete: complete, timestamp: CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sampleBuffer)))
    }
    @MainActor func image(after: TimeInterval, authority: () throws -> Void) async throws -> (image: CGImage, timestamp: TimeInterval) {
        // Static SCK sources may have no new callback. Caller can request an
        // independent full screenshot for exact repair instead of reusing raw pixels.
        let deadline = ProcessInfo.processInfo.systemUptime + 0.2
        while ProcessInfo.processInfo.systemUptime < deadline {
            try authority()
            guard !inbox.isClosed else { throw tagged(.native, "capture-stream-ended") }
            if let sample = inbox.take(after: after) {
                let image = CIImage(cvPixelBuffer: sample.buffer)
                let context = CIContext(options: [.cacheIntermediates: false, .useSoftwareRenderer: true])
                guard let result = context.createCGImage(image, from: image.extent, format: .RGBA8,
                                                          colorSpace: CGColorSpace(name: CGColorSpace.sRGB)!) else {
                    throw tagged(.native, "capture-stream-image")
                }
                return (result, sample.timestamp)
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        throw tagged(.native, "capture-stream-static")
    }
}
