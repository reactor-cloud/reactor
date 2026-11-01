import Foundation

func prefPath(_ pref: String, _ rest: String = "") -> String {
    "/console/v1/projects/\(PathEncode.segment(pref))\(rest)"
}

func holdKeys(_ title: String, _ body: JSONValue, names: [String]) -> SecretHold? {
    let fields = names.compactMap { name -> SecretField? in
        guard let value = body[name]?.stringValue else { return nil }
        return SecretField(name: name, value: value)
    }
    if fields.isEmpty { return nil }
    return SecretHold(title: title, fields: fields)
}

public final class ProjectsStore: @unchecked Sendable {
    public var hold: SecretHold?
    private let client: ConsoleClient

    public init(client: ConsoleClient) {
        self.client = client
    }

    public func load() async throws -> JSONValue {
        try await client.json(method: "GET", path: "/console/v1/projects")
    }

    public func loadMe() async throws -> JSONValue {
        try await client.json(method: "GET", path: "/console/v1/me")
    }

    public func create(name: String) async throws -> JSONValue {
        let body = try await client.json(
            method: "POST",
            path: "/console/v1/projects",
            json: .object(["name": .string(name)])
        )
        hold = holdKeys("Project keys", body, names: ["anon_key", "service_key"])
        return body
    }

    public func rename(pref: String, name: String) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref),
            json: .object(["name": .string(name)])
        )
    }

    public func delete(pref: String) async throws {
        _ = try await client.json(method: "DELETE", path: prefPath(pref))
    }

    public func acknowledgeSecrets() {
        hold = nil
    }
}

public final class OverviewStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func load(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/overview"))
    }
}

public final class KeysStore: @unchecked Sendable {
    public var hold: SecretHold?
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func rotate(pref: String) async throws -> JSONValue {
        let body = try await client.json(method: "POST", path: prefPath(pref, "/keys"))
        hold = holdKeys("Rotated keys", body, names: ["anon_key", "service_key"])
        return body
    }

    public func acknowledgeSecrets() { hold = nil }
}

public final class TeamStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func list(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/members"))
    }

    public func add(pref: String, email: String, name: String, password: String, role: String) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/members"),
            json: .object([
                "email": .string(email),
                "name": .string(name),
                "password": .string(password),
                "role": .string(role),
            ])
        )
    }
}

public final class LogsStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func search(pref: String, query: String) async throws -> JSONValue {
        var items: [URLQueryItem] = []
        if !query.isEmpty { items.append(URLQueryItem(name: "q", value: query)) }
        return try await client.json(method: "GET", path: prefPath(pref, "/logs"), query: items)
    }
}

public final class ExtensionsStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func queueEnabled() async throws -> Bool {
        let body = try await client.json(method: "GET", path: "/console/v1/extensions")
        return body.arrayValue?.contains { $0["name"]?.stringValue == "queue" } ?? false
    }
}

public final class DataStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func schemas(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/schemas"))
    }

    public func schema(pref: String, schema: String?) async throws -> JSONValue {
        var query: [URLQueryItem] = []
        if let schema, !schema.isEmpty { query.append(URLQueryItem(name: "schema", value: schema)) }
        return try await client.json(method: "GET", path: prefPath(pref, "/schema"), query: query)
    }

    public func rows(pref: String, table: String, schema: String?, query: String, offset: Int) async throws -> JSONValue {
        var items = [
            URLQueryItem(name: "limit", value: "50"),
            URLQueryItem(name: "offset", value: String(offset)),
        ]
        if let schema, !schema.isEmpty { items.append(URLQueryItem(name: "schema", value: schema)) }
        if !query.isEmpty { items.append(URLQueryItem(name: "q", value: query)) }
        return try await client.json(
            method: "GET",
            path: prefPath(pref, "/tables/\(PathEncode.segment(table))"),
            query: items
        )
    }

    public func createTable(pref: String, schema: String?, name: String, columns: [(String, String)]) async throws -> JSONValue {
        let cols = columns.map { JSONValue.object(["name": .string($0.0), "type": .string($0.1)]) }
        var query: [URLQueryItem] = []
        if let schema, !schema.isEmpty { query.append(URLQueryItem(name: "schema", value: schema)) }
        return try await client.json(
            method: "POST",
            path: prefPath(pref, "/tables"),
            query: query,
            json: .object(["name": .string(name), "columns": .array(cols)])
        )
    }

    public func insert(pref: String, table: String, schema: String?, values: [String: JSONValue]) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/tables/\(PathEncode.segment(table))"),
            query: schemaQuery(schema),
            json: .object(["values": .object(values)])
        )
    }

    public func updateCell(pref: String, table: String, schema: String?, pk: JSONValue, column: String, value: JSONValue) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/tables/\(PathEncode.segment(table))/rows"),
            query: schemaQuery(schema),
            json: .object(["pk": pk, "column": .string(column), "value": value])
        )
    }

    public func deleteRow(pref: String, table: String, schema: String?, pk: JSONValue) async throws {
        _ = try await client.json(
            method: "DELETE",
            path: prefPath(pref, "/tables/\(PathEncode.segment(table))/rows"),
            query: schemaQuery(schema),
            json: .object(["pk": pk])
        )
    }

    public func duplicate(pref: String, table: String, schema: String?, primaryKey: String, values: [String: JSONValue]) async throws -> JSONValue {
        var copy = values
        copy.removeValue(forKey: primaryKey)
        return try await insert(pref: pref, table: table, schema: schema, values: copy)
    }

    private func schemaQuery(_ schema: String?) -> [URLQueryItem] {
        guard let schema, !schema.isEmpty else { return [] }
        return [URLQueryItem(name: "schema", value: schema)]
    }
}

