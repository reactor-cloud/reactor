import SwiftUI
import UniformTypeIdentifiers
import Reactor

struct TodoItem: Identifiable {
    var id: String
    var title: String
}

@MainActor
final class TodosModel: ObservableObject {
    let client: ReactorClient
    @Published var email = ""
    @Published var password = ""
    @Published var draft = ""
    @Published var items: [TodoItem] = []
    @Published var error = ""
    @Published var ping = ""
    @Published var fileNote = ""
    @Published var signedIn = false

    init() {
        let url = Bundle.main.object(forInfoDictionaryKey: "REACTOR_URL") as? String ?? "http://127.0.0.1:18000"
        let key = Bundle.main.object(forInfoDictionaryKey: "REACTOR_ANON_KEY") as? String ?? ""
        client = ReactorClient(url: url, anonKey: key, sessionStore: KeychainSessionStore(service: "lab.reactor.todos"))
        signedIn = client.auth.getSession() != nil
    }

    func signUp() async {
        await run {
            _ = try await client.auth.signUp(email: email, password: password)
            signedIn = true
            await load()
        }
    }

    func signIn() async {
        await run {
            _ = try await client.auth.signInWithPassword(email: email, password: password)
            signedIn = true
            await load()
        }
    }

    func signOut() async {
        await run {
            try await client.auth.signOut()
            signedIn = false
            items = []
        }
    }

    func load() async {
        guard let user = client.auth.getSession()?.user else { return }
        await run {
            let rows = try await client.from("todos").select().eq("user_id", user.id).order("created_at", ascending: false).execute().array() ?? []
            items = rows.compactMap { row in
                guard let id = row["id"]?.string(), let title = row["title"]?.string() else { return nil }
                return TodoItem(id: id, title: title)
            }
        }
    }

    func add() async {
        guard let user = client.auth.getSession()?.user else { return }
        let title = draft
        await run {
            _ = try await client.from("todos").insert(["title": .string(title), "user_id": .string(user.id)]).select().execute()
            draft = ""
            await load()
        }
    }

    func save(_ item: TodoItem, title: String) async {
        await run {
            _ = try await client.from("todos").update(["title": .string(title)]).eq("id", item.id).select().execute()
            await load()
        }
    }

    func remove(_ item: TodoItem) async {
        await run {
            _ = try await client.from("todos").delete().eq("id", item.id).select().execute()
            await load()
        }
    }

    func upload(_ url: URL) async {
        await run {
            guard url.startAccessingSecurityScopedResource() else { return }
            defer { url.stopAccessingSecurityScopedResource() }
            let data = try Data(contentsOf: url)
            let name = url.lastPathComponent
            try await client.storage.from("files").upload(path: name, data: data)
            let back = try await client.storage.from("files").download(path: name)
            fileNote = "Uploaded \(name) (\(back.count) bytes)"
        }
    }

    func callPing() async {
        await run {
            let body = try await client.functions.invoke("ping")
            ping = body["ok"]?.bool() == true ? "ok" : "failed"
        }
    }

    private func run(_ work: () async throws -> Void) async {
        do {
            error = ""
            try await work()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

struct ContentView: View {
    @StateObject private var model = TodosModel()
    @State private var importing = false

    var body: some View {
        NavigationStack {
            Form {
                if model.signedIn {
                    Section("Todo") {
                        TextField("New todo", text: $model.draft)
                        Button("Add") { Task { await model.add() } }
                    }
                    Section("List") {
                        ForEach(model.items) { item in
                            TodoRow(item: item, model: model)
                        }
                    }
                    Section("File") {
                        Button("Upload") { importing = true }
                        if !model.fileNote.isEmpty { Text(model.fileNote) }
                    }
                    Section("Function") {
                        Button("Call ping function") { Task { await model.callPing() } }
                        if !model.ping.isEmpty { Text(model.ping) }
                    }
                    Button("Log out") { Task { await model.signOut() } }
                } else {
                    Section("Create an account") {
                        TextField("Email", text: $model.email)
                            .textInputAutocapitalization(.never)
                        SecureField("Password", text: $model.password)
                        Button("Sign up") { Task { await model.signUp() } }
                    }
                    Section("Log in") {
                        Button("Log in") { Task { await model.signIn() } }
                    }
                }
                if !model.error.isEmpty {
                    Text(model.error).foregroundStyle(.red)
                }
            }
            .navigationTitle("Todos")
            .fileImporter(isPresented: $importing, allowedContentTypes: [.data]) { result in
                if case .success(let url) = result {
                    Task { await model.upload(url) }
                }
            }
            .task {
                if model.signedIn { await model.load() }
            }
        }
    }
}

struct TodoRow: View {
    let item: TodoItem
    @ObservedObject var model: TodosModel
    @State private var title: String

    init(item: TodoItem, model: TodosModel) {
        self.item = item
        self.model = model
        _title = State(initialValue: item.title)
    }

    var body: some View {
        VStack(alignment: .leading) {
            Text(item.title)
            TextField("Edit title", text: $title)
            HStack {
                Button("Save") { Task { await model.save(item, title: title) } }
                Button("Delete") { Task { await model.remove(item) } }
            }
        }
    }
}
