import ConsoleCore
import Foundation
import Security

final class KeychainVault: SessionVault, @unchecked Sendable {
    private let account = "lab.reactor.console.clusters"

    func load() async throws -> [ClusterRecord] {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess, let data = item as? Data else {
            throw ConsoleError(status: Int(status), message: "keychain read failed")
        }
        return try ClusterCoding.decoder().decode([ClusterRecord].self, from: data)
    }

    func save(_ records: [ClusterRecord]) async throws {
        let data = try ClusterCoding.encoder().encode(records)
        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(base as CFDictionary)
        var insert = base
        insert[kSecValueData as String] = data
        let status = SecItemAdd(insert as CFDictionary, nil)
        if status != errSecSuccess {
            throw ConsoleError(status: Int(status), message: "keychain write failed")
        }
    }
}
