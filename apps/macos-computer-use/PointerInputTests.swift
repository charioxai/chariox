import AppKit

func testPointerInput() throws {
    let oversized = ClickGeometry(windowBounds: CGRect(x: 100, y: 100, width: 500, height: 300),
                                  elementBounds: CGRect(x: 120, y: 120, width: 400, height: 1200))
    guard oversized.location == CGPoint(x: 320, y: 260) else {
        print("FAIL oversized text element click center is outside its visible window"); exit(1)
    }
    _ = try oversized.checkedLocation(oversized.location, current: oversized)
    let partlyOffscreen = CGRect(x: -300, y: 100, width: 500, height: 300)
    let screen = CGRect(x: 0, y: 0, width: 1000, height: 800)
    let visible = try visibleWindowFrame(window: partlyOffscreen, element: partlyOffscreen, displays: [screen], point: nil)
    let clamped = ClickGeometry(windowBounds: partlyOffscreen, elementBounds: partlyOffscreen, visibleWindowBounds: visible)
    precondition(clamped.location == CGPoint(x: 100, y: 250))
    _ = try clamped.checkedLocation(clamped.location, current: clamped)
    for element in [CGRect(x: 700, y: 700, width: 100, height: 100),
                    CGRect(x: -200, y: 120, width: 100, height: 100)] {
        do {
            _ = try visibleWindowFrame(window: partlyOffscreen, element: element, displays: [screen], point: nil)
            preconditionFailure("invisible element admitted")
        } catch Refusal.target { }
    }
    let explicit = ClickGeometry(windowBounds: oversized.windowBounds, elementBounds: oversized.elementBounds,
                                 requestedPoint: CGPoint(x: 140, y: 180))
    _ = try explicit.checkedLocation(explicit.location, current: explicit)
    do {
        _ = try visibleWindowFrame(window: partlyOffscreen, element: partlyOffscreen, displays: [screen],
                                   point: CGPoint(x: -100, y: 200))
        preconditionFailure("offscreen requested point admitted")
    } catch Refusal.target { }
    print("PASS oversized/offscreen element centers clamp to visible intersection; invisible targets refuse")
    let requested = CGPoint(x: 140, y: 180)
    for range in [CFRange(location: 12, length: 1), CFRange(location: 12, length: 2), CFRange(location: 12, length: 0)] {
        let resolution = try resolveTextClick(at: requested) { point in
            precondition(point == requested); return range
        }
        precondition(resolution == .selection(12))
    }
    let fallback = try resolveTextClick(at: requested) { _ in nil }
    precondition(fallback == .hid)
    for range in [CFRange(location: kCFNotFound, length: 0), CFRange(location: 1, length: -1),
                  CFRange(location: Int.max, length: 1)] {
        do {
            _ = try resolveTextClick(at: requested) { _ in range }
            preconditionFailure("malformed AX range selected fallback")
        } catch Refusal.target { }
    }
    do {
        _ = try resolveTextClick(at: requested) { _ in throw Refusal.permission }
        preconditionFailure("AX permission failure selected fallback")
    } catch Refusal.permission { }
    let parsed = try Request.parse(["--enable-owner-window", "--owner-pid", "42", "--window-id", "1",
                                   "--click-at", "140", "180", "--click"])
    precondition(parsed.clickPoint == requested && parsed.operations == [.click])
    let pointerRequest = try Request.parse(["--enable-owner-window", "--click-at-pointer", "--click"])
    precondition(pointerRequest.clickAtPointer && pointerRequest.clickPoint == nil)
    for args in [["--click-at", "nan", "180", "--click"], ["--click-at", "140", "180", "--scroll"],
                 ["--click-at", "140", "180", "--click-at", "140", "180", "--click"],
                 ["--click-at-pointer", "--click-at", "140", "180", "--click"],
                 ["--click-at-pointer", "--scroll"], ["--click-at-pointer", "--click-at-pointer", "--click"]] {
        do { _ = try Request.parse(args); preconditionFailure("invalid point arguments admitted") }
        catch Refusal.arguments { }
    }
    print("PASS range-at-point preserves composed-character start; only unresolved AX range selects HID")
    var failures = 0
    let saved = ClickGeometry(windowBounds: CGRect(x: 160, y: 100, width: 540, height: 360),
                              elementBounds: CGRect(x: 200, y: 140, width: 100, height: 80))
    let staleEvents = try hidClickEvents(window: 123, location: saved.location,
                                           windowBounds: saved.windowBounds, eventNumber: 7)
    var posted: [CGEventType] = []
    let changed = [
        ClickGeometry(windowBounds: saved.windowBounds, elementBounds: CGRect(x: 400, y: 140, width: 100, height: 80)),
        // Resizing around the same center must also invalidate the saved click.
        ClickGeometry(windowBounds: saved.windowBounds, elementBounds: CGRect(x: 210, y: 150, width: 80, height: 60)),
        ClickGeometry(windowBounds: CGRect(x: 180, y: 120, width: 540, height: 360), elementBounds: saved.elementBounds),
        ClickGeometry(windowBounds: CGRect(x: 150, y: 90, width: 560, height: 380), elementBounds: saved.elementBounds),
        ClickGeometry(windowBounds: saved.windowBounds, elementBounds: saved.elementBounds,
                      visibleWindowBounds: CGRect(x: 180, y: 120, width: 200, height: 180))
    ]
    for current in changed {
        posted = []
        do {
            // Bounds change after construction; focus and window identity stay admitted.
            try dispatchInputEvents(staleEvents, fence: { event in
                let hit = try saved.checkedLocation(event.location, current: current)
                guard current.elementBounds.contains(hit) else { throw Refusal.target }
            }, releaseFence: { _ in preconditionFailure() }, releaseAllowed: { true }, post: { posted.append($0.type) })
            print("FAIL changed geometry admitted stale click"); failures += 1
        } catch Refusal.target { }
        if !posted.isEmpty { print("FAIL stale mouse-down sent after geometry changed"); failures += 1 }
    }
    var hits: [CGPoint] = []
    posted = []
    try dispatchInputEvents(staleEvents, fence: { event in
        hits.append(try saved.checkedLocation(event.location, current: saved))
    }, releaseFence: { _ in preconditionFailure() }, releaseAllowed: { true }, post: { posted.append($0.type) })
    precondition(posted == [.leftMouseDown, .leftMouseUp] &&
                 hits == [CGPoint(x: 250, y: 180), CGPoint(x: 250, y: 180)])
    for moved in changed {
        posted = []
        var current = saved, cleanupCalls = 0
        do {
            try dispatchInputEvents(staleEvents, fence: { event in
                _ = try saved.checkedLocation(event.location, current: current)
            }, releaseFence: { event in
                precondition(event === staleEvents[1] && event.type == .leftMouseUp)
                cleanupCalls += 1
            }, releaseAllowed: { true }, post: { event in
                posted.append(event.type); current = moved
            })
            print("FAIL move between paired events admitted"); failures += 1
        } catch Refusal.target { }
        if posted != [.leftMouseDown, .leftMouseUp] || cleanupCalls != 1 {
            print("FAIL geometry change stranded owned mouse-down without cleanup"); failures += 1
        }
    }
    // Ownership/security loss and permission loss cannot silently strand a press.
    for denied in [Refusal.target, .secure, .permission] {
        posted = []
        var current = saved
        do {
            try dispatchInputEvents(staleEvents, fence: { event in
                _ = try saved.checkedLocation(event.location, current: current)
            }, releaseFence: { _ in
                if denied != .permission { throw denied }
            }, releaseAllowed: { posted.isEmpty || denied != .permission }, post: { event in
                posted.append(event.type); current = changed[0]
            })
            print("FAIL unsafe cleanup admitted"); failures += 1
        } catch Refusal.ownedInput {
            precondition(refusalMessage(Refusal.ownedInput) ==
                "refused: ownedInput; unresolved owned input; owner reset required")
        } catch {
            print("FAIL owned input reported as ordinary refusal"); failures += 1
        }
        precondition(posted == [.leftMouseDown])
    }
    // A revoked posting grant is checked on normal dispatch as well as cleanup.
    posted = []
    do {
        try dispatchInputEvents(staleEvents, fence: { _ in }, releaseFence: { _ in },
                                releaseAllowed: { posted.isEmpty }, post: { posted.append($0.type) })
        print("FAIL posting permission loss admitted"); failures += 1
    } catch Refusal.ownedInput { }
    precondition(posted == [.leftMouseDown])
    // Before a press there is no owned release and no owner reset requirement.
    posted = []
    do {
        try dispatchInputEvents(staleEvents, fence: { _ in },
                                releaseFence: { _ in preconditionFailure() },
                                releaseAllowed: { false }, post: { posted.append($0.type) })
        print("FAIL missing posting permission admitted"); failures += 1
    } catch Refusal.permission { }
    precondition(posted.isEmpty)
    for alteredPoint in [CGPoint(x: 255, y: 180), CGPoint(x: 450, y: 180)] {
        staleEvents[0].location = alteredPoint
        posted = []
        do {
            try dispatchInputEvents(staleEvents, fence: { event in
                _ = try saved.checkedLocation(event.location, current: saved)
            }, releaseFence: { _ in preconditionFailure() }, releaseAllowed: { true }, post: { posted.append($0.type) })
            print("FAIL changed event coordinate admitted"); failures += 1
        } catch Refusal.target { }
        precondition(posted.isEmpty)
    }
    if failures > 0 { exit(1) }
    print("PASS stale presses refused; geometry changes clean up owned releases or require owner reset")
    let events = try hidClickEvents(window: 123, location: CGPoint(x: 250, y: 180),
                                       windowBounds: CGRect(x: 160, y: 100, width: 540, height: 360), eventNumber: 7)
    for event in events {
        guard event.getIntegerValueField(.mouseEventClickState) == 1,
              event.getIntegerValueField(.mouseEventNumber) == 7,
              event.location == CGPoint(x: 250, y: 180), event.flags.isEmpty,
              event.getIntegerValueField(.mouseEventWindowUnderMousePointer) == 123,
              event.getIntegerValueField(.mouseEventWindowUnderMousePointerThatCanHandleThisEvent) == 123 else {
            print("FAIL HID click has no selected window/single-click/pair number"); failures += 1; continue
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
            _ = try hidClickEvents(window: window, location: location,
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
    precondition(inputReceipt(path: "CGEventHID") == "dispatched; path=CGEventHID; application completion unproven")
    precondition(inputReceipt(path: "AXPress", observed: "observed fixture counter increment") ==
        "dispatched; path=AXPress; observed fixture counter increment")
    for (operation, role, expected) in [(Operation.click, kAXButtonRole, InputPath.axPress),
                                       (.scroll, kAXScrollAreaRole, .axScrollValue),
                                       (.click, kAXTextAreaRole, .axTextClick),
                                       (.text("public"), kAXTextAreaRole, .pidText)] {
        if try inputPath(operation, role: role) != expected {
            print("FAIL wrong input path for \(operation) \(role)"); failures += 1
        }
    }
    if failures > 0 { exit(1) }
    print("PASS HID click pair and AX-first role-selected primary input paths")
}
