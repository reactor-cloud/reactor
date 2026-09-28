import Foundation
import Security

public struct User: Codable, Sendable, Equatable {
    public var id: String
    public var email: String
}

public struct Session: Codable, Sendable, Equatable {
    public var accessToken: String
    public var refreshToken: String
    public var user: User

    enum CodingKeys: String, CodingKey {
        case accessToken = "access_token"
        case refreshToken = "refresh_token"
        case user
    }
}

public protocol SessionStore: Sendable {
    func load() -> Session?
    func save(_ session: Session?)
}

public final class MemorySessionStore: SessionStore, @unchecked Sendable {
    private let lock = NSLock()
    private var session: Session?

    public init() {}

    public func load() -> Session? {
        lock.lock()
        defer { lock.unlock() }
        return session
    }

    public func save(_ session: Session?) {
        lock.lock()
        defer { lock.unlock() }
        self.session = session
    }
}

public final class KeychainSessionStore: SessionStore, @unchecked Sendable {
    private let service: String

    public init(service: String = "lab.reactor.session") {
        self.service = service
    }

    public func load() -> Session? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: "session",
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data else { return nil }
        return try? JSONDecoder().decode(Session.self, from: data)
    }

    public func save(_ session: Session?) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: "session",
        ]
        SecItemDelete(query as CFDictionary)
        guard let session, let data = try? JSONEncoder().encode(session) else { return }
        var insert = query
        insert[kSecValueData as String] = data
        SecItemAdd(insert as CFDictionary, nil)
    }
}

public struct ReactorError: Error, CustomStringConvertible, Equatable {
    public var status: Int
    public var message: String
    public var description: String { message }

    public init(status: Int, message: String) {
        self.status = status
        self.message = message
    }
}
