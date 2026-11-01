import XCTest
@testable import ConsoleCore

final class ProjectTests: XCTestCase {
    func testListCreateOverviewKeysTeamAndLogs() async throws {
        let (transport, client) = scripted([
            jsonResponse(#"[{"ref":"abcdefghijklmnopqrst","name":"Todos","role":"owner"}]"#),
            jsonResponse(#"{"operator":{"id":"op","email":"ada@example.com","name":"Ada","platform_admin":true},"cluster_name":"local","projects":[]}"#),
            jsonResponse(#"{"ref":"abcdefghijklmnopqrst","name":"Todos","role":"owner","anon_key":"anon","service_key":"service"}"#),
            jsonResponse(#"{"name":"Todos","counts":{"users":1,"tables":2,"files":3,"functions":4,"sites":5},"hours":[1],"series":{"auth":[{"ok":1,"warn":0,"error":0}]}}"#),
            jsonResponse(#"{"anon_key":"anon2","service_key":"service2"}"#),
            jsonResponse(#"[{"email":"ada@example.com","name":"Ada","role":"owner"}]"#),
            jsonResponse(#"{"email":"dev@example.com","name":"Dev","role":"developer"}"#),
            jsonResponse(#"[{"id":1,"kind":"function","name":"ping","status":200,"message":"ok","created_at":"t"}]"#),
        ])
        let projects = ProjectsStore(client: client)
        _ = try await projects.load()
        _ = try await projects.loadMe()
        _ = try await projects.create(name: "Todos")
        XCTAssertEqual(projects.hold?.fields.map(\.name), ["anon_key", "service_key"])
        projects.acknowledgeSecrets()
        XCTAssertNil(projects.hold)

        _ = try await OverviewStore(client: client).load(pref: "abcdefghijklmnopqrst")
        let keys = KeysStore(client: client)
        _ = try await keys.rotate(pref: "abcdefghijklmnopqrst")
        XCTAssertEqual(keys.hold?.fields.map(\.value), ["anon2", "service2"])
        keys.acknowledgeSecrets()
        XCTAssertNil(keys.hold)

        let team = TeamStore(client: client)
        _ = try await team.list(pref: "abcdefghijklmnopqrst")
        _ = try await team.add(pref: "abcdefghijklmnopqrst", email: "dev@example.com", name: "Dev", password: "scripted-pass", role: "developer")
        _ = try await LogsStore(client: client).search(pref: "abcdefghijklmnopqrst", query: "ping")

        let requests = await transport.requests
        XCTAssertEqual(requests.map(\.url.path), [
            "/console/v1/projects",
            "/console/v1/me",
            "/console/v1/projects",
            "/console/v1/projects/abcdefghijklmnopqrst/overview",
            "/console/v1/projects/abcdefghijklmnopqrst/keys",
            "/console/v1/projects/abcdefghijklmnopqrst/members",
            "/console/v1/projects/abcdefghijklmnopqrst/members",
            "/console/v1/projects/abcdefghijklmnopqrst/logs",
        ])
        XCTAssertEqual(requests[2].method, "POST")
        XCTAssertEqual(requests[4].method, "POST")
        let member = try objectBody(requests[6].body)
        XCTAssertEqual(member["role"] as? String, "developer")
        XCTAssertEqual(member["password"] as? String, "scripted-pass")
        XCTAssertTrue(requests[7].url.query?.contains("q=ping") == true)
    }
}
