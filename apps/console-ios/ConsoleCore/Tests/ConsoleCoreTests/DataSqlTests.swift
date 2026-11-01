import XCTest
@testable import ConsoleCore

final class DataSqlTests: XCTestCase {
    func testDataGrid() async throws {
        let (transport, client) = scripted([
            jsonResponse(#"[{"name":"proj_abcdefghijklmnopqrst","tables":[]}]"#),
            jsonResponse(#"[{"name":"todos","primary_key":"id","columns":[{"name":"title","type":"text"}]}]"#),
            jsonResponse(#"{"rows":[],"total":0}"#),
            jsonResponse("null", status: 201),
            jsonResponse("null", status: 201),
            jsonResponse("null", status: 204),
            jsonResponse("null", status: 204),
            jsonResponse("null", status: 201),
        ])
        let store = DataStore(client: client)
        let pref = "abcdefghijklmnopqrst"
        _ = try await store.schemas(pref: pref)
        _ = try await store.schema(pref: pref, schema: "proj_abcdefghijklmnopqrst")
        _ = try await store.rows(pref: pref, table: "todos", schema: "proj_abcdefghijklmnopqrst", query: "hello", offset: 0)
        _ = try await store.createTable(pref: pref, schema: "proj_abcdefghijklmnopqrst", name: "todos", columns: [("title", "text")])
        _ = try await store.insert(pref: pref, table: "todos", schema: nil, values: ["title": .string("hello")])
        _ = try await store.updateCell(pref: pref, table: "todos", schema: nil, pk: .string("pk"), column: "title", value: .string("edited"))
        try await store.deleteRow(pref: pref, table: "todos", schema: nil, pk: .string("pk"))
        _ = try await store.duplicate(pref: pref, table: "todos", schema: nil, primaryKey: "id", values: ["id": .string("pk"), "title": .string("copy")])

        let requests = await transport.requests
        XCTAssertEqual(requests[1].url.query, "schema=proj_abcdefghijklmnopqrst")
        XCTAssertTrue(requests[2].url.query?.contains("limit=50") == true)
        XCTAssertTrue(requests[2].url.query?.contains("offset=0") == true)
        XCTAssertTrue(requests[2].url.query?.contains("q=hello") == true)
        let created = try objectBody(requests[3].body)
        XCTAssertEqual(created["name"] as? String, "todos")
        let inserted = try objectBody(requests[7].body)
        let values = inserted["values"] as? [String: Any]
        XCTAssertEqual(values?["title"] as? String, "copy")
        XCTAssertNil(values?["id"])
        XCTAssertEqual(requests[5].url.path, "/console/v1/projects/\(pref)/tables/todos/rows")
        XCTAssertEqual(requests[6].method, "DELETE")
    }

    func testSqlConfirmMigrationAndRevert() async throws {
        let (transport, client) = scripted([
            jsonResponse(#"{"error":"confirmation required","warnings":["drop_table"]}"#, status: 409),
            jsonResponse(#"{"results":[{"command":"DROP","columns":[],"rows":[],"truncated":false,"rows_affected":0}]}"#),
            jsonResponse(#"{"ok":true}"#),
            jsonResponse(#"[{"version":"0004_change.sql","status":"applied","revertable":true}]"#),
            jsonResponse(#"{"ok":true,"version":"0004_change.sql"}"#),
        ])
        let sql = SqlStore(client: client, role: .admin, readOnly: true)
        let pref = "abcdefghijklmnopqrst"
        let pending = try await sql.run(pref: pref, sql: "DROP TABLE todos")
        guard case .confirmation(let warnings) = pending else { return XCTFail("expected confirm") }
        XCTAssertEqual(warnings, ["drop_table"])
        _ = try await sql.confirm(pref: pref, sql: "DROP TABLE todos")
        _ = try await sql.migrate(pref: pref, version: "0004_change.sql", sql: "CREATE TABLE todos (id uuid)", downSQL: "DROP TABLE todos")
        _ = try await sql.migrations(pref: pref)
        _ = try await sql.revert(
            pref: pref,
            migration: MigrationRecord(version: "0004_change.sql", status: "applied", revertable: true)
        )
        let requests = await transport.requests
        let first = try objectBody(requests[0].body)
        XCTAssertEqual(first["read_only"] as? Bool, true)
        XCTAssertEqual(first["confirm"] as? Bool, false)
        let second = try objectBody(requests[1].body)
        XCTAssertEqual(second["confirm"] as? Bool, true)
        let migration = try objectBody(requests[2].body)
        XCTAssertEqual(migration["mode"] as? String, "migration")
        XCTAssertEqual(migration["version"] as? String, "0004_change.sql")
        XCTAssertEqual(migration["down_sql"] as? String, "DROP TABLE todos")
        XCTAssertEqual(requests[4].url.path, "/console/v1/projects/\(pref)/migrations/0004_change.sql/revert")
    }

    func testDeveloperCannotRunAndUnrevertableIsSkipped() async throws {
        let (transport, client) = scripted([])
        let sql = SqlStore(client: client, role: .developer)
        do {
            _ = try await sql.run(pref: "abcdefghijklmnopqrst", sql: "SELECT 1")
            XCTFail("developer run should fail")
        } catch let error as ConsoleError {
            XCTAssertEqual(error.status, 403)
        }
        let admin = SqlStore(client: client, role: .admin)
        do {
            _ = try await admin.revert(
                pref: "abcdefghijklmnopqrst",
                migration: MigrationRecord(version: "0001_init.sql", status: "applied", revertable: false)
            )
            XCTFail("revert should fail closed")
        } catch let error as ConsoleError {
            XCTAssertEqual(error.status, 400)
        }
        let requests = await transport.requests
        XCTAssertTrue(requests.isEmpty)
    }
}
