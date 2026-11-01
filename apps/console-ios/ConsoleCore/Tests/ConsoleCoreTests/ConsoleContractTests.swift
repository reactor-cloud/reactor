import Foundation
import XCTest
@testable import ConsoleCore

final class ConsoleContractTests: XCTestCase {
    func testConsoleWalk() async throws {
        let env = ProcessInfo.processInfo.environment
        guard let rawURL = env["REACTOR_CONSOLE_URL"], !rawURL.isEmpty else {
            throw XCTSkip("REACTOR_CONSOLE_URL is required")
        }
        let email = try required("REACTOR_CONSOLE_EMAIL")
        let password = try required("REACTOR_CONSOLE_PASSWORD")
        let recovery = try required("REACTOR_CONSOLE_RECOVERY")
        let url = trimmedClusterURL(rawURL)
        let transport = URLSessionTransport()
        let machine = SessionMachine(transport: transport, vault: MemoryVault())
        let now = Date()
        let passwordPhase = try await machine.signIn(url: url, name: "contract", email: email, password: password, now: now)
        let record: ClusterRecord
        switch passwordPhase {
        case .session(let session):
            record = session
        case .otp:
            let done = try await machine.submitRecovery(url: url, code: recovery, now: now)
            guard case .session(let session) = done else {
                return XCTFail("recovery did not open a session: \(done)")
            }
            record = session
        default:
            return XCTFail("login did not reach a session: \(passwordPhase)")
        }

        guard let base = URL(string: record.url) else {
            return XCTFail("bad cluster url")
        }
        let client = ConsoleClient(baseURL: base, token: record.accessToken, transport: transport)
        let projects = ProjectsStore(client: client)
        let created = try await projects.create(name: "Contract \(UUID().uuidString.prefix(8))")
        guard let pref = created["ref"]?.stringValue else {
            return XCTFail("project ref missing")
        }
        XCTAssertNotNil(projects.hold)

        let data = DataStore(client: client)
        _ = try await data.createTable(pref: pref, schema: nil, name: "notes", columns: [("title", "text")])
        _ = try await data.insert(pref: pref, table: "notes", schema: nil, values: ["title": .string("hello")])
        let rows = try await data.rows(pref: pref, table: "notes", schema: nil, query: "hello", offset: 0)
        XCTAssertGreaterThan(rows["total"]?.intValue ?? 0, 0)

        let sql = SqlStore(client: client, role: .owner, readOnly: true)
        let selected = try await sql.run(pref: pref, sql: "SELECT 1")
        guard case .value = selected else { return XCTFail("select should not need confirmation") }

        let storage = StorageStore(client: client)
        _ = try await storage.createBucket(pref: pref, name: "files", isPublic: false)
        let uploaded = try await storage.upload(pref: pref, path: "files/hello.txt", bytes: Data("hello".utf8), contentType: "text/plain")
        XCTAssertNotNil(uploaded["key"]?.stringValue)

        let functions = FunctionsStore(client: client)
        _ = try await functions.deploy(pref: pref, name: "ping", zip: storedZip(name: "index.ts", contents: Data("process.stdout.write('ok')".utf8)), promote: true)
        let invoked = try await functions.invoke(pref: pref, name: "ping", version: nil, payload: Data("{}".utf8))
        XCTAssertNotNil(invoked["status"]?.intValue)

        let sites = SitesStore(client: client)
        _ = try await sites.setEnv(pref: pref, key: "VISIBLE", value: "shown", visibility: "visible")
        let envRows = try await sites.env(pref: pref)
        let visible = envRows.arrayValue?.contains { $0["key"]?.stringValue == "VISIBLE" } ?? false
        XCTAssertTrue(visible)

        let cluster = try await ClusterStore(client: client).load()
        XCTAssertNotNil(cluster["name"]?.stringValue)
        try await projects.delete(pref: pref)
    }

    private func required(_ name: String) throws -> String {
        guard let value = ProcessInfo.processInfo.environment[name], !value.isEmpty else {
            throw XCTSkip("\(name) is required")
        }
        return value
    }
}

private func storedZip(name: String, contents: Data) -> Data {
    let nameData = Data(name.utf8)
    let crc = crc32(contents)
    var local = Data()
    local.append(contentsOf: [0x50, 0x4b, 0x03, 0x04])
    local.appendU16(20)
    local.appendU16(0)
    local.appendU16(0)
    local.appendU16(0)
    local.appendU16(0)
    local.appendU32(crc)
    local.appendU32(UInt32(contents.count))
    local.appendU32(UInt32(contents.count))
    local.appendU16(UInt16(nameData.count))
    local.appendU16(0)
    local.append(nameData)
    local.append(contents)
    let localCount = local.count
    var central = Data()
    central.append(contentsOf: [0x50, 0x4b, 0x01, 0x02])
    central.appendU16(20)
    central.appendU16(20)
    central.appendU16(0)
    central.appendU16(0)
    central.appendU16(0)
    central.appendU16(0)
    central.appendU32(crc)
    central.appendU32(UInt32(contents.count))
    central.appendU32(UInt32(contents.count))
    central.appendU16(UInt16(nameData.count))
    central.appendU16(0)
    central.appendU16(0)
    central.appendU16(0)
    central.appendU16(0)
    central.appendU32(0)
    central.appendU32(0)
    central.append(nameData)
    var end = Data()
    end.append(contentsOf: [0x50, 0x4b, 0x05, 0x06])
    end.appendU16(0)
    end.appendU16(0)
    end.appendU16(1)
    end.appendU16(1)
    end.appendU32(UInt32(central.count))
    end.appendU32(UInt32(localCount))
    end.appendU16(0)
    var zip = local
    zip.append(central)
    zip.append(end)
    return zip
}

private func crc32(_ data: Data) -> UInt32 {
    var crc: UInt32 = 0xffff_ffff
    for byte in data {
        crc ^= UInt32(byte)
        for _ in 0..<8 {
            let mask = (crc & 1) == 1 ? UInt32(0xedb8_8320) : 0
            crc = (crc >> 1) ^ mask
        }
    }
    return crc ^ 0xffff_ffff
}

private extension Data {
    mutating func appendU16(_ value: UInt16) {
        append(contentsOf: [UInt8(value & 0xff), UInt8(value >> 8)])
    }

    mutating func appendU32(_ value: UInt32) {
        appendU16(UInt16(value & 0xffff))
        appendU16(UInt16(value >> 16))
    }
}
