import AppKit

@main struct StreamTests {
    static func main() throws {
        let inbox = NativeFrameInbox()
        let pixel = CVPixelBufferCreateEmpty()
        inbox.offer(pixel, complete: true, timestamp: 1)
        inbox.offer(pixel, complete: true, timestamp: 2)
        precondition(inbox.take(after: 1)?.timestamp == 2)
        precondition(inbox.take(after: 1) == nil, "one latest pending sample")
        inbox.offer(pixel, complete: true, timestamp: 3)
        inbox.offer(pixel, complete: false, timestamp: 4)
        precondition(inbox.take(after: 0) == nil, "incomplete callback flushes older pixels")
        inbox.offer(pixel, complete: true, timestamp: 5)
        inbox.close()
        inbox.offer(pixel, complete: true, timestamp: 6)
        precondition(inbox.take(after: 0) == nil, "closed callback cannot restore pixels")
        print("PASS bounded latest complete SCK sample, superseded sample drop, incomplete flush and closed callback fence")
    }
    static func CVPixelBufferCreateEmpty() -> CVPixelBuffer {
        var buffer: CVPixelBuffer?
        precondition(CVPixelBufferCreate(nil, 2, 2, kCVPixelFormatType_32BGRA, nil, &buffer) == kCVReturnSuccess)
        return buffer!
    }
}
