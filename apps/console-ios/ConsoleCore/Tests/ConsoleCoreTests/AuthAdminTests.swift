import XCTest
@testable import ConsoleCore

final class AuthAdminTests: XCTestCase {
    func testUsersProvidersMfaEmailAndTemplates() async throws {
        let (transport, client) = scripted(Array(repeating: jsonResponse(#"{"ok":true}"#), count: 17))
        let store = AuthAdminStore(client: client)
        let pref = "abcdefghijklmnopqrst"
        _ = try await store.users(pref: pref)
        _ = try await store.user(pref: pref, id: "user-1")
        try await store.setPassword(pref: pref, id: "user-1", password: "scripted-pass")
        try await store.deleteUser(pref: pref, id: "user-1")
        _ = try await store.authSettings(pref: pref)
        _ = try await store.updateAuth(pref: pref, requireEmailVerification: true, requireMFA: true)
        _ = try await store.providers(pref: pref)
        _ = try await store.upsertProvider(
            pref: pref,
            provider: "apple",
            clientID: "id",
            clientSecret: nil,
            redirects: ["https://app.example/callback"],
            extra: ["team_id": "TEAM", "key_id": "KEY", "private_key": "secret-key"]
        )
        _ = try await store.upsertProvider(
            pref: pref,
            provider: "github",
            clientID: "id",
            clientSecret: "",
            redirects: [],
            extra: [:]
        )
        try await store.deleteProvider(pref: pref, provider: "github")
        _ = try await store.email(pref: pref)
        _ = try await store.saveEmail(
            pref: pref,
            host: "smtp.example",
            port: 587,
            username: "mail",
            password: nil,
            fromAddress: "auth@example.com",
            tls: "starttls",
            linkBase: "https://app.example"
        )
        _ = try await store.setClusterSMTP(pref: pref, enabled: true)
        _ = try await store.testEmail(pref: pref, to: "ada@example.com")
        _ = try await store.templates(pref: pref)
        _ = try await store.saveTemplate(
            pref: pref,
            name: "invite",
            subject: "Hello",
            bodyText: "Hi {{email}} {{link}}",
            bodyHTML: "<p>{{email}}</p>"
        )
        _ = try await store.resetTemplate(pref: pref, name: "invite")

        let requests = await transport.requests
        XCTAssertEqual(requests.map(\.method), [
            "GET", "GET", "POST", "DELETE", "GET", "PUT", "GET", "POST", "POST", "DELETE", "GET", "PUT", "PUT", "POST", "GET", "PUT", "POST",
        ])
        XCTAssertEqual(OAuthCatalog.providers.count, 10)
        XCTAssertTrue(OAuthCatalog.providers.contains("apple"))
        let apple = try objectBody(requests[7].body)
        XCTAssertNil(apple["client_secret"])
        let extra = apple["extra"] as? [String: Any]
        XCTAssertEqual(extra?["private_key"] as? String, "secret-key")
        let github = try objectBody(requests[8].body)
        XCTAssertNil(github["client_secret"])
        let smtp = try objectBody(requests[11].body)
        XCTAssertNil(smtp["password"])
        XCTAssertEqual(smtp["tls"] as? String, "starttls")
        let cluster = try objectBody(requests[12].body)
        XCTAssertEqual(cluster["enabled"] as? Bool, true)
        let template = try objectBody(requests[15].body)
        XCTAssertEqual(template["body_text"] as? String, "Hi {{email}} {{link}}")
        XCTAssertEqual(requests[13].url.path, "/console/v1/projects/\(pref)/email/test")
        XCTAssertEqual(requests[16].url.path, "/console/v1/projects/\(pref)/email/templates/invite/reset")
        let flags = try objectBody(requests[5].body)
        XCTAssertEqual(flags["require_mfa"] as? Bool, true)
        XCTAssertEqual(flags["require_email_verification"] as? Bool, true)
    }
}
