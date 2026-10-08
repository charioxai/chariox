import AppKit

func testPointerInput() throws {
    var failures = 0
    let events = try windowClickEvents(window: 123, location: CGPoint(x: 250, y: 180),
                                       windowBounds: CGRect(x: 160, y: 100, width: 540, height: 360), eventNumber: 7)
    for event in events {
        guard let appKit = NSEvent(cgEvent: event), appKit.windowNumber == 123,
              appKit.clickCount == 1, appKit.eventNumber == 7,
              event.location == CGPoint(x: 250, y: 180), event.flags.isEmpty,
              event.getIntegerValueField(.mouseEventWindowUnderMousePointer) == 123,
              event.getIntegerValueField(.mouseEventWindowUnderMousePointerThatCanHandleThisEvent) == 123 else {
            print("FAIL AppKit click has no selected window/single-click/pair number"); failures += 1; continue
        }
    }
    for (operation, role) in [(Operation.click, kAXWindowRole), (.scroll, kAXTextAreaRole),
                              (.text("public"), kAXButtonRole), (.click, kAXUnknownRole)] {
        do { _ = try inputPath(operation, role: role); print("FAIL unsupported role admitted"); failures += 1 }
        catch Refusal.target { }
    }
    for (window, location, number) in [(UInt32(0), CGPoint(x: 250, y: 180), 7),
                                      (123, CGPoint(x: 159, y: 180), 7), (123, CGPoint(x: 250, y: 180), 0)] {
        do {
            _ = try windowClickEvents(window: window, location: location,
                windowBounds: CGRect(x: 160, y: 100, width: 540, height: 360), eventNumber: number)
            print("FAIL invalid click target admitted"); failures += 1
        } catch Refusal.target { }
    }
    let firstStep = try nextScrollValue(0), lastStep = try nextScrollValue(0.99)
    precondition(firstStep == 0.05 && lastStep == 1)
    for value in [-0.1, 1, 1.1, Double.nan, Double.infinity] {
        do { _ = try nextScrollValue(value); print("FAIL invalid scroll value admitted"); failures += 1 }
        catch Refusal.target { }
    }
    precondition(inputReceipt(path: "CGEventWindow") == "dispatched; path=CGEventWindow; application completion unproven")
    precondition(inputReceipt(path: "AXPress", observed: "observed fixture counter increment") ==
        "dispatched; path=AXPress; observed fixture counter increment")
    for (operation, role, expected) in [(Operation.click, kAXButtonRole, InputPath.axPress),
                                       (.scroll, kAXScrollAreaRole, .axScrollValue),
                                       (.click, kAXTextAreaRole, .windowEvent),
                                       (.text("public"), kAXTextAreaRole, .pidText)] {
        if try inputPath(operation, role: role) != expected {
            print("FAIL wrong input path for \(operation) \(role)"); failures += 1
        }
    }
    if failures > 0 { exit(1) }
    print("PASS AppKit window-bound click pair and role-selected primary input paths")
}
