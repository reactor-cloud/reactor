import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

public struct HTTPRequest: Sendable, Equatable {
    public var method: String
    public var url: URL
    public var headers: [String: String]
    public var body: Data?

    public init(method: String, url: URL, headers: [String: String] = [:], body: Data? = nil) {
        self.method = method
        self.url = url
        self.headers = headers
        self.body = body
    }
}

public struct HTTPResponse: Sendable, Equatable {
    public var status: Int
    public var body: Data

    public init(status: Int, body: Data) {
        self.status = status
        self.body = body
    }

    public init(status: Int, json: String) {
        self.status = status
        self.body = Data(json.utf8)
    }
}

public protocol HTTPTransport: Sendable {
    func send(_ request: HTTPRequest) async throws -> HTTPResponse
}

public enum Bearer: Sendable, Equatable {
    case session
    case token(String)
    case none
}

public actor ScriptedTransport: HTTPTransport {
    public private(set) var requests: [HTTPRequest] = []
    private var responses: [HTTPResponse]

    public init(_ responses: [HTTPResponse]) {
        self.responses = responses
    }

    public func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        requests.append(request)
        if responses.isEmpty {
            return HTTPResponse(status: 500, json: #"{"error":"empty script"}"#)
        }
        return responses.removeFirst()
    }
}

public final class URLSessionTransport: HTTPTransport, @unchecked Sendable {
    private let session: URLSession

    public init(session: URLSession? = nil) {
        if let session {
            self.session = session
        } else {
            let config = URLSessionConfiguration.ephemeral
            config.timeoutIntervalForRequest = 30
            config.timeoutIntervalForResource = 60
            config.httpAdditionalHeaders = ["Expect": ""]
            self.session = URLSession(configuration: config)
        }
    }

    public func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        var urlRequest = URLRequest(url: request.url)
        urlRequest.httpMethod = request.method
        urlRequest.httpBody = request.body
        for (name, value) in request.headers {
            urlRequest.setValue(value, forHTTPHeaderField: name)
        }
        let (data, response) = try await session.data(for: urlRequest)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        return HTTPResponse(status: status, body: data)
    }
}

enum PathEncode {
    static func segment(_ value: String) -> String {
        var allowed = CharacterSet.urlPathAllowed
        allowed.remove(charactersIn: "/")
        return value.addingPercentEncoding(withAllowedCharacters: allowed) ?? value
    }
}

public struct ConsoleClient: Sendable {
    public var baseURL: URL
    public var token: String?
    public var transport: any HTTPTransport

    public init(baseURL: URL, token: String? = nil, transport: any HTTPTransport) {
        self.baseURL = baseURL
        self.token = token
        self.transport = transport
    }

    public func send(
        method: String,
        path: String,
        query: [URLQueryItem] = [],
        json: JSONValue? = nil,
        raw: Data? = nil,
        contentType: String? = nil,
        bearer: Bearer = .session
    ) async throws -> HTTPResponse {
        let url = try urlFor(path: path, query: query)
        var headers: [String: String] = [:]
        if let authorization = authorization(bearer) {
            headers["authorization"] = authorization
        }
        var body = raw
        if let json {
            body = try json.encoded()
            headers["content-type"] = contentType ?? "application/json"
        } else if let contentType {
            headers["content-type"] = contentType
        }
        let request = HTTPRequest(method: method, url: url, headers: headers, body: body)
        return try await transport.send(request)
    }

    public func json(
        method: String,
        path: String,
        query: [URLQueryItem] = [],
        json body: JSONValue? = nil,
        raw: Data? = nil,
        contentType: String? = nil,
        bearer: Bearer = .session
    ) async throws -> JSONValue {
        let response = try await send(
            method: method,
            path: path,
            query: query,
            json: body,
            raw: raw,
            contentType: contentType,
            bearer: bearer
        )
        try throwIfNeeded(response)
        return try JSONValue.parse(response.body)
    }

    public func runSql(
        pref: String,
        sql: String,
        readOnly: Bool,
        confirm: Bool,
        mode: String = "run",
        version: String? = nil,
        downSQL: String? = nil
    ) async throws -> SqlOutcome {
        var fields: [String: JSONValue] = [
            "sql": .string(sql),
            "mode": .string(mode),
            "read_only": .bool(readOnly),
            "confirm": .bool(confirm),
        ]
        if let version { fields["version"] = .string(version) }
        if let downSQL { fields["down_sql"] = .string(downSQL) }
        let response = try await send(
            method: "POST",
            path: "/console/v1/projects/\(PathEncode.segment(pref))/sql",
            json: .object(fields)
        )
        if response.status == 409 {
            let body = try JSONValue.parse(response.body)
            let warnings = body["warnings"]?.arrayValue?.compactMap(\.stringValue) ?? []
            return .confirmation(warnings)
        }
        try throwIfNeeded(response)
        return .value(try JSONValue.parse(response.body))
    }

    public func uploadZip(pref: String, name: String, bytes: Data, promote: Bool) async throws -> JSONValue {
        try await json(
            method: "POST",
            path: "/console/v1/projects/\(PathEncode.segment(pref))/functions/\(PathEncode.segment(name))",
            query: [URLQueryItem(name: "promote", value: promote ? "true" : "false")],
            raw: bytes,
            contentType: "application/zip"
        )
    }

    public func uploadObject(pref: String, path: String, bytes: Data, contentType: String) async throws -> JSONValue {
        let issued = try await json(
            method: "POST",
            path: "/console/v1/projects/\(PathEncode.segment(pref))/objects/upload",
            json: .object(["path": .string(path)])
        )
        guard let target = issued["url"]?.stringValue else {
            throw ConsoleError(status: 500, message: "upload url missing")
        }
        let put = try await send(method: "PUT", path: target, raw: bytes, contentType: contentType, bearer: .none)
        try throwIfNeeded(put)
        return issued
    }

    func urlFor(path: String, query: [URLQueryItem]) throws -> URL {
        if path.hasPrefix("http://") || path.hasPrefix("https://") {
            guard let url = URL(string: path) else {
                throw ConsoleError(status: 400, message: "invalid url")
            }
            return url
        }
        let root = baseURL.absoluteString.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard var components = URLComponents(string: root + path) else {
            throw ConsoleError(status: 400, message: "invalid path")
        }
        if !query.isEmpty {
            components.queryItems = query
        }
        guard let url = components.url else {
            throw ConsoleError(status: 400, message: "invalid path")
        }
        return url
    }

    private func authorization(_ bearer: Bearer) -> String? {
        switch bearer {
        case .none:
            return nil
        case .token(let value):
            return "Bearer \(value)"
        case .session:
            guard let token, !token.isEmpty else { return nil }
            return "Bearer \(token)"
        }
    }

    private func throwIfNeeded(_ response: HTTPResponse) throws {
        guard response.status >= 400 else { return }
        let parsed = try? JSONValue.parse(response.body)
        let message = parsed?["error"]?.stringValue ?? "request failed"
        throw ConsoleError(status: response.status, message: message)
    }
}

public enum SqlOutcome: Equatable, Sendable {
    case value(JSONValue)
    case confirmation([String])
}
