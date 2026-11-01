import Foundation
import XCTest
@testable import ConsoleCore

func jsonResponse(_ json: String, status: Int = 200) -> HTTPResponse {
    HTTPResponse(status: status, json: json)
}

func scripted(_ responses: [HTTPResponse], token: String? = "session-token") -> (ScriptedTransport, ConsoleClient) {
    let transport = ScriptedTransport(responses)
    let client = ConsoleClient(
        baseURL: URL(string: "https://cluster.example")!,
        token: token,
        transport: transport
    )
    return (transport, client)
}

func objectBody(_ data: Data?) throws -> [String: Any] {
    let raw = try JSONSerialization.jsonObject(with: data ?? Data()) 
    return raw as? [String: Any] ?? [:]
}
