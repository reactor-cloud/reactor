import ConsoleCore
import Foundation

@MainActor
final class AppModel: ObservableObject {
    @Published var records: [ClusterRecord] = []
    @Published var phase: AuthPhase = .credentials
    @Published var current: ClusterRecord?
    @Published var clusterURL = "http://127.0.0.1:18000"
    @Published var email = ""
    @Published var password = ""
    @Published var clusterName = ""
    @Published var operatorName = ""
    @Published var otp = ""
    @Published var useRecovery = false
    @Published var message = ""
    @Published var me: JSONValue = .null
    @Published var projects: JSONValue = .null
    @Published var projectRef: String?
    @Published var role: ProjectRole = .developer
    @Published var platformAdmin = false
    @Published var operatorID = ""
    @Published var queueEnabled = false
    @Published var drawer = DrawerModel()
    @Published var drawerOpen = false
    @Published var accountOpen = false
    @Published var secrets: SecretHold?
    @Published var payload: JSONValue = .null
    @Published var busy = false

    let transport: URLSessionTransport
    private let machine: SessionMachine

    init(vault: any SessionVault, transport: URLSessionTransport = URLSessionTransport()) {
        self.transport = transport
        self.machine = SessionMachine(transport: transport, vault: vault)
    }

    var client: ConsoleClient? {
        guard let current, let url = URL(string: current.url) else { return nil }
        return ConsoleClient(baseURL: url, token: current.accessToken, transport: transport)
    }

    func reload() async {
        records = (try? await machine.sessions()) ?? []
    }

    func begin() async {
        await run {
            phase = try await machine.begin(url: clusterURL)
            message = ""
        }
    }

    func submitSetup() async {
        await run {
            phase = try await machine.submitSetup(
                url: clusterURL,
                clusterName: clusterName,
                name: operatorName,
                email: email,
                password: password
            )
            try await afterAuth()
        }
    }

    func signIn(url: String? = nil) async {
        let target = url ?? clusterURL
        clusterURL = target
        await run {
            if try await machine.needsReauth(url: target), password.isEmpty, current?.url == target {
                phase = .credentials
                return
            }
            phase = try await machine.signIn(url: target, name: hostName(target), email: email, password: password)
            try await afterAuth()
        }
    }

    func submitCode() async {
        await run {
            phase = useRecovery
                ? try await machine.submitRecovery(url: clusterURL, code: otp)
                : try await machine.submitOTP(url: clusterURL, code: otp)
            otp = ""
            try await afterAuth()
        }
    }

    func open(_ record: ClusterRecord) async {
        clusterURL = record.url
        email = record.email
        current = record
        if try await machine.needsReauth(url: record.url) {
            phase = .credentials
            password = ""
            return
        }
        phase = .session(record)
        await enter(record)
    }

    func logoutCurrent() async {
        guard let current else { return }
        try? await machine.logout(url: current.url)
        self.current = nil
        projectRef = nil
        phase = .credentials
        await reload()
    }

    func select(_ screen: Screen) {
        drawer.select(screen)
        drawerOpen = false
        payload = .null
    }

    func openProject(_ ref: String, role: String) {
        projectRef = ref
        self.role = ProjectRole(rawValue: role) ?? .developer
        drawer = DrawerModel(projectRef: ref, role: self.role, platformAdmin: platformAdmin, queueEnabled: queueEnabled)
        drawer.select(.overview)
    }

    func leaveProject() {
        projectRef = nil
        drawer = DrawerModel(platformAdmin: platformAdmin, queueEnabled: queueEnabled)
        drawer.select(.projects)
    }

    func refreshHome() async {
        guard let client else { return }
        await run {
            let store = ProjectsStore(client: client)
            me = try await store.loadMe()
            projects = try await store.load()
            platformAdmin = me["operator"]?["platform_admin"]?.boolValue ?? false
            operatorID = me["operator"]?["id"]?.stringValue ?? ""
            queueEnabled = try await ExtensionsStore(client: client).queueEnabled()
            if projectRef == nil {
                drawer.platformAdmin = platformAdmin
                drawer.queueEnabled = queueEnabled
            }
        }
    }

    func createProject(name: String) async {
        guard let client else { return }
        await run {
            let store = ProjectsStore(client: client)
            let created = try await store.create(name: name)
            secrets = store.hold
            if let ref = created["ref"]?.stringValue {
                openProject(ref, role: "owner")
            }
            projects = try await store.load()
        }
    }

    func acknowledgeSecrets() {
        secrets = nil
    }

    private func afterAuth() async throws {
        if case .session(let record) = phase {
            current = record
            password = ""
            await enter(record)
        }
        await reload()
    }

    private func enter(_ record: ClusterRecord) async {
        current = record
        await refreshHome()
        drawer = DrawerModel(platformAdmin: platformAdmin, queueEnabled: queueEnabled)
        drawer.select(.projects)
    }

    private func hostName(_ url: String) -> String {
        URL(string: url)?.host ?? url
    }

    func run(_ work: () async throws -> Void) async {
        busy = true
        defer { busy = false }
        do {
            try await work()
            if message == "request failed" { message = "" }
        } catch let error as ConsoleError {
            if error.status == 401, let url = current?.url {
                await machine.noteUnauthorized(url: url)
                phase = .credentials
            }
            message = error.message
        } catch {
            message = error.localizedDescription
        }
    }
}