public final class SqlStore: @unchecked Sendable {
    public var role: ProjectRole
    public var readOnly: Bool
    private let client: ConsoleClient

    public init(client: ConsoleClient, role: ProjectRole, readOnly: Bool = true) {
        self.client = client
        self.role = role
        self.readOnly = readOnly
    }

    public func run(pref: String, sql: String) async throws -> SqlOutcome {
        try requireWriter()
        return try await client.runSql(pref: pref, sql: sql, readOnly: readOnly, confirm: false)
    }

    public func confirm(pref: String, sql: String) async throws -> SqlOutcome {
        try requireWriter()
        return try await client.runSql(pref: pref, sql: sql, readOnly: readOnly, confirm: true)
    }

    public func migrate(pref: String, version: String, sql: String, downSQL: String?) async throws -> JSONValue {
        try requireWriter()
        let outcome = try await client.runSql(
            pref: pref,
            sql: sql,
            readOnly: false,
            confirm: true,
            mode: "migration",
            version: version,
            downSQL: downSQL
        )
        switch outcome {
        case .value(let value):
            return value
        case .confirmation(let warnings):
            throw ConsoleError(status: 409, message: warnings.joined(separator: ", "))
        }
    }

    public func migrations(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/migrations"))
    }

    public func revert(pref: String, migration: MigrationRecord) async throws -> JSONValue {
        try requireWriter()
        guard migration.revertable && migration.status == "applied" else {
            throw ConsoleError(status: 400, message: "migration cannot be reverted")
        }
        return try await client.json(
            method: "POST",
            path: prefPath(pref, "/migrations/\(PathEncode.segment(migration.version))/revert")
        )
    }

    public func resetDatabase(pref: String) async throws -> JSONValue {
        try requireWriter()
        return try await client.json(method: "POST", path: prefPath(pref, "/database/reset"))
    }

    private func requireWriter() throws {
        if role == .developer {
            throw ConsoleError(status: 403, message: "Owners and admins can run SQL.")
        }
    }
}

