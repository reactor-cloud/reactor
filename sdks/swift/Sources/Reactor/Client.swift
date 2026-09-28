import Foundation

public final class ReactorClient: @unchecked Sendable {
    let base: URL
    let anonKey: String
    private let store: any SessionStore
    private let lock = NSLock()
    private var session: Session?

    let http: URLSession

    public init(url: String, anonKey: String, sessionStore: (any SessionStore)? = nil) {
        let trimmed = url.hasSuffix("/") ? String(url.dropLast()) : url
        self.base = URL(string: trimmed)!
        self.anonKey = anonKey
        let store = sessionStore ?? MemorySessionStore()
        self.store = store
        self.session = store.load()
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 20
        config.timeoutIntervalForResource = 30
        config.waitsForConnectivity = false
        config.httpAdditionalHeaders = ["Expect": ""]
        self.http = URLSession(configuration: config)
    }

    public var auth: AuthAPI { AuthAPI(client: self) }
    public var storage: StorageAPI { StorageAPI(client: self) }
    public var functions: FunctionsAPI { FunctionsAPI(client: self) }

    public func from(_ table: String) -> Query {
        Query(client: self, table: table)
    }

    func token() -> String {
        lock.lock()
        defer { lock.unlock() }
        return session?.accessToken ?? anonKey
    }

    func currentSession() -> Session? {
        lock.lock()
        defer { lock.unlock() }
        return session
    }

    func replaceSession(_ session: Session?) {
        lock.lock()
        self.session = session
        lock.unlock()
        store.save(session)
    }

    func url(_ path: String) -> URL {
        URL(string: base.absoluteString + path)!
    }

    func call(_ path: String, method: String = "GET", token: String? = nil, json: JSON? = nil, prefer: String? = nil) async throws -> (Int, JSON) {
        var request = URLRequest(url: url(path))
        request.httpMethod = method
        request.setValue("", forHTTPHeaderField: "Expect")
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        if let prefer {
            request.setValue(prefer, forHTTPHeaderField: "Prefer")
        }
        if let json {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try json.data()
        }
        let (data, response) = try await http.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        if data.isEmpty {
            if status >= 400 { throw ReactorError(status: status, message: "request failed") }
            return (status, .null)
        }
        let body = try JSON.parse(data)
        if status >= 400 {
            let message = body["error"]?.string() ?? body["message"]?.string() ?? "request failed"
            throw ReactorError(status: status, message: message)
        }
        return (status, body)
    }

    func raw(_ url: URL, method: String, body: Data?, contentType: String) async throws -> Data {
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("", forHTTPHeaderField: "Expect")
        request.setValue(contentType, forHTTPHeaderField: "Content-Type")
        request.httpBody = body
        let (data, response) = try await http.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        if status >= 400 {
            throw ReactorError(status: status, message: String(data: data, encoding: .utf8) ?? "request failed")
        }
        return data
    }
}

public struct AuthAPI: Sendable {
    let client: ReactorClient

    public func getSession() -> Session? {
        client.currentSession()
    }

    public func signUp(email: String, password: String) async throws -> Session {
        try await client.issue(path: "/auth/v1/signup", json: .object([
            "email": .string(email),
            "password": .string(password),
        ]))
    }

    public func signInWithPassword(email: String, password: String) async throws -> Session {
        try await client.issue(path: "/auth/v1/token", json: .object([
            "email": .string(email),
            "password": .string(password),
        ]))
    }

    public func signInWithMagicLink(email: String) async throws {
        _ = try await client.call("/auth/v1/magic-link", method: "POST", token: client.anonKey, json: .object([
            "email": .string(email),
        ]))
    }

    public func verifyMagicLink(token: String) async throws -> Session {
        try await client.issue(path: "/auth/v1/verify", json: .object(["token": .string(token)]))
    }

    public func recover(email: String) async throws {
        _ = try await client.call("/auth/v1/recover", method: "POST", token: client.anonKey, json: .object([
            "email": .string(email),
        ]))
    }

    public func completeRecovery(token: String, password: String) async throws -> Session {
        try await client.issue(path: "/auth/v1/recover/complete", json: .object([
            "token": .string(token),
            "password": .string(password),
        ]))
    }

    public func invite(email: String, serviceKey: String) async throws {
        _ = try await client.call("/auth/v1/invite", method: "POST", token: serviceKey, json: .object([
            "email": .string(email),
        ]))
    }

    public func acceptInvite(token: String, password: String) async throws -> Session {
        try await client.issue(path: "/auth/v1/invite/accept", json: .object([
            "token": .string(token),
            "password": .string(password),
        ]))
    }

    public func getUser() async throws -> User {
        guard client.currentSession()?.accessToken != nil else {
            throw ReactorError(status: 401, message: "not signed in")
        }
        let (_, body) = try await client.call("/auth/v1/user", token: client.token())
        let data = try JSONEncoder().encode(sessionUserJSON(body))
        return try JSONDecoder().decode(User.self, from: data)
    }

