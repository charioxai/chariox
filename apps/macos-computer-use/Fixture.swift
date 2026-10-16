import AppKit

final class Fixture: NSObject, NSApplicationDelegate {
    let window = NSWindow(contentRect: NSRect(x: 160, y: 160, width: 540, height: 360),
                          styleMask: [.titled, .closable], backing: .buffered, defer: false)
    let patch = NSView(frame: NSRect(x: 20, y: 20, width: 60, height: 60))
    let secondPatch = NSView(frame: NSRect(x: 460, y: 40, width: 30, height: 30))
    var ticks = 0
    var drillText: NSTextView?
    var lastSelection: NSRange?
    var clickCount = 0
    var coverPanel: NSPanel?
    var coverSignal: DispatchSourceSignal?
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
        if CommandLine.arguments.contains("--cover-panel") { armCoverPanel() }
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
        // Human-visible click oracle: a large banner and a highlighted line follow every selection change.
        let banner = NSTextField(labelWithString: "Waiting for the agent's click")
        banner.frame = NSRect(x: 20, y: 300, width: 500, height: 44)
        banner.font = .boldSystemFont(ofSize: 26); banner.alignment = .center
        banner.wantsLayer = true; banner.layer?.backgroundColor = NSColor.systemGray.withAlphaComponent(0.3).cgColor
        window.contentView!.addSubview(banner)
        let scroll = NSScrollView(frame: NSRect(x: 20, y: 20, width: 500, height: 270))
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
                if selected.location > 0 { markClick(selected.location, in: text, banner: banner, layout: layout) }
                print("fixture selection location=\(selected.location) length=\(selected.length)"); fflush(stdout)
            }
        }
    }
    func markClick(_ location: Int, in text: NSTextView, banner: NSTextField, layout: NSLayoutManager) {
        let string = text.string as NSString
        let line = string.substring(to: location).filter { $0 == "\n" }.count + 1
        clickCount += 1
        layout.removeTemporaryAttribute(.backgroundColor, forCharacterRange: NSRange(location: 0, length: string.length))
        layout.addTemporaryAttribute(.backgroundColor, value: NSColor.systemYellow, forCharacterRange: string.lineRange(for: NSRange(location: location, length: 0)))
        banner.stringValue = "agent click #\(clickCount) on line \(line)"
        banner.layer?.backgroundColor = NSColor.systemYellow.cgColor
    }
    // Same-process cover drill: SIGUSR1 toggles a non-key, click-through floating panel over the
    // still-focused main window. Focus and the app-scoped AX hit still land on the target, so only
    // the WindowServer stack fence can refuse it.
    func armCoverPanel() {
        signal(SIGUSR1, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: SIGUSR1, queue: .main)
        source.setEventHandler { [self] in
            if let panel = coverPanel { panel.close(); coverPanel = nil; print("coverPanel closed"); fflush(stdout); return }
            let frame = NSRect(x: window.frame.minX + 170, y: window.frame.minY + 60, width: 200, height: 140)
            let panel = NSPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
            panel.level = .floating; panel.hidesOnDeactivate = false; panel.isReleasedWhenClosed = false
            panel.ignoresMouseEvents = true
            panel.backgroundColor = .systemPurple; panel.isOpaque = true; panel.hasShadow = false
            panel.orderFrontRegardless(); coverPanel = panel
            print("coverPanelPID=\(getpid()) windowID=\(panel.windowNumber) mainWindowID=\(window.windowNumber) frame=\(panel.frame)"); fflush(stdout)
        }
        source.resume(); coverSignal = source
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