public final class AuthAdminStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func users(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/users"))
    }

    public func user(pref: String, id: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/users/\(PathEncode.segment(id))"))
    }

    public func setPassword(pref: String, id: String, password: String) async throws {
        _ = try await client.json(
            method: "POST",
            path: prefPath(pref, "/users/\(PathEncode.segment(id))/password"),
            json: .object(["password": .string(password)])
        )
    }

    public func deleteUser(pref: String, id: String) async throws {
        _ = try await client.json(method: "DELETE", path: prefPath(pref, "/users/\(PathEncode.segment(id))"))
    }

    public func authSettings(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/auth"))
    }

    public func updateAuth(pref: String, requireEmailVerification: Bool, requireMFA: Bool) async throws -> JSONValue {
        try await client.json(
            method: "PUT",
            path: prefPath(pref, "/auth"),
            json: .object([
                "require_email_verification": .bool(requireEmailVerification),
                "require_mfa": .bool(requireMFA),
            ])
        )
    }

    public func providers(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/auth/providers"))
    }

    public func upsertProvider(
        pref: String,
        provider: String,
        clientID: String,
        clientSecret: String?,
        redirects: [String],
        extra: [String: String]
    ) async throws -> JSONValue {
        var fields: [String: JSONValue] = [
            "provider": .string(provider),
            "client_id": .string(clientID),
            "redirects": .array(redirects.map { .string($0) }),
            "enabled": .bool(true),
            "extra": .object(extra.mapValues { .string($0) }),
        ]
        if let clientSecret, !clientSecret.isEmpty {
            fields["client_secret"] = .string(clientSecret)
        }
        return try await client.json(method: "POST", path: prefPath(pref, "/auth/providers"), json: .object(fields))
    }

    public func deleteProvider(pref: String, provider: String) async throws {
        _ = try await client.json(
            method: "DELETE",
            path: prefPath(pref, "/auth/providers/\(PathEncode.segment(provider))")
        )
    }

    public func email(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/email"))
    }

    public func saveEmail(
        pref: String,
        host: String,
        port: Int,
        username: String,
        password: String?,
        fromAddress: String,
        tls: String,
        linkBase: String
    ) async throws -> JSONValue {
        var fields: [String: JSONValue] = [
            "host": .string(host),
            "port": .int(port),
            "username": .string(username),
            "from_address": .string(fromAddress),
            "tls": .string(tls),
            "link_base": .string(linkBase),
        ]
        if let password, !password.isEmpty { fields["password"] = .string(password) }
        return try await client.json(method: "PUT", path: prefPath(pref, "/email"), json: .object(fields))
    }

    public func setClusterSMTP(pref: String, enabled: Bool) async throws -> JSONValue {
        try await client.json(
            method: "PUT",
            path: prefPath(pref, "/email/cluster"),
            json: .object(["enabled": .bool(enabled)])
        )
    }

    public func testEmail(pref: String, to: String) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/email/test"),
            json: .object(["to": .string(to)])
        )
    }

    public func templates(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/email/templates"))
    }

    public func template(pref: String, name: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/email/templates/\(PathEncode.segment(name))"))
    }

    public func saveTemplate(pref: String, name: String, subject: String, bodyText: String, bodyHTML: String) async throws -> JSONValue {
        try await client.json(
            method: "PUT",
            path: prefPath(pref, "/email/templates/\(PathEncode.segment(name))"),
            json: .object([
                "subject": .string(subject),
                "body_text": .string(bodyText),
                "body_html": .string(bodyHTML),
            ])
        )
    }

    public func resetTemplate(pref: String, name: String) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/email/templates/\(PathEncode.segment(name))/reset")
        )
    }
}

public final class StorageStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func objects(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/objects"))
    }

    public func deleteObject(pref: String, key: String) async throws {
        _ = try await client.json(
            method: "DELETE",
            path: prefPath(pref, "/objects"),
            json: .object(["key": .string(key)])
        )
    }

    public func objectURL(pref: String, key: String) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/objects/url"),
            json: .object(["key": .string(key)])
        )
    }

    public func upload(pref: String, path: String, bytes: Data, contentType: String) async throws -> JSONValue {
        try await client.uploadObject(pref: pref, path: path, bytes: bytes, contentType: contentType)
    }

    public func buckets(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/buckets"))
    }

    public func createBucket(pref: String, name: String, isPublic: Bool) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/buckets"),
            json: .object(["name": .string(name), "public": .bool(isPublic)])
        )
    }

    public func updateBucket(pref: String, name: String, isPublic: Bool) async throws -> JSONValue {
        try await client.json(
            method: "PATCH",
            path: prefPath(pref, "/buckets/\(PathEncode.segment(name))"),
            json: .object(["public": .bool(isPublic)])
        )
    }
}

