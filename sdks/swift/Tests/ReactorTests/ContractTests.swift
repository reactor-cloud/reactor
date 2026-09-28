import XCTest
@testable import Reactor

final class ContractTests: XCTestCase {
    func testTodosProject() async throws {
        let url = ProcessInfo.processInfo.environment["REACTOR_URL"] ?? "http://127.0.0.1:18000"
        let anonKey = ProcessInfo.processInfo.environment["REACTOR_ANON_KEY"] ?? ""
        XCTAssertFalse(anonKey.isEmpty, "REACTOR_ANON_KEY is required")
        if anonKey.isEmpty { return }

        let email = "sdk-\(UUID().uuidString.lowercased())@example.com"
        let otherEmail = "sdk-\(UUID().uuidString.lowercased())@example.com"
        let password = "password123"
        let client = ReactorClient(url: url, anonKey: anonKey)

        let session = try await client.auth.signUp(email: email, password: password)
        XCTAssertEqual(session.user.email, email)
        let user = try await client.auth.getUser()
        XCTAssertEqual(user.email, email)

        let title = "todo-\(UUID().uuidString.lowercased())"
        let inserted = try await client.from("todos")
            .insert(["title": .string(title), "user_id": .string(session.user.id)])
            .select()
            .execute()
            .array()!
        XCTAssertEqual(inserted.count, 1)
        let id = inserted[0]["id"]!.string()!

        let listed = try await client.from("todos")
            .select()
            .eq("user_id", session.user.id)
            .order("created_at", ascending: false)
            .execute()
            .array()!
        XCTAssertTrue(listed.contains { $0["id"]?.string() == id && $0["title"]?.string() == title })

        let renamed = "\(title)-edited"
        _ = try await client.from("todos").update(["title": .string(renamed)]).eq("id", id).select().execute()
        let after = try await client.from("todos").select().eq("id", id).execute().array()!
        XCTAssertEqual(after.first?["title"]?.string(), renamed)

        let path = "note-\(UUID().uuidString.lowercased()).txt"
        let bytes = Data("reactor-sdk".utf8)
        try await client.storage.from("files").upload(path: path, data: bytes)
        let downloaded = try await client.storage.from("files").download(path: path)
        XCTAssertEqual(downloaded, bytes)

        let ping = try await client.functions.invoke("ping")
        XCTAssertEqual(ping["ok"]?.bool(), true)

        let other = ReactorClient(url: url, anonKey: anonKey)
        _ = try await other.auth.signUp(email: otherEmail, password: password)
        let hidden = try await other.from("todos").select().eq("id", id).execute().array()!
        XCTAssertEqual(hidden.count, 0)

        let anon = ReactorClient(url: url, anonKey: anonKey)
        do {
            _ = try await anon.from("todos").select().execute()
            XCTFail("anon read should fail")
        } catch {
            XCTAssertTrue(error is ReactorError)
        }

        _ = try await client.from("todos").delete().eq("id", id).select().execute()
        let gone = try await client.from("todos").select().eq("id", id).execute().array()!
        XCTAssertEqual(gone.count, 0)

        let previous = session.refreshToken
        let refreshed = try await client.auth.refreshSession()
        XCTAssertNotEqual(refreshed.refreshToken, previous)
        let stale = try await postRefresh(url: url, anonKey: anonKey, token: previous)
        XCTAssertEqual(stale, 401)

        let current = refreshed.refreshToken
        try await client.auth.signOut()
        let revoked = try await postRefresh(url: url, anonKey: anonKey, token: current)
        XCTAssertEqual(revoked, 401)

        let again = try await client.auth.signInWithPassword(email: email, password: password)
        XCTAssertEqual(again.user.email, email)
    }
}

private func postRefresh(url: String, anonKey: String, token: String) async throws -> Int {
    var request = URLRequest(url: URL(string: "\(url)/auth/v1/token")!)
    request.httpMethod = "POST"
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    request.setValue("Bearer \(anonKey)", forHTTPHeaderField: "Authorization")
    request.httpBody = try JSONSerialization.data(withJSONObject: ["refresh_token": token])
    request.timeoutInterval = 20
    let config = URLSessionConfiguration.ephemeral
    config.timeoutIntervalForRequest = 20
    let (_, response) = try await URLSession(configuration: config).data(for: request)
    return (response as? HTTPURLResponse)?.statusCode ?? 0
}
