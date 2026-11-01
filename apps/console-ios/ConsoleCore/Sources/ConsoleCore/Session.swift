import Foundation
import Synchronization

public struct ClusterRecord: Codable, Equatable, Sendable, Identifiable {
    public var id: String { url }
    public var url: String
    public var name: String
    public var email: String
    public var accessToken: String
    public var expiresAt: Date

    public init(url: String, name: String, email: String, accessToken: String, expiresAt: Date) {
        self.url = url
        self.name = name
        self.email = email
        self.accessToken = accessToken
        self.expiresAt = expiresAt
    }
}

public enum ClusterCoding {
    public static func encoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        return encoder
    }

    public static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        return decoder
    }
}

public protocol SessionVault: Sendable {
    func load() async throws -> [ClusterRecord]
    func save(_ records: [ClusterRecord]) async throws
}

public final class MemoryVault: SessionVault, @unchecked Sendable {
    private let state: Mutex<[ClusterRecord]>

    public init(_ records: [ClusterRecord] = []) {
        state = Mutex(records)
    }

    public func load() async throws -> [ClusterRecord] {
        state.withLock { $0 }
    }

    public func save(_ records: [ClusterRecord]) async throws {
        state.withLock { $0 = records }
    }
}

public enum AuthPhase: Equatable, Sendable {
    case setup
    case credentials
    case otp(mfaToken: String)
    case enrollment(url: String)
    case session(ClusterRecord)
}

enum TokenTime {
    static let horizon: TimeInterval = 12 * 60 * 60
    static let reauthWindow: TimeInterval = 10 * 60

    static func expiry(of token: String, now: Date) -> Date {
        let parts = token.split(separator: ".")
        guard parts.count >= 2, let data = base64URLDecode(String(parts[1])) else {
            return now.addingTimeInterval(horizon)
        }
        guard let json = try? JSONValue.parse(data), let exp = json["exp"]?.intValue else {
            return now.addingTimeInterval(horizon)
        }
        return Date(timeIntervalSince1970: TimeInterval(exp))
    }

