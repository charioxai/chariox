import AppKit

@main struct M3ReleaseTests {
    static func main() throws {
        for (down, up, button) in [(CGEventType.rightMouseDown, CGEventType.rightMouseUp, CGMouseButton.right),
                                   (.otherMouseDown, .otherMouseUp, .center)] {
            let events = try [down, up].map { type -> CGEvent in
                guard let event = CGEvent(mouseEventSource: nil, mouseType: type,
                    mouseCursorPosition: CGPoint(x: 10, y: 10), mouseButton: button) else { throw Refusal.native }
                return event
            }
            var posted: [CGEventType] = []
            do {
                try dispatchInputEvents(events, fence: { event in
                    if event.type == up { throw Refusal.target }
                }, releaseFence: { _ in throw Refusal.permission }, releaseAllowed: { true }, post: { posted.append($0.type) })
                preconditionFailure("failed owned release accepted")
            } catch {
                precondition(refusalKind(error) == .ownedInput, "right/middle press did not require owner reset")
            }
            precondition(posted == [down], "failed release posted unverified input")
        }
        print("PASS right/middle cancellation requires owner reset on unconfirmed release")
    }
}