    public func refreshSession() async throws -> Session {
        guard let refresh = client.currentSession()?.refreshToken else {
            throw ReactorError(status: 401, message: "not signed in")
        }
        return try await client.issue(path: "/auth/v1/token", json: .object(["refresh_token": .string(refresh)]))
    }

    public func signOut() async throws {
        guard let refresh = client.currentSession()?.refreshToken else { return }
        _ = try await client.call("/auth/v1/logout", method: "POST", json: .object(["refresh_token": .string(refresh)]))
        client.replaceSession(nil)
    }

    public func signInWithOAuth() throws -> Never {
        throw ReactorError(status: 0, message: "unsupported")
    }
}

extension ReactorClient {
    func issue(path: String, json: JSON) async throws -> Session {
        let (_, body) = try await call(path, method: "POST", token: anonKey, json: json)
        let data = try JSONSerialization.data(withJSONObject: body.foundation())
        let session = try JSONDecoder().decode(Session.self, from: data)
        replaceSession(session)
        return session
    }
}

private func sessionUserJSON(_ body: JSON) -> [String: String] {
    ["id": body["id"]?.string() ?? "", "email": body["email"]?.string() ?? ""]
}

public struct StorageAPI: Sendable {
    let client: ReactorClient

    public func from(_ bucket: String) -> Bucket {
        Bucket(client: client, bucket: bucket)
    }
}

public struct Bucket: Sendable {
    let client: ReactorClient
    let bucket: String

    public func upload(path: String, data: Data, contentType: String = "application/octet-stream") async throws {
        let signed = try await presign(path: path, method: "PUT")
        _ = try await client.raw(signed, method: "PUT", body: data, contentType: contentType)
    }

    public func download(path: String) async throws -> Data {
        let signed = try await presign(path: path, method: "GET")
        return try await client.raw(signed, method: "GET", body: nil, contentType: "application/octet-stream")
    }

    private func presign(path: String, method: String) async throws -> URL {
        let (_, body) = try await client.call(
            "/storage/v1/object/presign",
            method: "POST",
            token: client.token(),
            json: .object(["bucket": .string(bucket), "key": .string(path), "method": .string(method)])
        )
        guard let value = body["url"]?.string(), let url = URL(string: value) else {
            throw ReactorError(status: 500, message: "missing presign url")
        }
        return url
    }
}

public struct FunctionsAPI: Sendable {
    let client: ReactorClient

    public func invoke(_ name: String, body: JSON = .object([:])) async throws -> JSON {
        let (_, response) = try await client.call("/fn/v1/\(name)", method: "POST", token: client.token(), json: body)
        return response
    }
}

public struct Query: Sendable {
    let client: ReactorClient
    let table: String
    var method = "GET"
    var columns: String?
    var body: JSON?
    var filters: [(String, String)] = []
    var orderBy: String?
    var rowLimit: Int?

    public func select(_ columns: String = "*") -> Query {
        var copy = self
        copy.columns = columns
        return copy
    }

    public func insert(_ row: [String: JSON]) -> Query {
        var copy = self
        copy.method = "POST"
        copy.body = .object(row)
        return copy
    }

    public func update(_ row: [String: JSON]) -> Query {
        var copy = self
        copy.method = "PATCH"
        copy.body = .object(row)
        return copy
    }

    public func delete() -> Query {
        var copy = self
        copy.method = "DELETE"
        return copy
    }

    public func eq(_ column: String, _ value: String) -> Query {
        var copy = self
        copy.filters.append((column, value))
        return copy
    }

    public func order(_ column: String, ascending: Bool = true) -> Query {
        var copy = self
        copy.orderBy = ascending ? "\(column).asc" : "\(column).desc"
        return copy
    }

    public func limit(_ count: Int) -> Query {
        var copy = self
        copy.rowLimit = count
        return copy
    }

    public func execute() async throws -> JSON {
        var items: [URLQueryItem] = []
        if let columns { items.append(URLQueryItem(name: "select", value: columns)) }
        for (column, value) in filters {
            items.append(URLQueryItem(name: column, value: "eq.\(value)"))
        }
        if let orderBy { items.append(URLQueryItem(name: "order", value: orderBy)) }
        if let rowLimit { items.append(URLQueryItem(name: "limit", value: String(rowLimit))) }
        var components = URLComponents(url: client.url("/data/v1/\(table)"), resolvingAgainstBaseURL: false)!
        if !items.isEmpty { components.queryItems = items }
        var request = URLRequest(url: components.url!)
        request.httpMethod = method
        request.setValue("", forHTTPHeaderField: "Expect")
        request.setValue("Bearer \(client.token())", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if method != "GET" {
            request.setValue("return=representation", forHTTPHeaderField: "Prefer")
        }
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try body.data()
        }
        let (data, response) = try await client.http.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        if data.isEmpty {
            if status >= 400 { throw ReactorError(status: status, message: "request failed") }
            return .null
        }
        let parsed = try JSON.parse(data)
        if status >= 400 {
            let message = parsed["error"]?.string() ?? parsed["message"]?.string() ?? "request failed"
            throw ReactorError(status: status, message: message)
        }
        return parsed
    }
}
