import AppKit

enum InputPath: String { case axPress, axScrollValue, windowEvent, pidText }

func inputPath(_ operation: Operation, role: String) throws -> InputPath {
    switch operation {
    case .click where role == kAXButtonRole: return .axPress
    case .scroll where role == kAXScrollAreaRole: return .axScrollValue
    case .click where [kAXTextFieldRole, kAXTextAreaRole].contains(role): return .windowEvent
    case .text where [kAXTextFieldRole, kAXTextAreaRole].contains(role): return .pidText
    case .click, .scroll, .text: throw Refusal.target
    default: throw Refusal.arguments
    }
}

func windowClickEvents(window: UInt32, location: CGPoint, windowBounds: CGRect,
                       eventNumber: Int) throws -> [CGEvent] {
    guard window > 0, eventNumber > 0, location.x.isFinite, location.y.isFinite,
          windowBounds.contains(location) else { throw Refusal.target }
    // NSEvent supplies AppKit's window number. CGEvent's public pointer-window
    // fields alone leave NSEvent(cgEvent:).windowNumber at zero.
    let local = CGPoint(x: location.x - windowBounds.minX, y: windowBounds.maxY - location.y)
    return try [NSEvent.EventType.leftMouseDown, .leftMouseUp].map { type in
        guard let event = NSEvent.mouseEvent(with: type, location: local, modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: Int(window), context: nil,
            eventNumber: eventNumber, clickCount: 1, pressure: type == .leftMouseDown ? 1 : 0)?.cgEvent
        else { throw Refusal.native }
        event.location = location
        event.setIntegerValueField(.mouseEventWindowUnderMousePointer, value: Int64(window))
        event.setIntegerValueField(.mouseEventWindowUnderMousePointerThatCanHandleThisEvent, value: Int64(window))
        event.flags = []
        return event
    }
}

func nextScrollValue(_ value: Double) throws -> Double {
    guard value.isFinite, (0...1).contains(value), value < 1 else { throw Refusal.target }
    // A single bounded step toward the bottom, independent of natural scrolling.
    return min(1, value + 0.05)
}

func inputReceipt(path: String, observed: String? = nil) -> String {
    "dispatched; path=\(path); " + (observed ?? "application completion unproven")
}
