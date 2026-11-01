import XCTest
@testable import ConsoleCore

final class LoginTests: XCTestCase {
    func testSetupRedirect() async throws {
        let transport = ScriptedTransport([jsonResponse(#"{"needs_setup":true,"cluster_name":""}"#)])
        let machine = SessionMachine(transport: transport, vault: MemoryVault())
        let phase = try await machine.begin(url: "https://cluster.example")
        XCTAssertEqual(phase, .setup)
        let requests = await transport.requests
        XCTAssertEqual(requests.map(\.url.path), ["/console/v1/setup"])
        let saved = try await machine.sessions()
        XCTAssertTrue(saved.isEmpty)
    }

    func testDirectSession() async throws {
        let now = Date(timeIntervalSince1970: 1_700_000_000)
        let token = TokenTime.make(exp: now.addingTimeInterval(12 * 60 * 60))
        let transport = ScriptedTransport([jsonResponse(#"{"access_token":"\#(token)"}"#)])
        let machine = SessionMachine(transport: transport, vault: MemoryVault())
        let phase = try await machine.signIn(
            url: "https://cluster.example",
            name: "Local",
            email: "ada@example.com",
            password: "scripted-pass",
            now: now
        )
        guard case .session(let record) = phase else {
            return XCTFail("expected a session")
        }
        XCTAssertEqual(record.email, "ada@example.com")
        XCTAssertEqual(record.accessToken, token)
        let saved = try await machine.sessions()
        XCTAssertEqual(saved.map(\.accessToken), [token])
    }

    func testOTPSuccess() async throws {
        let now = Date(timeIntervalSince1970: 1_700_000_000)
        let token = TokenTime.make(exp: now.addingTimeInterval(12 * 60 * 60))
        let transport = ScriptedTransport([
            jsonResponse(#"{"mfa_required":true,"mfa_token":"mfa-1"}"#),
            jsonResponse(#"{"access_token":"\#(token)"}"#),
        ])
        let machine = SessionMachine(transport: transport, vault: MemoryVault())
        let pending = try await machine.signIn(
            url: "https://cluster.example",
            name: "Local",
            email: "ada@example.com",
            password: "scripted-pass",
            now: now
        )
        XCTAssertEqual(pending, .otp(mfaToken: "mfa-1"))
        let saved = try await machine.sessions()
        XCTAssertTrue(saved.isEmpty)
        let phase = try await machine.submitOTP(url: "https://cluster.example", code: "123456", now: now)
        guard case .session(let record) = phase else { return XCTFail("expected a session") }
        XCTAssertEqual(record.accessToken, token)
        let requests = await transport.requests
        XCTAssertEqual(requests[1].url.path, "/console/v1/mfa/totp")
        XCTAssertEqual(requests[1].headers["authorization"], "Bearer mfa-1")
    }

    func testRecoverySuccess() async throws {
        let now = Date(timeIntervalSince1970: 1_700_000_000)
        let token = TokenTime.make(exp: now.addingTimeInterval(12 * 60 * 60))
        let transport = ScriptedTransport([
            jsonResponse(#"{"mfa_token":"mfa-1"}"#),
            jsonResponse(#"{"access_token":"\#(token)"}"#),
        ])
        let machine = SessionMachine(transport: transport, vault: MemoryVault())
        _ = try await machine.signIn(url: "https://cluster.example", name: "Local", email: "ada@example.com", password: "pw", now: now)
        let phase = try await machine.submitRecovery(url: "https://cluster.example/", code: "backup", now: now)
        guard case .session = phase else { return XCTFail("expected a session") }
        let requests = await transport.requests
        XCTAssertEqual(requests[1].url.path, "/console/v1/mfa/recovery")
    }

    func testBadOTPStaysOnTheOTPStep() async throws {
        let transport = ScriptedTransport([
            jsonResponse(#"{"mfa_token":"mfa-1"}"#),
            jsonResponse(#"{"error":"invalid code"}"#, status: 401),
        ])
        let machine = SessionMachine(transport: transport, vault: MemoryVault())
        _ = try await machine.signIn(url: "https://cluster.example", name: "Local", email: "ada@example.com", password: "pw")
        let phase = try await machine.submitOTP(url: "https://cluster.example", code: "000000")
        XCTAssertEqual(phase, .otp(mfaToken: "mfa-1"))
        let saved = try await machine.sessions()
        XCTAssertTrue(saved.isEmpty)
    }

    func testEnrollmentReturnsTheBrowserURLAndWritesNothing() async throws {
        let transport = ScriptedTransport([
            jsonResponse(#"{"enrollment_required":true,"setup_token":"setup-1"}"#),
        ])
        let machine = SessionMachine(transport: transport, vault: MemoryVault())
        let phase = try await machine.signIn(url: "https://cluster.example/", name: "Local", email: "ada@example.com", password: "pw")
        XCTAssertEqual(phase, .enrollment(url: "https://cluster.example/console/login"))
        let saved = try await machine.sessions()
        XCTAssertTrue(saved.isEmpty)
    }

    func testTwoClustersDoNotShareTokens() async throws {
        let now = Date(timeIntervalSince1970: 1_700_000_000)
        let first = TokenTime.make(exp: now.addingTimeInterval(12 * 60 * 60))
        let second = TokenTime.make(exp: now.addingTimeInterval(11 * 60 * 60))
        let transport = ScriptedTransport([
            jsonResponse(#"{"access_token":"\#(first)"}"#),
            jsonResponse(#"{"access_token":"\#(second)"}"#),
        ])
        let machine = SessionMachine(transport: transport, vault: MemoryVault())
        _ = try await machine.signIn(url: "https://a.example", name: "A", email: "a@example.com", password: "pw", now: now)
        _ = try await machine.signIn(url: "https://b.example", name: "B", email: "b@example.com", password: "pw", now: now)
        let saved = try await machine.sessions()
        XCTAssertEqual(Set(saved.map(\.url)), ["https://a.example", "https://b.example"])
        XCTAssertEqual(Set(saved.map(\.accessToken)), [first, second])
        XCTAssertNotEqual(first, second)
    }

    func testLogoutOfOneLeavesTheOther() async throws {
        let now = Date(timeIntervalSince1970: 1_700_000_000)
        let first = TokenTime.make(exp: now.addingTimeInterval(12 * 60 * 60))
        let second = TokenTime.make(exp: now.addingTimeInterval(11 * 60 * 60))
        let transport = ScriptedTransport([
            jsonResponse(#"{"access_token":"\#(first)"}"#),
            jsonResponse(#"{"access_token":"\#(second)"}"#),
        ])
        let machine = SessionMachine(transport: transport, vault: MemoryVault())
        _ = try await machine.signIn(url: "https://a.example", name: "A", email: "a@example.com", password: "pw", now: now)
        _ = try await machine.signIn(url: "https://b.example", name: "B", email: "b@example.com", password: "pw", now: now)
        try await machine.logout(url: "https://a.example")
        let saved = try await machine.sessions()
        XCTAssertEqual(saved.map(\.url), ["https://b.example"])
        XCTAssertEqual(saved.map(\.accessToken), [second])
    }

    func testUnauthorizedAndNearExpiryMarkOnlyThatCluster() async throws {
        let now = Date(timeIntervalSince1970: 1_700_000_000)
        let soon = TokenTime.make(exp: now.addingTimeInterval(5 * 60))
        let later = TokenTime.make(exp: now.addingTimeInterval(12 * 60 * 60))
        let transport = ScriptedTransport([
            jsonResponse(#"{"access_token":"\#(soon)"}"#),
            jsonResponse(#"{"access_token":"\#(later)"}"#),
        ])
        let machine = SessionMachine(transport: transport, vault: MemoryVault())
        _ = try await machine.signIn(url: "https://a.example", name: "A", email: "a@example.com", password: "pw", now: now)
        _ = try await machine.signIn(url: "https://b.example", name: "B", email: "b@example.com", password: "pw", now: now)
        let aNeeds = try await machine.needsReauth(url: "https://a.example", now: now)
        let bNeeds = try await machine.needsReauth(url: "https://b.example", now: now)
        XCTAssertTrue(aNeeds)
        XCTAssertFalse(bNeeds)
        await machine.noteUnauthorized(url: "https://b.example")
        let bAfter = try await machine.needsReauth(url: "https://b.example", now: now)
        let cNeeds = try await machine.needsReauth(url: "https://c.example", now: now)
        XCTAssertTrue(bAfter)
        XCTAssertFalse(cNeeds)
        let saved = try await machine.sessions()
        XCTAssertEqual(Set(saved.map(\.url)), ["https://a.example", "https://b.example"])
    }
}
