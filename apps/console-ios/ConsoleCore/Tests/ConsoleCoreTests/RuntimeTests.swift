import XCTest
@testable import ConsoleCore

final class RuntimeTests: XCTestCase {
    func testStorageFunctionsSitesAndQueue() async throws {
        let (transport, client) = scripted([
            jsonResponse(#"[{"key":"abcdefghijklmnopqrst/note.txt","size":4}]"#),
            jsonResponse(#"[{"id":"files","public":false}]"#),
            jsonResponse(#"{"name":"files","public":true}"#),
            jsonResponse(#"{"name":"files","public":false}"#),
            jsonResponse(#"{"url":"https://cdn.example/put","key":"abcdefghijklmnopqrst/note.txt"}"#),
            jsonResponse("", status: 200),
            jsonResponse(#"{"url":"https://cdn.example/get","public":false}"#),
            jsonResponse("null", status: 204),
            jsonResponse(#"[{"name":"ping","version":1,"live":true,"pinned":false,"created_at":"t"}]"#),
            jsonResponse(#"{"name":"ping","version":2,"promoted":true}"#),
            jsonResponse(#"{"name":"ping","version":1,"pinned":true}"#),
            jsonResponse(#"{"name":"ping","pinned":false}"#),
            jsonResponse(#"{"hours":[1],"points":[{"ok":1,"warn":0,"error":0}]}"#),
            jsonResponse(#"[{"key":"TOKEN","updated_at":"t"}]"#),
            jsonResponse(#"{"key":"TOKEN"}"#),
            jsonResponse("null", status: 204),
            jsonResponse(#"{"status":200,"body":"ok"}"#),
            jsonResponse(#"{"files":[],"domains":[],"url":"https://abcdefghijklmnopqrst.apps.localhost"}"#),
            jsonResponse(#"{"hours":[],"points":[]}"#),
            jsonResponse(#"[{"id":"dep","status":"ready","error":null,"file_count":1,"created_at":"t"}]"#),
            jsonResponse(#"{"id":"dep","status":"ready","error":null,"file_count":1,"created_at":"t","files":["index.html"]}"#),
            jsonResponse(#"[{"key":"PUBLIC","visibility":"visible","value":"shown"}]"#),
            jsonResponse(#"{"key":"PUBLIC","visibility":"visible"}"#),
            jsonResponse(#"{"key":"SECRET","visibility":"secret"}"#),
            jsonResponse("null", status: 204),
            jsonResponse(#"[{"host":"app.example","verified_at":null,"txt_name":"_reactor-verify","txt_value":"token"}]"#),
            jsonResponse(#"{"host":"app.example","txt_name":"_reactor-verify","txt_value":"token"}"#),
            jsonResponse("null", status: 204),
            jsonResponse("null", status: 204),
            jsonResponse(#"[{"name":"jobs","created_at":"t"}]"#),
            jsonResponse(#"{"name":"jobs"}"#),
            jsonResponse(#"{"msg_id":1}"#),
            jsonResponse(#"[{"msg_id":1,"message":{"ok":true},"vt":"t","read_ct":0,"enqueued_at":"t"}]"#),
            jsonResponse(#"[{"function_name":"ping","vt_secs":30,"qty":1,"max_reads":5}]"#),
        ])
        let pref = "abcdefghijklmnopqrst"
        let storage = StorageStore(client: client)
        _ = try await storage.objects(pref: pref)
        _ = try await storage.buckets(pref: pref)
        _ = try await storage.createBucket(pref: pref, name: "files", isPublic: true)
        _ = try await storage.updateBucket(pref: pref, name: "files", isPublic: false)
        _ = try await storage.upload(pref: pref, path: "note.txt", bytes: Data("note".utf8), contentType: "text/plain")
        _ = try await storage.objectURL(pref: pref, key: "abcdefghijklmnopqrst/note.txt")
        try await storage.deleteObject(pref: pref, key: "abcdefghijklmnopqrst/note.txt")

        let functions = FunctionsStore(client: client)
        _ = try await functions.list(pref: pref)
        _ = try await functions.deploy(pref: pref, name: "ping", zip: Data("zip".utf8), promote: false)
        _ = try await functions.promote(pref: pref, name: "ping", version: 1)
        _ = try await functions.demote(pref: pref, name: "ping")
        _ = try await functions.stats(pref: pref, name: "ping")
        _ = try await functions.env(pref: pref, name: "ping")
        _ = try await functions.setEnv(pref: pref, name: "ping", key: "TOKEN", value: "secret")
        try await functions.deleteEnv(pref: pref, name: "ping", key: "TOKEN")
        _ = try await functions.invoke(pref: pref, name: "ping", version: 2, payload: Data("{}".utf8))

        let sites = SitesStore(client: client)
        _ = try await sites.overview(pref: pref)
        _ = try await sites.stats(pref: pref)
        _ = try await sites.deployments(pref: pref)
        _ = try await sites.deployment(pref: pref, id: "dep")
        _ = try await sites.env(pref: pref)
        _ = try await sites.setEnv(pref: pref, key: "PUBLIC", value: "shown", visibility: "visible")
        _ = try await sites.setEnv(pref: pref, key: "SECRET", value: "hidden", visibility: "secret")
        try await sites.deleteEnv(pref: pref, key: "SECRET")
        _ = try await sites.domains(pref: pref)
        _ = try await sites.addDomain(pref: pref, host: "app.example")
        try await sites.verifyDomain(pref: pref, host: "app.example")
        try await sites.deleteDomain(pref: pref, host: "app.example")

        let queue = QueueStore(client: client, enabled: true)
        _ = try await queue.list(pref: pref)
        _ = try await queue.create(pref: pref, name: "jobs")
        _ = try await queue.send(pref: pref, name: "jobs", message: .object(["ok": .bool(true)]), delaySecs: 0)
        _ = try await queue.peek(pref: pref, name: "jobs")
        _ = try await queue.subscriptions(pref: pref, name: "jobs")

        let requests = await transport.requests
        let uploadPut = requests[5]
        XCTAssertEqual(uploadPut.method, "PUT")
        XCTAssertEqual(uploadPut.url.absoluteString, "https://cdn.example/put")
        XCTAssertNil(uploadPut.headers["authorization"])
        XCTAssertEqual(uploadPut.headers["content-type"], "text/plain")
        XCTAssertEqual(requests[4].headers["authorization"], "Bearer session-token")
        let zip = requests[9]
        XCTAssertEqual(zip.headers["content-type"], "application/zip")
        XCTAssertEqual(zip.url.query, "promote=false")
        let env = try objectBody(requests[14].body)
        XCTAssertEqual(env["value"] as? String, "secret")
        let visible = try objectBody(requests[22].body)
        XCTAssertEqual(visible["visibility"] as? String, "visible")
        let secret = try objectBody(requests[23].body)
        XCTAssertEqual(secret["visibility"] as? String, "secret")
        XCTAssertEqual(requests[31].url.path, "/console/v1/projects/\(pref)/queues/jobs/send")
        XCTAssertFalse(requests.contains { $0.url.path.contains("/run") })
    }

    func testQueueIsAbsentWhenTheExtensionIsOff() async throws {
        let (transport, client) = scripted([
            jsonResponse(#"[{"name":"other","routes":[]}]"#),
        ])
        let enabled = try await ExtensionsStore(client: client).queueEnabled()
        XCTAssertFalse(enabled)
        let queue = QueueStore(client: client, enabled: false)
        do {
            _ = try await queue.list(pref: "abcdefghijklmnopqrst")
            XCTFail("queue should stay closed")
        } catch is QueueUnavailable {
        }
        let requests = await transport.requests
        XCTAssertEqual(requests.map(\.url.path), ["/console/v1/extensions"])
    }
}
