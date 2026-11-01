import XCTest
@testable import ConsoleCore

final class DrawerTests: XCTestCase {
    func testItemListAndOrder() {
        var drawer = DrawerModel(projectRef: "abcdefghijklmnopqrst", role: .developer)
        XCTAssertEqual(
            drawer.items().map(\.title),
            ["Overview", "API keys", "Auth", "Data", "SQL", "Storage", "Functions", "Logs", "Team", "Settings"]
        )
        drawer.queueEnabled = true
        XCTAssertEqual(
            drawer.items().map(\.title),
            ["Overview", "API keys", "Auth", "Data", "SQL", "Storage", "Functions", "Queue", "Logs", "Team", "Settings"]
        )
    }

    func testQueueAppearsOnlyWithTheExtension() {
        let off = DrawerModel(projectRef: "abcdefghijklmnopqrst", queueEnabled: false)
        let on = DrawerModel(projectRef: "abcdefghijklmnopqrst", queueEnabled: true)
        XCTAssertFalse(off.items().contains { $0.screen == .queue })
        XCTAssertTrue(on.items().contains { $0.screen == .queue })
    }

    func testPlatformAdminItems() {
        let member = DrawerModel(platformAdmin: false)
        let admin = DrawerModel(platformAdmin: true)
        XCTAssertEqual(member.items().map(\.screen), [.projects])
        XCTAssertEqual(admin.items().map(\.screen), [.projects, .cluster, .operators])
    }

    func testSectionReplaceVersusPush() {
        var drawer = DrawerModel(projectRef: "abcdefghijklmnopqrst", role: .admin)
        drawer.select(.overview)
        drawer.select(.data)
        XCTAssertEqual(drawer.stack, [.section(.data)])
        drawer.openTable("todos")
        XCTAssertEqual(drawer.stack, [.section(.data), .table("todos")])
        drawer.select(.authUsers)
        drawer.openUser("user-1")
        XCTAssertEqual(drawer.stack, [.section(.authUsers), .user("user-1")])
        drawer.select(.functionOverview)
        drawer.openFunction("ping")
        XCTAssertEqual(drawer.stack, [.section(.functionOverview), .function("ping")])
        drawer.select(.functionLogs)
        XCTAssertEqual(drawer.stack, [.section(.functionLogs), .function("ping")])
    }

    func testDeveloperWriteActionsHidden() {
        let drawer = DrawerModel(projectRef: "abcdefghijklmnopqrst", role: .developer, platformAdmin: false)
        XCTAssertFalse(drawer.can(.editRows))
        XCTAssertFalse(drawer.can(.runSql))
        XCTAssertFalse(drawer.can(.upload))
        XCTAssertFalse(drawer.can(.deployFunction))
        XCTAssertFalse(drawer.can(.writeEnv))
        XCTAssertFalse(drawer.can(.manageAuth))
        XCTAssertFalse(drawer.can(.renameProject))
        XCTAssertFalse(drawer.can(.deleteProject))
        XCTAssertFalse(drawer.can(.manageCluster))
    }

    func testAdminCannotRenameOrDelete() {
        let drawer = DrawerModel(projectRef: "abcdefghijklmnopqrst", role: .admin)
        XCTAssertTrue(drawer.can(.runSql))
        XCTAssertTrue(drawer.can(.editRows))
        XCTAssertFalse(drawer.can(.renameProject))
        XCTAssertFalse(drawer.can(.deleteProject))
    }

    func testOwnerCanRenameAndDelete() {
        let drawer = DrawerModel(projectRef: "abcdefghijklmnopqrst", role: .owner, platformAdmin: true)
        XCTAssertTrue(drawer.can(.renameProject))
        XCTAssertTrue(drawer.can(.deleteProject))
        XCTAssertTrue(drawer.can(.manageCluster))
        XCTAssertFalse(SettingsAvailability.migrateAvailable)
        XCTAssertFalse(SettingsAvailability.transferAvailable)
    }
}
