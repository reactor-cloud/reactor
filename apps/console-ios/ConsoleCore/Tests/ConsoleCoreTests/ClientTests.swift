import XCTest
@testable import ConsoleCore

final class ClientTests: XCTestCase {
    func testBearerAttachedUnlessCallerSetOne() async throws {
        let (transport, client) = scripted([
            jsonResponse(#"{"ok":true}"#),
            jsonResponse(#"{"ok":true}"#),
        ])
        _ = try await client.json(method: "GET", path: "/console/v1/me")
        _ = try await client.json(method: "GET", path: "/console/v1/me", bearer: .token("other-token"))
        let requests = await transport.requests
        XCTAssertEqual(requests[0].headers["authorization"], "Bearer session-token")
        XCTAssertEqual(requests[1].headers["authorization"], "Bearer other-token")
    }

    func testJsonErrorSurfacesErrorField() async throws {
        let (_, client) = scripted([jsonResponse(#"{"error":"invalid email or password"}"#, status: 401)])
        do {
            _ = try await client.json(method: "POST", path: "/console/v1/login")
            XCTFail("expected an error")
        } catch let error as ConsoleError {
            XCTAssertEqual(error.status, 401)
            XCTAssertEqual(error.message, "invalid email or password")
        }
    }

    func testSql409ReturnsWarnings() async throws {
        let (transport, client) = scripted([
            jsonResponse(#"{"error":"confirmation required","warnings":["drop_table","delete_without_where"]}"#, status: 409),
        ])
        let outcome = try await client.runSql(pref: "abcdefghijklmnopqrst", sql: "DROP TABLE todos", readOnly: false, confirm: false)
        guard case .confirmation(let warnings) = outcome else {
            return XCTFail("expected confirmation")
        }
        XCTAssertEqual(warnings, ["drop_table", "delete_without_where"])
        let body = try objectBody((await transport.requests)[0].body)
        XCTAssertEqual(body["confirm"] as? Bool, false)
        XCTAssertEqual(body["read_only"] as? Bool, false)
    }

    func testZipUploadSetsContentTypeAndRawBody() async throws {
        let (transport, client) = scripted([jsonResponse(#"{"name":"ping","version":1,"promoted":true}"#)])
        let bytes = Data("zip-bytes".utf8)
        _ = try await client.uploadZip(pref: "abcdefghijklmnopqrst", name: "ping", bytes: bytes, promote: true)
        let request = await transport.requests[0]
        XCTAssertEqual(request.headers["content-type"], "application/zip")
        XCTAssertEqual(request.body, bytes)
        XCTAssertEqual(request.url.path, "/console/v1/projects/abcdefghijklmnopqrst/functions/ping")
        XCTAssertEqual(request.url.query, "promote=true")
        XCTAssertEqual(request.headers["authorization"], "Bearer session-token")
    }
}
