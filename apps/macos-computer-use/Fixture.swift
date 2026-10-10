import AppKit

final class Fixture: NSObject, NSApplicationDelegate {
    let window = NSWindow(contentRect: NSRect(x: 160, y: 160, width: 540, height: 360),
                          styleMask: [.titled, .closable], backing: .buffered, defer: false)
    let patch = NSView(frame: NSRect(x: 20, y: 20, width: 60, height: 60))
    let secondPatch = NSView(frame: NSRect(x: 460, y: 40, width: 30, height: 30))
    var ticks = 0
    var drillText: NSTextView?
    var lastSelection: NSRange?
    func applicationDidFinishLaunching(_ notification: Notification) {
        window.title = "Chariox public fixture"
        // Bound runner-owned instances; manual owner sessions stay open.
        if CommandLine.arguments.contains("--unattended") {
            Timer.scheduledTimer(withTimeInterval: 20, repeats: false) { _ in NSApp.terminate(nil) }
        }
        if CommandLine.arguments.contains("--input-drill") {
            NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp, .leftMouseDown, .leftMouseUp,
                .rightMouseDown, .rightMouseUp, .otherMouseDown, .otherMouseUp, .scrollWheel]) { event in
                // Public drill oracle logs event class only, never human keys or text.
                print("fixture input event=\(event.type.rawValue)"); fflush(stdout)
                return event
            }
        }
        if CommandLine.arguments.contains("--text-drill") { startTextDrill(); return }
        let ordinary = NSTextField(frame: NSRect(x: 20, y: 295, width: 310, height: 28))
        ordinary.placeholderString = "Public Unicode test text"; ordinary.setAccessibilityIdentifier("ordinary")
        let secure = NSSecureTextField(frame: NSRect(x: 20, y: 250, width: 310, height: 28))
        secure.placeholderString = "Synthetic only, never enter a secret"; secure.setAccessibilityIdentifier("secure")
        let button = NSButton(title: "Clicks: 0", target: self, action: #selector(click(_:)))
        button.frame = NSRect(x: 350, y: 290, width: 160, height: 32); button.setAccessibilityIdentifier("button")
        let stop = NSButton(title: "Stop fixture", target: NSApp, action: #selector(NSApplication.terminate(_:)))
        stop.frame = NSRect(x: 350, y: 245, width: 160, height: 32)
        let scroll = NSScrollView(frame: NSRect(x: 20, y: 100, width: 490, height: 130))
        scroll.hasVerticalScroller = true; scroll.setAccessibilityIdentifier("scroll")
        let rows = NSTextView(frame: NSRect(x: 0, y: 0, width: 465, height: 700))
        rows.isEditable = false; rows.string = (1...35).map { "Public scroll row \($0)" }.joined(separator: "\n")
        scroll.documentView = rows
        patch.wantsLayer = true; secondPatch.wantsLayer = true
        for view in [ordinary, secure, button, stop, scroll, patch, secondPatch] { window.contentView!.addSubview(view) }
        Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [self] _ in
            ticks += 1; patch.frame.origin.x = CGFloat(20 + ticks % 400)
            patch.layer?.backgroundColor = (ticks % 2 == 0 ? NSColor.systemBlue : .systemOrange).cgColor
            secondPatch.frame.origin.x = CGFloat(460 - ticks % 400)
            secondPatch.layer?.backgroundColor = NSColor.systemGreen.cgColor
        }
        window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
        window.makeFirstResponder(ordinary)
        print("fixturePID=\(getpid()) windowID=\(window.windowNumber)"); fflush(stdout)
    }
    func startTextDrill() {
        let scroll = NSScrollView(frame: NSRect(x: 20, y: 20, width: 500, height: 320))
        scroll.hasVerticalScroller = true
        let text = NSTextView(frame: NSRect(x: 0, y: 0, width: 480, height: 1200))
        text.setAccessibilityIdentifier("text")
        text.font = .monospacedSystemFont(ofSize: 16, weight: .regular)
        text.textContainerInset = NSSize(width: 8, height: 8)
        text.isVerticallyResizable = false
        text.string = (1...12).map { String(format: "line %02d public caret probe", $0) }.joined(separator: "\n")
        scroll.documentView = text; window.contentView!.addSubview(scroll); drillText = text
        window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
        window.makeFirstResponder(text); text.setSelectedRange(NSRange(location: 0, length: 0))
        guard let layout = text.layoutManager, let container = text.textContainer else { exit(1) }
        layout.ensureLayout(for: container)
        print("fixturePID=\(getpid()) windowID=\(window.windowNumber)")
        // Known UTF-16 locations in our public document. Each point is inside
        // the left quarter of its glyph, with an independent AppKit insertion
        // lookup before publication. No owner pointer placement is needed.
        for (name, expected) in [("line5-before-r", 125), ("line5-after-r", 126),
                                 ("line1", 3), ("line9", 233)] {
            let glyph = layout.glyphIndexForCharacter(at: expected)
            let rect = layout.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: container)
            let local = NSPoint(x: rect.minX + rect.width * 0.25 + text.textContainerOrigin.x,
                                y: rect.midY + text.textContainerOrigin.y)
            let reference = text.characterIndexForInsertion(at: local)
            guard reference == expected else {
                print("fixture mapping failed probe=\(name) expectedLocation=\(expected) referenceLocation=\(reference)")
                fflush(stdout); NSApp.terminate(nil); return
            }
            let cocoa = window.convertPoint(toScreen: text.convert(local, to: nil))
            // AppKit and AX/CG use screen points. Retina scale applies only to
            // bitmap pixels; it must not enter either side of this conversion.
            let primaryTop = NSScreen.screens[0].frame.maxY
            let point = quartzPoint(fromCocoa: cocoa, primaryTop: primaryTop)
            let roundTrip = text.convert(window.convertPoint(fromScreen:
                NSPoint(x: point.x, y: primaryTop - point.y)), from: nil)
            guard text.characterIndexForInsertion(at: roundTrip) == expected else {
                print("fixture coordinate round-trip failed probe=\(name)")
                fflush(stdout); NSApp.terminate(nil); return
            }
            print("probe=\(name) clickX=\(point.x) clickY=\(point.y) expectedLocation=\(expected) referenceLocation=\(reference)")
        }
        print("fixture geometry cocoaFrame=\(window.frame) textFrame=\(text.frame) primaryTop=\(NSScreen.screens[0].frame.maxY) scale=\(window.backingScaleFactor)")
        print("fixture selection location=0 length=0"); fflush(stdout)
        Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [self] _ in
            let selected = text.selectedRange()
            if selected != lastSelection {
                lastSelection = selected
                print("fixture selection location=\(selected.location) length=\(selected.length)"); fflush(stdout)
            }
        }
    }
    @objc func click(_ sender: NSButton) {
        let count = Int(sender.title.split(separator: " ").last!) ?? 0
        sender.title = "Clicks: \(count + 1)"
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
@main struct FixtureMain {
    static func main() {
        let app = NSApplication.shared, delegate = Fixture()
        app.setActivationPolicy(.regular); app.delegate = delegate; app.run()
    }
}
