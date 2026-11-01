import XCTest
@testable import ConsoleCore

final class PlatformTests: XCTestCase {
    func testClusterOperatorsKeysAndAgent() async throws {
        let (transport, client) = scripted([
            jsonResponse(#"{"name":"local","place":"docker","type":"single","functions_runtime":"bun","projects":[],"reactor":{"ok":true},"postgres":{"ok":true},"postgrest":{"ok":true},"postgrest_dedicated":null,"blobs":{"ok":true,"backend":"fs"}}"#),
            jsonResponse(#"{"name":"renamed"}"#),
            jsonResponse(#"{"host":"","port":587,"username":"","password_set":false,"from_address":"","tls":"starttls"}"#),
            jsonResponse(#"{"host":"smtp.example","port":587,"username":"","password_set":true,"from_address":"a@example.com","tls":"starttls"}"#),
            jsonResponse(#"[{"id":"op","email":"ada@example.com","name":"Ada","platform_admin":true,"projects":[]}]"#),
            jsonResponse(#"{"id":"new","email":"new@example.com","name":"New","platform_admin":false,"projects":[]}"#),
            jsonResponse("null", status: 204),
            jsonResponse("null", status: 204),
            jsonResponse("null", status: 204),
            jsonResponse("null", status: 204),
            jsonResponse(#"[{"id":"key","name":"ci","scopes":["projects.create"],"created_at":"t"}]"#),
            jsonResponse(#"{"id":"key","name":"ci","scopes":["projects.create"],"token":"once"}"#),
            jsonResponse("null", status: 204),
            jsonResponse(#"{"default":"model","models":[{"id":"model","name":"Model"}]}"#),
            jsonResponse(#"[{"id":"thread","running":false,"preview":"hi","at":"t"}]"#),
            jsonResponse(#"{"id":"thread"}"#),
            jsonResponse(#"{"id":"thread"}"#, status: 202),
            jsonResponse(#"{"id":"thread","running":true,"project_ref":"abcdefghijklmnopqrst","messages":[]}"#),
            jsonResponse(#"{"id":"thread","running":false,"project_ref":"abcdefghijklmnopqrst","messages":[{"role":"assistant","content":"done"}]}"#),
        ])
        let cluster = ClusterStore(client: client)
        _ = try await cluster.load()
        _ = try await cluster.rename(name: "renamed")
        _ = try await cluster.email()
        _ = try await cluster.saveEmail(host: "smtp.example", port: 587, username: "", password: "secret", fromAddress: "a@example.com", tls: "starttls")

        let operators = OperatorsStore(client: client)
        _ = try await operators.list()
        _ = try await operators.create(name: "New", email: "new@example.com", password: "scripted-pass", platformAdmin: false)
        try await operators.setPlatformAdmin(id: "new", enabled: true)
        try await operators.grant(id: "new", pref: "abcdefghijklmnopqrst", role: "owner")
        try await operators.revoke(id: "new", pref: "abcdefghijklmnopqrst")
        do {
            try await operators.delete(id: "self", selfID: "self")
            XCTFail("self delete should be refused")
        } catch let error as ConsoleError {
            XCTAssertEqual(error.message, "you cannot delete yourself")
        }
        try await operators.delete(id: "new", selfID: "self")

        let keys = ServiceKeysStore(client: client)
        _ = try await keys.list()
        _ = try await keys.create(name: "ci", scopes: ConsoleKeyScope.all)
        XCTAssertEqual(keys.hold?.fields.first?.value, "once")
        keys.acknowledgeSecrets()
        XCTAssertNil(keys.hold)
        try await keys.revoke(id: "key")

        let agent = AgentStore(client: client)
        _ = try await agent.models()
        _ = try await agent.threads()
        _ = try await agent.createThread(projectRef: "abcdefghijklmnopqrst")
        _ = try await agent.post(id: "thread", text: "hello", model: "model")
        _ = try await agent.poll(id: "thread", maxPolls: 3)

        let requests = await transport.requests
        XCTAssertEqual(ConsoleKeyScope.all.count, 7)
        let created = try objectBody(requests[11].body)
        XCTAssertEqual((created["scopes"] as? [String])?.count, 7)
        let thread = try objectBody(requests[15].body)
        XCTAssertEqual(thread["project_ref"] as? String, "abcdefghijklmnopqrst")
        XCTAssertFalse(requests.contains { $0.url.path.hasSuffix("/run") })
        XCTAssertEqual(requests.filter { $0.url.path == "/console/v1/agent/threads/thread" }.count, 2)
        let selfDeleteSkipped = requests.filter { $0.method == "DELETE" && $0.url.path == "/console/v1/operators/self" }
        XCTAssertTrue(selfDeleteSkipped.isEmpty)
        XCTAssertTrue(requests.contains { $0.method == "DELETE" && $0.url.path == "/console/v1/operators/new" })
    }
}
