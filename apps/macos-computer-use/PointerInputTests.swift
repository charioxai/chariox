import AppKit

func testPointerInput() throws {
    var failures = 0
    let saved = ClickGeometry(windowBounds: CGRect(x: 160, y: 100, width: 540, height: 360),
                              elementBounds: CGRect(x: 200, y: 140, width: 100, height: 80))
    let staleEvents = try windowClickEvents(window: 123, location: saved.location,
                                           windowBounds: saved.windowBounds, eventNumber: 7)
    var posted: [CGEventType] = []
    let changed = [
        ClickGeometry(windowBounds: saved.windowBounds, elementBounds: CGRect(x: 400, y: 140, width: 100, height: 80)),
        // Resizing around the same center must also invalidate the saved click.
        ClickGeometry(windowBounds: saved.windowBounds, elementBounds: CGRect(x: 210, y: 150, width: 80, height: 60)),
        ClickGeometry(windowBounds: CGRect(x: 180, y: 120, width: 540, height: 360), elementBounds: saved.elementBounds),
        ClickGeometry(windowBounds: CGRect(x: 150, y: 90, width: 560, height: 380), elementBounds: saved.elementBounds)
    ]
    for current in changed {
        posted = []
        do {
            // Bounds change after construction; focus and window identity stay admitted.
            try dispatchInputEvents(staleEvents, fence: { event in
                let hit = try saved.checkedLocation(event.location, current: current)
                guard current.elementBounds.contains(hit) else { throw Refusal.target }
            }, releaseAllowed: { true }, post: { posted.append($0.type) })
            print("FAIL changed geometry admitted stale click"); failures += 1
        } catch Refusal.target { }
        if !posted.isEmpty { print("FAIL stale mouse-down sent after geometry changed"); failures += 1 }
    }
    var hits: [CGPoint] = []
    posted = []
    try dispatchInputEvents(staleEvents, fence: { event in
        hits.append(try saved.checkedLocation(event.location, current: saved))
    }, releaseAllowed: { true }, post: { posted.append($0.type) })
    precondition(posted == [.leftMouseDown, .leftMouseUp] &&
                 hits == [CGPoint(x: 250, y: 180), CGPoint(x: 250, y: 180)])
    posted = []
    var current = saved
    do {
        try dispatchInputEvents(staleEvents, fence: { event in
            _ = try saved.checkedLocation(event.location, current: current)
        }, releaseAllowed: { true }, post: { event in
            posted.append(event.type); current = changed[0]
        })
        print("FAIL move between paired events admitted"); failures += 1
    } catch Refusal.target { }
    precondition(posted == [.leftMouseDown]) // Neither normal nor deferred up uses stale geometry.
    for alteredPoint in [CGPoint(x: 255, y: 180), CGPoint(x: 450, y: 180)] {
        staleEvents[0].location = alteredPoint
        posted = []
        do {
            try dispatchInputEvents(staleEvents, fence: { event in
                _ = try saved.checkedLocation(event.location, current: saved)
            }, releaseAllowed: { true }, post: { posted.append($0.type) })
            print("FAIL changed event coordinate admitted"); failures += 1
        } catch Refusal.target { }
        precondition(posted.isEmpty)
    }
    if failures > 0 { exit(1) }
    print("PASS injected geometry changes refuse stale clicks; hit-tests use event coordinates")
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