    static func make(exp: Date) -> String {
        let header = base64URLEncode(Data(#"{"alg":"none"}"#.utf8))
        let payload = base64URLEncode(Data("{\"exp\":\(Int(exp.timeIntervalSince1970))}".utf8))
        return "\(header).\(payload).sig"
    }

    private static func base64URLEncode(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    private static func base64URLDecode(_ value: String) -> Data? {
        var text = value.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while text.count % 4 != 0 { text.append("=") }
        return Data(base64Encoded: text)
    }
}

public func trimmedClusterURL(_ url: String) -> String {
    var value = url.trimmingCharacters(in: .whitespacesAndNewlines)
    while value.hasSuffix("/") { value.removeLast() }
    return value
}

private struct PendingFactor: Sendable {
    var mfaToken: String
    var email: String
    var name: String
}

public actor SessionMachine {
    public static let reauthWindow: TimeInterval = TokenTime.reauthWindow

    private let transport: any HTTPTransport
    private let vault: any SessionVault
    private var reauthURLs: Set<String> = []
    private var pending: [String: PendingFactor] = [:]

    public init(transport: any HTTPTransport, vault: any SessionVault) {
        self.transport = transport
        self.vault = vault
    }

    public func begin(url: String) async throws -> AuthPhase {
        let body = try await client(url).json(method: "GET", path: "/console/v1/setup")
        if body["needs_setup"]?.boolValue == true { return .setup }
        return .credentials
    }

    public func submitSetup(
        url: String,
        clusterName: String,
        name: String,
        email: String,
        password: String,
        now: Date = Date()
    ) async throws -> AuthPhase {
        let body = try await client(url).json(
            method: "POST",
            path: "/console/v1/setup",
            json: .object([
                "cluster_name": .string(clusterName),
                "name": .string(name),
                "email": .string(email),
                "password": .string(password),
            ])
        )
        return try await interpret(url: url, name: clusterName, email: email, body: body, now: now)
    }

    public func signIn(
        url: String,
        name: String,
        email: String,
        password: String,
        now: Date = Date()
    ) async throws -> AuthPhase {
        let body = try await client(url).json(
            method: "POST",
            path: "/console/v1/login",
            json: .object([
                "email": .string(email),
                "password": .string(password),
            ])
        )
        return try await interpret(url: url, name: name, email: email, body: body, now: now)
    }

    public func submitOTP(url: String, code: String, now: Date = Date()) async throws -> AuthPhase {
        try await secondFactor(url: url, path: "/console/v1/mfa/totp", code: code, now: now)
    }

    public func submitRecovery(url: String, code: String, now: Date = Date()) async throws -> AuthPhase {
        try await secondFactor(url: url, path: "/console/v1/mfa/recovery", code: code, now: now)
    }

    public func logout(url: String) async throws {
        let key = trimmedClusterURL(url)
        var records = try await vault.load()
        records.removeAll { $0.url == key }
        try await vault.save(records)
        reauthURLs.remove(key)
        pending.removeValue(forKey: key)
    }

    public func sessions() async throws -> [ClusterRecord] {
        try await vault.load()
    }

    public func needsReauth(url: String, now: Date = Date()) async throws -> Bool {
        let key = trimmedClusterURL(url)
        if reauthURLs.contains(key) { return true }
        let records = try await vault.load()
        guard let record = records.first(where: { $0.url == key }) else { return false }
        return record.expiresAt.timeIntervalSince(now) < TokenTime.reauthWindow
    }

    public func noteUnauthorized(url: String) {
        reauthURLs.insert(trimmedClusterURL(url))
    }

    private func secondFactor(url: String, path: String, code: String, now: Date) async throws -> AuthPhase {
        let key = trimmedClusterURL(url)
        guard let factor = pending[key] else {
            throw ConsoleError(status: 400, message: "sign in before the second factor")
        }
        do {
            let body = try await client(key, token: factor.mfaToken).json(
                method: "POST",
                path: path,
                json: .object(["code": .string(code)]),
                bearer: .token(factor.mfaToken)
            )
            guard let token = body["access_token"]?.stringValue else {
                throw ConsoleError(status: 500, message: "missing access token")
            }
            let record = ClusterRecord(
                url: key,
                name: factor.name,
                email: factor.email,
                accessToken: token,
                expiresAt: TokenTime.expiry(of: token, now: now)
            )
            try await upsert(record)
            reauthURLs.remove(key)
            pending.removeValue(forKey: key)
            return .session(record)
        } catch let error as ConsoleError where error.status == 401 {
            return .otp(mfaToken: factor.mfaToken)
        }
    }

    private func interpret(url: String, name: String, email: String, body: JSONValue, now: Date) async throws -> AuthPhase {
        let key = trimmedClusterURL(url)
        if let token = body["access_token"]?.stringValue {
            let record = ClusterRecord(
                url: key,
                name: name,
                email: email,
                accessToken: token,
                expiresAt: TokenTime.expiry(of: token, now: now)
            )
            try await upsert(record)
            reauthURLs.remove(key)
            pending.removeValue(forKey: key)
            return .session(record)
        }
        if let mfa = body["mfa_token"]?.stringValue {
            pending[key] = PendingFactor(mfaToken: mfa, email: email, name: name)
            return .otp(mfaToken: mfa)
        }
        pending.removeValue(forKey: key)
        return .enrollment(url: key + "/console/login")
    }

    private func upsert(_ record: ClusterRecord) async throws {
        var records = try await vault.load()
        records.removeAll { $0.url == record.url }
        records.append(record)
        try await vault.save(records)
    }

    private func client(_ url: String, token: String? = nil) -> ConsoleClient {
        let base = URL(string: trimmedClusterURL(url)) ?? URL(string: "http://127.0.0.1:18000")!
        return ConsoleClient(baseURL: base, token: token, transport: transport)
    }
}