public final class FunctionsStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func list(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/functions"))
    }

    public func deploy(pref: String, name: String, zip: Data, promote: Bool) async throws -> JSONValue {
        try await client.uploadZip(pref: pref, name: name, bytes: zip, promote: promote)
    }

    public func promote(pref: String, name: String, version: Int) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/functions/\(PathEncode.segment(name))/promote"),
            json: .object(["version": .int(version)])
        )
    }

    public func demote(pref: String, name: String) async throws -> JSONValue {
        try await client.json(method: "POST", path: prefPath(pref, "/functions/\(PathEncode.segment(name))/demote"))
    }

    public func stats(pref: String, name: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/functions/\(PathEncode.segment(name))/stats"))
    }

    public func env(pref: String, name: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/functions/\(PathEncode.segment(name))/env"))
    }

    public func setEnv(pref: String, name: String, key: String, value: String) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/functions/\(PathEncode.segment(name))/env"),
            json: .object(["key": .string(key), "value": .string(value)])
        )
    }

    public func deleteEnv(pref: String, name: String, key: String) async throws {
        _ = try await client.json(
            method: "DELETE",
            path: prefPath(pref, "/functions/\(PathEncode.segment(name))/env/\(PathEncode.segment(key))")
        )
    }

    public func invoke(pref: String, name: String, version: Int?, payload: Data) async throws -> JSONValue {
        var query: [URLQueryItem] = []
        if let version { query.append(URLQueryItem(name: "version", value: String(version))) }
        return try await client.json(
            method: "POST",
            path: prefPath(pref, "/functions/\(PathEncode.segment(name))/invoke"),
            query: query,
            raw: payload,
            contentType: "application/json"
        )
    }
}

public final class SitesStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func overview(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/sites"))
    }

    public func stats(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/site/stats"))
    }

    public func deployments(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/site/deployments"))
    }

    public func deployment(pref: String, id: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/site/deployments/\(PathEncode.segment(id))"))
    }

    public func env(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/site/env"))
    }

    public func setEnv(pref: String, key: String, value: String, visibility: String) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/site/env"),
            json: .object([
                "key": .string(key),
                "value": .string(value),
                "visibility": .string(visibility),
            ])
        )
    }

    public func deleteEnv(pref: String, key: String) async throws {
        _ = try await client.json(method: "DELETE", path: prefPath(pref, "/site/env/\(PathEncode.segment(key))"))
    }

    public func domains(pref: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: prefPath(pref, "/site/domains"))
    }

    public func addDomain(pref: String, host: String) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: prefPath(pref, "/site/domains"),
            json: .object(["host": .string(host)])
        )
    }

    public func verifyDomain(pref: String, host: String) async throws {
        _ = try await client.json(
            method: "POST",
            path: prefPath(pref, "/site/domains/\(PathEncode.segment(host))/verify")
        )
    }

    public func deleteDomain(pref: String, host: String) async throws {
        _ = try await client.json(
            method: "DELETE",
            path: prefPath(pref, "/site/domains/\(PathEncode.segment(host))")
        )
    }
}

public struct QueueUnavailable: Error, Equatable {}

public final class QueueStore: @unchecked Sendable {
    public var enabled: Bool
    private let client: ConsoleClient

    public init(client: ConsoleClient, enabled: Bool) {
        self.client = client
        self.enabled = enabled
    }

    public func list(pref: String) async throws -> JSONValue {
        try requireEnabled()
        return try await client.json(method: "GET", path: prefPath(pref, "/queues"))
    }

    public func create(pref: String, name: String) async throws -> JSONValue {
        try requireEnabled()
        return try await client.json(
            method: "POST",
            path: prefPath(pref, "/queues"),
            json: .object(["name": .string(name)])
        )
    }

    public func send(pref: String, name: String, message: JSONValue, delaySecs: Int) async throws -> JSONValue {
        try requireEnabled()
        return try await client.json(
            method: "POST",
            path: prefPath(pref, "/queues/\(PathEncode.segment(name))/send"),
            json: .object(["message": message, "delay_secs": .int(delaySecs)])
        )
    }

    public func peek(pref: String, name: String) async throws -> JSONValue {
        try requireEnabled()
        return try await client.json(method: "GET", path: prefPath(pref, "/queues/\(PathEncode.segment(name))/peek"))
    }

    public func subscriptions(pref: String, name: String) async throws -> JSONValue {
        try requireEnabled()
        return try await client.json(
            method: "GET",
            path: prefPath(pref, "/queues/\(PathEncode.segment(name))/subscriptions")
        )
    }

    private func requireEnabled() throws {
        if !enabled { throw QueueUnavailable() }
    }
}

