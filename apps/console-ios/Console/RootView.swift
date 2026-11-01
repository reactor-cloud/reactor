import ConsoleCore
import SwiftUI

struct RootView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Group {
            if case .session = model.phase, model.current != nil {
                ShellView()
            } else {
                SignInView()
            }
        }
        .task { await model.reload() }
        .alert("Something went wrong", isPresented: errorShown) {
            Button("OK") { model.message = "" }
        } message: {
            Text(model.message)
        }
        .sheet(item: secretItem) { hold in
            SecretHoldView(hold: hold) { model.acknowledgeSecrets() }
        }
    }

    private var errorShown: Binding<Bool> {
        Binding(get: { !model.message.isEmpty && model.phase != .credentials }, set: { if !$0 { model.message = "" } })
    }

    private var secretItem: Binding<SecretItem?> {
        Binding(
            get: { model.secrets.map { SecretItem(hold: $0) } },
            set: { if $0 == nil { model.acknowledgeSecrets() } }
        )
    }
}

struct SecretItem: Identifiable {
    var id: String { hold.title }
    var hold: SecretHold
}

struct SecretHoldView: View {
    var hold: SecretHold
    var done: () -> Void

    var body: some View {
        NavigationStack {
            List(hold.fields, id: \.name) { field in
                VStack(alignment: .leading) {
                    Text(field.name).font(.caption)
                    Text(field.value).font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                }
            }
            .navigationTitle(hold.title)
            .toolbar {
                Button("Continue", action: done)
            }
        }
    }
}

struct SignInView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        NavigationStack {
            Form {
                if !model.records.isEmpty {
                    Section("Clusters") {
                        ForEach(model.records) { record in
                            Button(record.name.isEmpty ? record.url : record.name) {
                                Task { await model.open(record) }
                            }
                        }
                    }
                }
                Section("Cluster") {
                    TextField("URL", text: $model.clusterURL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Email", text: $model.email)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    SecureField("Password", text: $model.password)
                }
                if case .setup = model.phase {
                    Section("First operator") {
                        TextField("Cluster name", text: $model.clusterName)
                        TextField("Your name", text: $model.operatorName)
                        Button("Create cluster") { Task { await model.submitSetup() } }
                    }
                } else if case .otp = model.phase {
                    Section("Second factor") {
                        TextField(model.useRecovery ? "Recovery code" : "Authenticator code", text: $model.otp)
                        Toggle("Use a recovery code", isOn: $model.useRecovery)
                        Button("Continue") { Task { await model.submitCode() } }
                    }
                } else if case .enrollment(let url) = model.phase {
                    Section("Finish enrollment") {
                        Text("This operator still needs an authenticator and a passkey. Finish that in the browser, then come back and sign in with the current code.")
                        if let link = URL(string: url) {
                            Link("Open \(url)", destination: link)
                        }
                    }
                } else {
                    Section {
                        Button("Continue") { Task { await model.begin() } }
                        Button("Sign in") { Task { await model.signIn() } }
                    }
                }
                if !model.message.isEmpty {
                    Section { Text(model.message).foregroundStyle(.red) }
                }
            }
            .navigationTitle("Reactor")
        }
    }
}

struct ShellView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        NavigationStack {
            screen
                .navigationTitle(title)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Menu") { model.drawerOpen = true }
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Account") { model.accountOpen = true }
                    }
                }
        }
        .overlay(alignment: .leading) {
            if model.drawerOpen {
                DrawerMenu()
                    .frame(maxWidth: 300, maxHeight: .infinity)
                    .background(.background)
                    .shadow(radius: 8)
            }
        }
        .sheet(isPresented: $model.accountOpen) {
            AccountSheet()
        }
        .task { await model.refreshHome() }
    }

    @ViewBuilder private var screen: some View {
        if let user = userID {
            UserDetailView(userID: user)
        } else if let table = tableName {
            TableDetailView(table: table)
        } else {
            switch model.drawer.section ?? .projects {
            case .projects: ProjectsView()
            case .overview: OverviewView()
            case .keys: KeysView()
            case .authUsers: AuthUsersView()
            case .authProviders: ProvidersView()
            case .authMfa: MfaView()
            case .authEmail: EmailView()
            case .authTemplates: TemplatesView()
            case .data: DataView()
            case .sql: SqlView()
            case .storage: StorageView()
            case .functionOverview, .functionVersions, .functionLogs, .functionEnv, .functionTest:
                FunctionsView()
            case .queue: QueueView()
            case .siteOverview, .siteDeployments, .siteLogs, .siteEnv, .siteDomains:
                SitesView()
            case .logs: LogsView()
            case .team: TeamView()
            case .settings: SettingsView()
            case .cluster: ClusterView()
            case .operators: OperatorsView()
            case .serviceKeys: ServiceKeysView()
            case .agent: AgentView()
            }
        }
    }

    private var title: String {
        if userID != nil { return "User" }
        if let table = tableName { return table }
        return model.drawer.section.map { ScreenTitle.of($0) } ?? "Reactor"
    }

    private var userID: String? {
        for item in model.drawer.stack.reversed() {
            if case .user(let id) = item { return id }
        }
        return nil
    }

    private var tableName: String? {
        for item in model.drawer.stack.reversed() {
            if case .table(let name) = item { return name }
        }
        return nil
    }
}

