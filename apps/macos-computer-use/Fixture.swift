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
        let expected = 4 * 27 + 17 // Mid-word in "caret" on line 5, UTF-16.
        let glyph = layout.glyphIndexForCharacter(at: expected)
        let rect = layout.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: container)
        let local = NSPoint(x: rect.midX + text.textContainerOrigin.x, y: rect.midY + text.textContainerOrigin.y)
        let cocoa = window.convertPoint(toScreen: text.convert(local, to: nil))
        let point = CGPoint(x: cocoa.x, y: CGDisplayBounds(CGMainDisplayID()).height - cocoa.y)
        print("fixturePID=\(getpid()) windowID=\(window.windowNumber)")
        print("clickX=\(point.x) clickY=\(point.y) expectedLocation=\(expected)")
        print("fixture selection location=0 length=0"); fflush(stdout)
        Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [self] _ in
            let selected = text.selectedRange()
            if selected != lastSelection {
                lastSelection = selected
                print("fixture selection location=\(selected.location) length=\(selected.length)"); fflush(stdout)
            }
        }
        // The unattended drill cannot leave a window/process behind, even if
        // its runner dies before reading a helper refusal.
        Timer.scheduledTimer(withTimeInterval: 20, repeats: false) { _ in NSApp.terminate(nil) }
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
