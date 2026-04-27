import XCTest

final class ArrobaUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testWaitingRoomLaunches() throws {
        let app = XCUIApplication()
        app.launch()

        XCTAssertTrue(app.staticTexts["ARROBA"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["waiting-refresh"].exists)
        XCTAssertTrue(app.textFields["Kernel"].exists)
        XCTAssertTrue(app.buttons["waiting-attach-session"].exists)
        XCTAssertTrue(app.buttons["waiting-detach-session"].exists)
        XCTAssertTrue(app.textViews["prompt-composer"].exists)
        XCTAssertTrue(app.buttons["prompt-send"].exists)
        XCTAssertTrue(app.buttons["prompt-stop"].exists)
    }
}