public final class ClusterStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func load() async throws -> JSONValue {
        try await client.json(method: "GET", path: "/console/v1/cluster")
    }

    public func rename(name: String) async throws -> JSONValue {
        try await client.json(method: "POST", path: "/console/v1/cluster", json: .object(["cluster_name": .string(name)]))
    }

    public func email() async throws -> JSONValue {
        try await client.json(method: "GET", path: "/console/v1/cluster/email")
    }

    public func saveEmail(host: String, port: Int, username: String, password: String?, fromAddress: String, tls: String) async throws -> JSONValue {
        var fields: [String: JSONValue] = [
            "host": .string(host),
            "port": .int(port),
            "username": .string(username),
            "from_address": .string(fromAddress),
            "tls": .string(tls),
        ]
        if let password, !password.isEmpty { fields["password"] = .string(password) }
        return try await client.json(method: "PUT", path: "/console/v1/cluster/email", json: .object(fields))
    }
}

public final class OperatorsStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func list() async throws -> JSONValue {
        try await client.json(method: "GET", path: "/console/v1/operators")
    }

    public func create(name: String, email: String, password: String, platformAdmin: Bool) async throws -> JSONValue {
        try await client.json(
            method: "POST",
            path: "/console/v1/operators",
            json: .object([
                "name": .string(name),
                "email": .string(email),
                "password": .string(password),
                "platform_admin": .bool(platformAdmin),
            ])
        )
    }

    public func setPlatformAdmin(id: String, enabled: Bool) async throws {
        _ = try await client.json(
            method: "POST",
            path: "/console/v1/operators/\(PathEncode.segment(id))",
            json: .object(["platform_admin": .bool(enabled)])
        )
    }

    public func grant(id: String, pref: String, role: String) async throws {
        _ = try await client.json(
            method: "POST",
            path: "/console/v1/operators/\(PathEncode.segment(id))/projects/\(PathEncode.segment(pref))",
            json: .object(["role": .string(role)])
        )
    }

    public func revoke(id: String, pref: String) async throws {
        _ = try await client.json(
            method: "DELETE",
            path: "/console/v1/operators/\(PathEncode.segment(id))/projects/\(PathEncode.segment(pref))"
        )
    }

    public func delete(id: String, selfID: String) async throws {
        if id == selfID {
            throw ConsoleError(status: 400, message: "you cannot delete yourself")
        }
        _ = try await client.json(method: "DELETE", path: "/console/v1/operators/\(PathEncode.segment(id))")
    }
}

public final class ServiceKeysStore: @unchecked Sendable {
    public var hold: SecretHold?
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func list() async throws -> JSONValue {
        try await client.json(method: "GET", path: "/console/v1/keys")
    }

    public func create(name: String, scopes: [String]) async throws -> JSONValue {
        let body = try await client.json(
            method: "POST",
            path: "/console/v1/keys",
            json: .object([
                "name": .string(name),
                "scopes": .array(scopes.map { .string($0) }),
            ])
        )
        hold = holdKeys("Service key", body, names: ["token"])
        return body
    }

    public func revoke(id: String) async throws {
        _ = try await client.json(method: "DELETE", path: "/console/v1/keys/\(PathEncode.segment(id))")
    }

    public func acknowledgeSecrets() { hold = nil }
}

public final class AgentStore: @unchecked Sendable {
    private let client: ConsoleClient
    public init(client: ConsoleClient) { self.client = client }

    public func models() async throws -> JSONValue {
        try await client.json(method: "GET", path: "/console/v1/agent/models")
    }

    public func threads() async throws -> JSONValue {
        try await client.json(method: "GET", path: "/console/v1/agent/threads")
    }

    public func createThread(projectRef: String?) async throws -> JSONValue {
        var fields: [String: JSONValue] = [:]
        if let projectRef { fields["project_ref"] = .string(projectRef) }
        return try await client.json(method: "POST", path: "/console/v1/agent/threads", json: .object(fields))
    }

    public func post(id: String, text: String, model: String?) async throws -> JSONValue {
        var fields: [String: JSONValue] = ["text": .string(text)]
        if let model { fields["model"] = .string(model) }
        return try await client.json(
            method: "POST",
            path: "/console/v1/agent/threads/\(PathEncode.segment(id))/messages",
            json: .object(fields)
        )
    }

    public func thread(id: String) async throws -> JSONValue {
        try await client.json(method: "GET", path: "/console/v1/agent/threads/\(PathEncode.segment(id))")
    }

    public func poll(id: String, maxPolls: Int = 5) async throws -> JSONValue {
        var last: JSONValue = .null
        let polls = max(1, maxPolls)
        for _ in 0..<polls {
            last = try await thread(id: id)
            if last["running"]?.boolValue != true { return last }
        }
        return last
    }
}