enum ScreenTitle {
    static func of(_ screen: Screen) -> String {
        switch screen {
        case .projects: "Projects"
        case .overview: "Overview"
        case .keys: "API keys"
        case .authUsers: "Users"
        case .authProviders: "Providers"
        case .authMfa: "2FA"
        case .authEmail: "Email"
        case .authTemplates: "Templates"
        case .data: "Data"
        case .sql: "SQL"
        case .storage: "Storage"
        case .functionOverview: "Functions"
        case .functionVersions: "Versions"
        case .functionLogs: "Function logs"
        case .functionEnv: "Variables"
        case .functionTest: "Test"
        case .queue: "Queue"
        case .siteOverview: "Site"
        case .siteDeployments: "Deployments"
        case .siteLogs: "Site logs"
        case .siteEnv: "Site variables"
        case .siteDomains: "Domains"
        case .logs: "Logs"
        case .team: "Team"
        case .settings: "Settings"
        case .cluster: "Cluster"
        case .operators: "Console users"
        case .serviceKeys: "Service keys"
        case .agent: "Operator"
        }
    }
}

struct DrawerMenu: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        List {
            if model.projectRef != nil {
                Button("All projects") { model.leaveProject() }
            }
            ForEach(model.drawer.items()) { item in
                Button(item.title) { model.select(item.screen) }
            }
            if let section = model.drawer.section, !model.drawer.subitems(for: section).isEmpty {
                Section(section == .authUsers ? "Auth" : "Section") {
                    ForEach(model.drawer.subitems(for: section)) { item in
                        Button(item.title) { model.select(item.screen) }
                    }
                }
            }
        }
    }
}

struct AccountSheet: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        NavigationStack {
            List {
                ForEach(model.records) { record in
                    Button(record.url) { Task { await model.open(record) } }
                }
                Button("Service keys") {
                    model.accountOpen = false
                    model.select(.serviceKeys)
                }
                Button("Reactor Operator") {
                    model.accountOpen = false
                    model.select(.agent)
                }
                Button("Log out", role: .destructive) { Task { await model.logoutCurrent() } }
            }
            .navigationTitle("Account")
        }
    }
}

struct ProjectsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var name = ""

    var body: some View {
        List {
            Section {
                TextField("Project name", text: $name)
                Button("Create project") {
                    let project = name
                    name = ""
                    Task { await model.createProject(name: project) }
                }
                .disabled(name.isEmpty)
            }
            ForEach(model.projects.arrayValue ?? [], id: \.refText) { project in
                Button(project["name"]?.stringValue ?? project.refText) {
                    model.openProject(project.refText, role: project["role"]?.stringValue ?? "developer")
                }
            }
        }
    }
}

struct OverviewView: View {
    @EnvironmentObject private var model: AppModel
    @State private var bodyJSON: JSONValue = .null

    var body: some View {
        List {
            if let counts = bodyJSON["counts"]?.objectValue {
                ForEach(counts.keys.sorted(), id: \.self) { key in
                    LabeledContent(key, value: counts[key]?.intValue.map(String.init) ?? "")
                }
            }
            ForEach(bodyJSON["series"]?.objectValue?.keys.sorted() ?? [], id: \.self) { key in
                Text(key)
            }
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { bodyJSON = try await OverviewStore(client: client).load(pref: pref) }
    }
}

struct KeysView: View {
    @EnvironmentObject private var model: AppModel
    @State private var revealed = false

    var body: some View {
        Form {
            if let hold = model.secrets {
                ForEach(hold.fields, id: \.name) { field in
                    if field.name == "service_key" && !revealed {
                        LabeledContent(field.name, value: "Hidden")
                    } else {
                        Text(field.value).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                    }
                }
                Button(revealed ? "Hide service key" : "Reveal service key") { revealed.toggle() }
            } else {
                Text("Rotate to issue a new pair. The anon key from create stays on the secrets screen until you continue.")
            }
            Button("Rotate keys") { Task { await rotate() } }
                .disabled(!model.drawer.can(.rotateKeys))
        }
    }

