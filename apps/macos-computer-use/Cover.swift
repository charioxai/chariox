import AppKit
final class CoverDelegate: NSObject, NSApplicationDelegate {
    var panel: NSPanel?
    func applicationDidFinishLaunching(_ notification: Notification) {
        let args = CommandLine.arguments.dropFirst().compactMap(Double.init)
        guard args.count == 4 else { exit(2) }
        let frame = NSRect(x: args[0], y: args[1], width: args[2], height: args[3])
        let panel = NSPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        self.panel = panel
        panel.level = .floating; panel.hidesOnDeactivate = false
        panel.backgroundColor = .systemPurple; panel.isOpaque = true; panel.hasShadow = false
        panel.orderFrontRegardless()
        print("coverPID=\(getpid()) windowID=\(panel.windowNumber) frame=\(panel.frame)"); fflush(stdout)
        Timer.scheduledTimer(withTimeInterval: 10, repeats: false) { _ in NSApp.terminate(nil) }
    }
}
@main struct PublicCover {
    static func main() {
        let app = NSApplication.shared, delegate = CoverDelegate()
        app.setActivationPolicy(.accessory); app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}
