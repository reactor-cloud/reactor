import Foundation

public enum JSON: Sendable, Equatable {
    case string(String)
    case int(Int)
    case double(Double)
    case bool(Bool)
    case object([String: JSON])
    case array([JSON])
    case null

    public func object() -> [String: JSON]? {
        if case .object(let value) = self { return value }
        return nil
    }

    public func array() -> [JSON]? {
        if case .array(let value) = self { return value }
        return nil
    }

    public func string() -> String? {
        if case .string(let value) = self { return value }
        return nil
    }

    public func bool() -> Bool? {
        if case .bool(let value) = self { return value }
        return nil
    }

    public subscript(_ key: String) -> JSON? {
        object()?[key]
    }

    func foundation() -> Any {
        switch self {
        case .string(let value): return value
        case .int(let value): return value
        case .double(let value): return value
        case .bool(let value): return value
        case .object(let value): return value.mapValues { $0.foundation() }
        case .array(let value): return value.map { $0.foundation() }
        case .null: return NSNull()
        }
    }

    static func from(_ value: Any) -> JSON {
        switch value {
        case is NSNull:
            return .null
        case let value as String:
            return .string(value)
        case let value as NSNumber:
            if CFGetTypeID(value) == CFBooleanGetTypeID() {
                return .bool(value.boolValue)
            }
            let type = String(cString: value.objCType)
            if type == "d" || type == "f" {
                return .double(value.doubleValue)
            }
            return .int(value.intValue)
        case let value as [Any]:
            return .array(value.map(from))
        case let value as [String: Any]:
            return .object(value.mapValues(from))
        default:
            return .null
        }
    }

    func data() throws -> Data {
        try JSONSerialization.data(withJSONObject: foundation())
    }

    static func parse(_ data: Data) throws -> JSON {
        if data.isEmpty { return .null }
        let value = try JSONSerialization.jsonObject(with: data)
        return from(value)
    }
}