    private func rotate() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let store = KeysStore(client: client)
            _ = try await store.rotate(pref: pref)
            model.secrets = store.hold
        }
    }
}

struct AuthUsersView: View {
    @EnvironmentObject private var model: AppModel
    @State private var users: JSONValue = .null

    var body: some View {
        List(users.arrayValue ?? [], id: \.userID) { user in
            Button(user["email"]?.stringValue ?? user.userID) {
                model.drawer.openUser(user.userID)
            }
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { users = try await AuthAdminStore(client: client).users(pref: pref) }
    }
}

struct UserDetailView: View {
    @EnvironmentObject private var model: AppModel
    var userID: String
    @State private var user: JSONValue = .null
    @State private var password = ""

    var body: some View {
        Form {
            LabeledContent("Email", value: user["email"]?.stringValue ?? "")
            LabeledContent("Verified", value: user["email_verified_at"]?.stringValue ?? "no")
            if model.drawer.can(.manageAuth) {
                SecureField("New password", text: $password)
                Button("Set password") { Task { await save() } }
                Button("Delete user", role: .destructive) { Task { await remove() } }
            }
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { user = try await AuthAdminStore(client: client).user(pref: pref, id: userID) }
    }

    private func save() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { try await AuthAdminStore(client: client).setPassword(pref: pref, id: userID, password: password) }
    }

    private func remove() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            try await AuthAdminStore(client: client).deleteUser(pref: pref, id: userID)
            model.drawer.select(.authUsers)
        }
    }
}

struct ProvidersView: View {
    @EnvironmentObject private var model: AppModel
    @State private var settings: JSONValue = .null
    @State private var providers: JSONValue = .null
    @State private var provider = "google"
    @State private var clientID = ""
    @State private var clientSecret = ""
    @State private var redirects = ""
    @State private var tenant = "common"
    @State private var teamID = ""
    @State private var keyID = ""
    @State private var privateKey = ""

    var body: some View {
        Form {
            Toggle("Require email verification", isOn: emailBinding)
                .disabled(!model.drawer.can(.manageAuth))
            Picker("Provider", selection: $provider) {
                ForEach(OAuthCatalog.providers, id: \.self, content: Text.init)
            }
            TextField("Client id", text: $clientID)
            SecureField("Client secret", text: $clientSecret)
            TextField("Redirects, one per line", text: $redirects, axis: .vertical)
            if provider == "microsoft" { TextField("Tenant", text: $tenant) }
            if provider == "apple" {
                TextField("Team id", text: $teamID)
                TextField("Key id", text: $keyID)
                TextField("Private key", text: $privateKey, axis: .vertical)
            }
            Button("Save provider") { Task { await save() } }
                .disabled(!model.drawer.can(.manageAuth))
            ForEach(providers.arrayValue ?? [], id: \.providerName) { item in
                Text(item.providerName)
            }
        }
        .task { await load() }
    }

    private var emailBinding: Binding<Bool> {
        Binding(
            get: { settings["require_email_verification"]?.boolValue ?? false },
            set: { value in Task { await setEmail(value) } }
        )
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let store = AuthAdminStore(client: client)
            settings = try await store.authSettings(pref: pref)
            providers = try await store.providers(pref: pref)
        }
    }

    private func setEmail(_ value: Bool) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            settings = try await AuthAdminStore(client: client).updateAuth(
                pref: pref,
                requireEmailVerification: value,
                requireMFA: settings["require_mfa"]?.boolValue ?? false
            )
        }
    }

    private func save() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        var extra: [String: String] = [:]
        if provider == "microsoft" { extra["tenant"] = tenant }
        if provider == "apple" {
            extra["team_id"] = teamID
            extra["key_id"] = keyID
            if !privateKey.isEmpty { extra["private_key"] = privateKey }
        }
        await model.run {
            providers = try await AuthAdminStore(client: client).upsertProvider(
                pref: pref,
                provider: provider,
                clientID: clientID,
                clientSecret: clientSecret,
                redirects: redirects.split(separator: "\n").map { String($0).trimmingCharacters(in: .whitespaces) },
                extra: extra
            )
        }
    }
}

struct MfaView: View {
    @EnvironmentObject private var model: AppModel
    @State private var settings: JSONValue = .null

    var body: some View {
        Form {
            Toggle("Require a second factor for password sign-in", isOn: binding)
                .disabled(!model.drawer.can(.manageAuth))
            Text("This applies to password sign-in. OAuth is unchanged.")
        }
        .task { await load() }
    }

