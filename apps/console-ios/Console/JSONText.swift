import ConsoleCore
import Foundation

extension JSONValue {
    var refText: String { self["ref"]?.stringValue ?? self["name"]?.stringValue ?? "" }
    var userID: String { self["id"]?.stringValue ?? "" }
    var providerName: String { self["provider"]?.stringValue ?? "" }
    var templateName: String { self["name"]?.stringValue ?? "" }
    var rowKey: String { self["id"]?.stringValue ?? self["key"]?.stringValue ?? self["host"]?.stringValue ?? descriptionText }
    var descriptionText: String {
        switch self {
        case .string(let value): return value
        case .int(let value): return String(value)
        case .bool(let value): return value ? "true" : "false"
        default: return ""
        }
    }
}