    private var binding: Binding<Bool> {
        Binding(
            get: { settings["require_mfa"]?.boolValue ?? false },
            set: { value in Task { await save(value) } }
        )
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { settings = try await AuthAdminStore(client: client).authSettings(pref: pref) }
    }

    private func save(_ value: Bool) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            settings = try await AuthAdminStore(client: client).updateAuth(
                pref: pref,
                requireEmailVerification: settings["require_email_verification"]?.boolValue ?? false,
                requireMFA: value
            )
        }
    }
}

struct EmailView: View {
    @EnvironmentObject private var model: AppModel
    @State private var host = ""
    @State private var port = "587"
    @State private var username = ""
    @State private var password = ""
    @State private var from = ""
    @State private var tls = "starttls"
    @State private var linkBase = ""
    @State private var cluster = false
    @State private var testTo = ""

    var body: some View {
        Form {
            TextField("Host", text: $host)
            TextField("Port", text: $port)
            TextField("Username", text: $username)
            SecureField("Password", text: $password)
            TextField("From", text: $from)
            Picker("TLS", selection: $tls) {
                Text("STARTTLS").tag("starttls")
                Text("TLS").tag("tls")
                Text("None").tag("none")
            }
            TextField("Link base", text: $linkBase)
            if model.platformAdmin {
                Toggle("Use cluster SMTP when this project has no host", isOn: $cluster)
                    .onChange(of: cluster) { _, value in Task { await setCluster(value) } }
            }
            Button("Save") { Task { await save() } }.disabled(!model.drawer.can(.manageAuth))
            TextField("Send test to", text: $testTo)
            Button("Send test") { Task { await sendTest() } }.disabled(!model.drawer.can(.manageAuth))
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let body = try await AuthAdminStore(client: client).email(pref: pref)
            host = body["host"]?.stringValue ?? ""
            port = body["port"]?.intValue.map(String.init) ?? "587"
            username = body["username"]?.stringValue ?? ""
            from = body["from_address"]?.stringValue ?? ""
            tls = body["tls"]?.stringValue ?? "starttls"
            linkBase = body["link_base"]?.stringValue ?? ""
            cluster = body["cluster_smtp"]?.boolValue ?? false
        }
    }

    private func save() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            _ = try await AuthAdminStore(client: client).saveEmail(
                pref: pref,
                host: host,
                port: Int(port) ?? 587,
                username: username,
                password: password,
                fromAddress: from,
                tls: tls,
                linkBase: linkBase
            )
        }
    }

    private func setCluster(_ value: Bool) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { _ = try await AuthAdminStore(client: client).setClusterSMTP(pref: pref, enabled: value) }
    }

    private func sendTest() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { _ = try await AuthAdminStore(client: client).testEmail(pref: pref, to: testTo) }
    }
}

struct TemplatesView: View {
    @EnvironmentObject private var model: AppModel
    @State private var templates: JSONValue = .null
    @State private var name = "invite"
    @State private var subject = ""
    @State private var bodyText = ""
    @State private var bodyHTML = ""

    var body: some View {
        Form {
            ForEach(templates.arrayValue ?? [], id: \.templateName) { item in
                Button(item.templateName) {
                    name = item.templateName
                    Task { await loadOne() }
                }
            }
            TextField("Name", text: $name)
            TextField("Subject", text: $subject)
            TextField("Text", text: $bodyText, axis: .vertical)
            TextField("HTML", text: $bodyHTML, axis: .vertical)
            Text("Placeholders stay as typed: {{email}} {{token}} {{link}} {{code}}")
                .font(.caption)
            Button("Save") { Task { await save() } }.disabled(!model.drawer.can(.manageAuth))
            Button("Reset reserved") { Task { await reset() } }.disabled(!model.drawer.can(.manageAuth))
        }
        .task { await reload() }
    }

    private func reload() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { templates = try await AuthAdminStore(client: client).templates(pref: pref) }
    }

    private func loadOne() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let body = try await AuthAdminStore(client: client).template(pref: pref, name: name)
            subject = body["subject"]?.stringValue ?? ""
            bodyText = body["body_text"]?.stringValue ?? ""
            bodyHTML = body["body_html"]?.stringValue ?? ""
        }
    }

    private func save() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            _ = try await AuthAdminStore(client: client).saveTemplate(pref: pref, name: name, subject: subject, bodyText: bodyText, bodyHTML: bodyHTML)
            templates = try await AuthAdminStore(client: client).templates(pref: pref)
        }
    }

    private func reset() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { _ = try await AuthAdminStore(client: client).resetTemplate(pref: pref, name: name) }
    }
}
