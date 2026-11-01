import ConsoleCore
import SwiftUI
import UniformTypeIdentifiers

struct DataView: View {
    @EnvironmentObject private var model: AppModel
    @State private var schemas: JSONValue = .null
    @State private var schema = ""
    @State private var tables: JSONValue = .null
    @State private var query = ""
    @State private var tableName = ""
    @State private var columnName = "title"
    @State private var columnType = "text"

    var body: some View {
        List {
            Picker("Schema", selection: $schema) {
                ForEach(schemaNames, id: \.self) { Text($0).tag($0) }
            }
            TextField("Search tables", text: $query)
            if model.drawer.can(.editRows) {
                TextField("New table", text: $tableName)
                TextField("Column", text: $columnName)
                Picker("Type", selection: $columnType) {
                    ForEach(["text", "uuid", "integer", "bigint", "boolean", "timestamptz", "jsonb", "numeric"], id: \.self, content: Text.init)
                }
                Button("Create table") { Task { await create() } }
            }
            ForEach(filteredTables, id: \.self) { name in
                Button(name) { model.drawer.openTable(name) }
            }
        }
        .task { await load() }
        .onChange(of: schema) { _, _ in Task { await loadTables() } }
    }

    private var schemaNames: [String] {
        (schemas.arrayValue ?? []).compactMap { $0["name"]?.stringValue }
    }

    private var filteredTables: [String] {
        let names = (tables.arrayValue ?? []).compactMap { $0["name"]?.stringValue }
        if query.isEmpty { return names }
        return names.filter { $0.localizedCaseInsensitiveContains(query) }
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            schemas = try await DataStore(client: client).schemas(pref: pref)
            if schema.isEmpty { schema = schemaNames.first ?? "" }
            tables = try await DataStore(client: client).schema(pref: pref, schema: schema)
        }
    }

    private func loadTables() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { tables = try await DataStore(client: client).schema(pref: pref, schema: schema) }
    }

    private func create() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            _ = try await DataStore(client: client).createTable(pref: pref, schema: schema, name: tableName, columns: [(columnName, columnType)])
            tables = try await DataStore(client: client).schema(pref: pref, schema: schema)
        }
    }
}

struct TableDetailView: View {
    @EnvironmentObject private var model: AppModel
    var table: String
    @State private var page: JSONValue = .null
    @State private var query = ""
    @State private var offset = 0
    @State private var insertJSON = "{\"title\":\"\"}"
    @State private var selected: JSONValue?

    var body: some View {
        List {
            TextField("Search rows", text: $query)
            Button("Search") { offset = 0; Task { await load() } }
            if model.drawer.can(.editRows) {
                TextField("Insert JSON object", text: $insertJSON, axis: .vertical)
                Button("Insert") { Task { await insert() } }
            }
            ForEach(page["rows"]?.arrayValue ?? [], id: \.rowKey) { row in
                Button(row.rowKey.isEmpty ? "row" : row.rowKey) { selected = row }
            }
            Text(footer)
            HStack {
                Button("Previous") { offset = max(0, offset - 50); Task { await load() } }.disabled(offset == 0)
                Button("Next") { offset += 50; Task { await load() } }
            }
        }
        .task { await load() }
        .sheet(item: selectedItem) { item in
            RowEditor(row: item.row, table: table) { Task { await load() } }
        }
    }

    private var footer: String {
        let total = page["total"]?.intValue ?? 0
        let from = total == 0 ? 0 : offset + 1
        let to = min(offset + 50, total)
        return "\(from)–\(to) of \(total)"
    }

    private var selectedItem: Binding<RowItem?> {
        Binding(get: { selected.map { RowItem(row: $0) } }, set: { selected = $0?.row })
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            page = try await DataStore(client: client).rows(pref: pref, table: table, schema: nil, query: query, offset: offset)
        }
    }

    private func insert() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        guard let data = insertJSON.data(using: .utf8), let values = try? JSONValue.parse(data).objectValue else { return }
        await model.run {
            _ = try await DataStore(client: client).insert(pref: pref, table: table, schema: nil, values: values)
            page = try await DataStore(client: client).rows(pref: pref, table: table, schema: nil, query: query, offset: offset)
        }
    }
}

struct RowItem: Identifiable {
    var id: String { row.rowKey }
    var row: JSONValue
}

struct RowEditor: View {
    @EnvironmentObject private var model: AppModel
    var row: JSONValue
    var table: String
    var changed: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var fields: [String: String] = [:]

    var body: some View {
        NavigationStack {
            Form {
                ForEach(fields.keys.sorted(), id: \.self) { key in
                    TextField(key, text: binding(key), axis: .vertical)
                        .disabled(key == "id" || !model.drawer.can(.editRows))
                }
                if model.drawer.can(.editRows) {
                    Button("Save") { Task { await save() } }
                    Button("Duplicate") { Task { await duplicate() } }
                    Button("Delete", role: .destructive) { Task { await remove() } }
                }
            }
            .navigationTitle("Row")
            .onAppear {
                fields = row.objectValue?.mapValues { $0.descriptionText } ?? [:]
            }
        }
    }

    private static func cell(_ original: JSONValue?, _ text: String) -> JSONValue {
        switch original {
        case .int:
            return .int(Int(text) ?? 0)
        case .double:
            return .double(Double(text) ?? 0)
        case .bool:
            return .bool(text == "true")
        default:
            return .string(text)
        }
    }

    private func binding(_ key: String) -> Binding<String> {
        Binding(get: { fields[key] ?? "" }, set: { fields[key] = $0 })
    }

    private func save() async {
        guard let client = model.client, let pref = model.projectRef, let pk = row["id"] else { return }
        await model.run {
            let store = DataStore(client: client)
            for (key, value) in fields where key != "id" && value != (row[key]?.descriptionText ?? "") {
                _ = try await store.updateCell(pref: pref, table: table, schema: nil, pk: pk, column: key, value: Self.cell(row[key], value))
            }
            dismiss()
            changed()
        }
    }

    private func duplicate() async {
        guard let client = model.client, let pref = model.projectRef, let values = row.objectValue else { return }
        await model.run {
            _ = try await DataStore(client: client).duplicate(pref: pref, table: table, schema: nil, primaryKey: "id", values: values)
            dismiss()
            changed()
        }
    }

    private func remove() async {
        guard let client = model.client, let pref = model.projectRef, let pk = row["id"] else { return }
        await model.run {
            try await DataStore(client: client).deleteRow(pref: pref, table: table, schema: nil, pk: pk)
            dismiss()
            changed()
        }
    }
}

struct SqlView: View {
    @EnvironmentObject private var model: AppModel
    @State private var sql = "SELECT 1;"
    @State private var readOnly = true
    @State private var version = "0004_change.sql"
    @State private var down = ""
    @State private var result = ""
    @State private var pendingConfirm = false
    @State private var migrations: JSONValue = .null

    var body: some View {
        Form {
            TextEditor(text: $sql).frame(minHeight: 160).font(.system(.body, design: .monospaced))
            Toggle("Read only", isOn: $readOnly)
            if model.drawer.can(.runSql) {
                Button("Run") { Task { await run(confirm: false) } }
                if pendingConfirm { Button("Confirm and run") { Task { await run(confirm: true) } } }
                TextField("Migration version", text: $version)
                TextField("Down SQL", text: $down, axis: .vertical)
                Button("Run as migration") { Task { await migrate() } }
            } else {
                Text("Owners and admins can run SQL.")
            }
            Text(result).font(.system(.footnote, design: .monospaced))
            ForEach(migrations.arrayValue ?? [], id: \.templateName) { item in
                VStack(alignment: .leading) {
                    Text(item.templateName)
                    Text(item["status"]?.stringValue ?? "").font(.caption)
                    if item["revertable"]?.boolValue == true && item["status"]?.stringValue == "applied" && model.drawer.can(.runSql) {
                        Button("Revert") { Task { await revert(item) } }
                    }
                }
            }
        }
        .task { await loadMigrations() }
    }

    private func store(_ client: ConsoleClient) -> SqlStore {
        let role = model.role
        var sqlStore = SqlStore(client: client, role: role, readOnly: readOnly)
        sqlStore.readOnly = readOnly
        return sqlStore
    }

    private func run(confirm: Bool) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let outcome = confirm
                ? try await store(client).confirm(pref: pref, sql: sql)
                : try await store(client).run(pref: pref, sql: sql)
            switch outcome {
            case .confirmation(let warnings):
                pendingConfirm = true
                result = warnings.joined(separator: "\n")
            case .value(let value):
                pendingConfirm = false
                result = value.display
            }
        }
    }

    private func migrate() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let value = try await store(client).migrate(pref: pref, version: version, sql: sql, downSQL: down.isEmpty ? nil : down)
            result = value.display
            migrations = try await store(client).migrations(pref: pref)
        }
    }

    private func loadMigrations() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { migrations = try await SqlStore(client: client, role: model.role).migrations(pref: pref) }
    }

    private func revert(_ item: JSONValue) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        let record = MigrationRecord(
            version: item.templateName,
            status: item["status"]?.stringValue ?? "",
            revertable: item["revertable"]?.boolValue ?? false
        )
        await model.run {
            _ = try await store(client).revert(pref: pref, migration: record)
            migrations = try await store(client).migrations(pref: pref)
        }
    }
}

struct StorageView: View {
    @EnvironmentObject private var model: AppModel
    @State private var objects: JSONValue = .null
    @State private var buckets: JSONValue = .null
    @State private var bucket = "files"
    @State private var isPublic = false
    @State private var importing = false
    @State private var linkURL = ""

    var body: some View {
        List {
            if model.drawer.can(.upload) {
                TextField("Bucket", text: $bucket)
                Toggle("Public", isOn: $isPublic)
                Button("Create bucket") { Task { await createBucket() } }
                Button("Upload") { importing = true }
            }
            if !linkURL.isEmpty {
                Text(linkURL).font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
            }
            ForEach(buckets.arrayValue ?? [], id: \.templateName) { item in
                Text("\(item.templateName) \(item["public"]?.boolValue == true ? "public" : "private")")
            }
            ForEach(objects.arrayValue ?? [], id: \.rowKey) { item in
                VStack(alignment: .leading) {
                    Text(item["key"]?.stringValue ?? "")
                    if let key = item["key"]?.stringValue {
                        Button("Link") { Task { await link(key) } }
                        if model.drawer.can(.upload) {
                            Button("Delete") { Task { await remove(key) } }
                        }
                    }
                }
            }
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.data]) { result in
            if case .success(let url) = result { Task { await upload(url) } }
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let store = StorageStore(client: client)
            objects = try await store.objects(pref: pref)
            buckets = try await store.buckets(pref: pref)
        }
    }

    private func createBucket() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            _ = try await StorageStore(client: client).createBucket(pref: pref, name: bucket, isPublic: isPublic)
            buckets = try await StorageStore(client: client).buckets(pref: pref)
        }
    }

    private func upload(_ url: URL) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        guard url.startAccessingSecurityScopedResource() else { return }
        defer { url.stopAccessingSecurityScopedResource() }
        guard let data = try? Data(contentsOf: url) else { return }
        await model.run {
            _ = try await StorageStore(client: client).upload(pref: pref, path: url.lastPathComponent, bytes: data, contentType: "application/octet-stream")
            objects = try await StorageStore(client: client).objects(pref: pref)
        }
    }

    private func link(_ key: String) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let issued = try await StorageStore(client: client).objectURL(pref: pref, key: key)
            linkURL = issued["url"]?.stringValue ?? ""
        }
    }

    private func remove(_ key: String) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            try await StorageStore(client: client).deleteObject(pref: pref, key: key)
            objects = try await StorageStore(client: client).objects(pref: pref)
        }
    }
}

struct FunctionsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var functions: JSONValue = .null
    @State private var name = ""
    @State private var promote = true
    @State private var importing = false
    @State private var detail: JSONValue = .null
    @State private var envKey = ""
    @State private var envValue = ""
    @State private var payload = "{}"
    @State private var output = ""
    @State private var version = "1"

    var body: some View {
        Form {
            TextField("Search", text: $name)
            if model.drawer.can(.deployFunction) {
                Toggle("Promote", isOn: $promote)
                Button("Add zip") { importing = true }
            }
            ForEach(Array(filtered.enumerated()), id: \.offset) { _, item in
                Button(item.templateName) {
                    model.drawer.openFunction(item.templateName)
                    name = item.templateName
                    Task { await loadDetail() }
                }
            }
            if let function = model.drawer.functionName {
                Picker("Section", selection: section) {
                    ForEach(model.drawer.subitems(for: .functionOverview)) { item in
                        Text(item.title).tag(item.screen)
                    }
                }
                switch model.drawer.section {
                case .functionLogs:
                    Text(detail.display)
                case .functionEnv:
                    TextField("Key", text: $envKey)
                    SecureField("Value", text: $envValue)
                    Button("Save variable") { Task { await saveEnv(function) } }.disabled(!model.drawer.can(.writeEnv))
                    ForEach(detail.arrayValue ?? [], id: \.rowKey) { item in
                        Text("\(item["key"]?.stringValue ?? "") secret")
                    }
                case .functionTest:
                    TextField("JSON", text: $payload, axis: .vertical)
                    Button("Invoke") { Task { await invoke(function) } }
                    Text(output).font(.system(.footnote, design: .monospaced))
                case .functionVersions:
                    TextField("Version to pin", text: $version)
                    Button("Promote") { Task { await promote(function) } }.disabled(!model.drawer.can(.deployFunction))
                    Button("Demote") { Task { await demote(function) } }.disabled(!model.drawer.can(.deployFunction))
                    Text(detail.display)
                default:
                    Text(detail.display)
                }
            }
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.zip, .data]) { result in
            if case .success(let url) = result { Task { await deploy(url) } }
        }
        .task { await load() }
    }

    private var section: Binding<Screen> {
        Binding(get: { model.drawer.section ?? .functionOverview }, set: { model.select($0) })
    }

    private var filtered: [JSONValue] {
        let rows = functions.arrayValue ?? []
        if name.isEmpty || model.drawer.functionName != nil { return rows }
        return rows.filter { ($0["name"]?.stringValue ?? "").localizedCaseInsensitiveContains(name) }
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { functions = try await FunctionsStore(client: client).list(pref: pref) }
    }

    private func loadDetail() async {
        guard let client = model.client, let pref = model.projectRef, let function = model.drawer.functionName else { return }
        await model.run {
            switch model.drawer.section {
            case .functionLogs:
                detail = try await LogsStore(client: client).search(pref: pref, query: function)
            case .functionEnv:
                detail = try await FunctionsStore(client: client).env(pref: pref, name: function)
            case .functionOverview:
                detail = try await FunctionsStore(client: client).stats(pref: pref, name: function)
            default:
                detail = functions
            }
        }
    }

    private func deploy(_ url: URL) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        guard url.startAccessingSecurityScopedResource() else { return }
        defer { url.stopAccessingSecurityScopedResource() }
        guard let data = try? Data(contentsOf: url) else { return }
        let function = name.isEmpty ? url.deletingPathExtension().lastPathComponent : name
        await model.run {
            _ = try await FunctionsStore(client: client).deploy(pref: pref, name: function, zip: data, promote: promote)
            functions = try await FunctionsStore(client: client).list(pref: pref)
        }
    }

    private func saveEnv(_ function: String) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            _ = try await FunctionsStore(client: client).setEnv(pref: pref, name: function, key: envKey, value: envValue)
            detail = try await FunctionsStore(client: client).env(pref: pref, name: function)
        }
    }

    private func invoke(_ function: String) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let body = try await FunctionsStore(client: client).invoke(pref: pref, name: function, version: nil, payload: Data(payload.utf8))
            output = body.display
        }
    }

    private func promote(_ function: String) async {
        guard let client = model.client, let pref = model.projectRef, let pin = Int(version) else { return }
        await model.run { _ = try await FunctionsStore(client: client).promote(pref: pref, name: function, version: pin) }
    }

    private func demote(_ function: String) async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { _ = try await FunctionsStore(client: client).demote(pref: pref, name: function) }
    }
}

struct QueueView: View {
    @EnvironmentObject private var model: AppModel
    @State private var queues: JSONValue = .null
    @State private var name = ""
    @State private var selected = ""
    @State private var message = "{\"ok\":true}"
    @State private var peeked: JSONValue = .null
    @State private var subs: JSONValue = .null

    var body: some View {
        Form {
            TextField("Queue name", text: $name)
            Button("Create") { Task { await create() } }.disabled(!model.drawer.can(.createQueue))
            ForEach(queues.arrayValue ?? [], id: \.templateName) { item in
                Button(item.templateName) {
                    selected = item.templateName
                    Task { await open() }
                }
            }
            if !selected.isEmpty {
                TextField("Message", text: $message, axis: .vertical)
                Button("Send") { Task { await send() } }.disabled(!model.drawer.can(.createQueue))
                Text("Messages").font(.headline)
                Text(peeked.display).font(.system(.footnote, design: .monospaced))
                Text("Subscribers").font(.headline)
                Text(subs.display).font(.system(.footnote, design: .monospaced))
            }
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef, model.queueEnabled else { return }
        await model.run { queues = try await QueueStore(client: client, enabled: true).list(pref: pref) }
    }

    private func create() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            _ = try await QueueStore(client: client, enabled: true).create(pref: pref, name: name)
            queues = try await QueueStore(client: client, enabled: true).list(pref: pref)
        }
    }

    private func open() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let store = QueueStore(client: client, enabled: true)
            peeked = try await store.peek(pref: pref, name: selected)
            subs = try await store.subscriptions(pref: pref, name: selected)
        }
    }

    private func send() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        let parsed = (try? JSONValue.parse(Data(message.utf8))) ?? .string(message)
        await model.run {
            _ = try await QueueStore(client: client, enabled: true).send(pref: pref, name: selected, message: parsed, delaySecs: 0)
            peeked = try await QueueStore(client: client, enabled: true).peek(pref: pref, name: selected)
        }
    }
}

struct SitesView: View {
    @EnvironmentObject private var model: AppModel
    @State private var bodyJSON: JSONValue = .null
    @State private var key = ""
    @State private var value = ""
    @State private var visibility = "secret"
    @State private var host = ""

    var body: some View {
        Form {
            Picker("Section", selection: section) {
                ForEach(model.drawer.subitems(for: .siteOverview)) { item in
                    Text(item.title).tag(item.screen)
                }
            }
            switch model.drawer.section {
            case .siteEnv:
                TextField("Key", text: $key)
                TextField("Value", text: $value)
                Picker("Visibility", selection: $visibility) {
                    Text("Secret").tag("secret")
                    Text("Visible").tag("visible")
                }
                Button("Save") { Task { await saveEnv() } }.disabled(!model.drawer.can(.writeEnv))
            case .siteDomains:
                TextField("Host", text: $host)
                Button("Add domain") { Task { await addDomain() } }.disabled(!model.drawer.can(.writeDomains))
                Button("Verify") { Task { await verifyDomain() } }.disabled(!model.drawer.can(.writeDomains))
                Button("Remove") { Task { await removeDomain() } }.disabled(!model.drawer.can(.writeDomains))
            default:
                EmptyView()
            }
            Text(bodyJSON.display).font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
        }
        .task { await load() }
        .onChange(of: model.drawer.section) { _, _ in Task { await load() } }
    }

    private var section: Binding<Screen> {
        Binding(get: { model.drawer.section ?? .siteOverview }, set: { model.select($0) })
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            let store = SitesStore(client: client)
            switch model.drawer.section {
            case .siteDeployments: bodyJSON = try await store.deployments(pref: pref)
            case .siteLogs: bodyJSON = try await LogsStore(client: client).search(pref: pref, query: "")
            case .siteEnv: bodyJSON = try await store.env(pref: pref)
            case .siteDomains: bodyJSON = try await store.domains(pref: pref)
            default:
                let site = try await store.overview(pref: pref)
                let stats = try await store.stats(pref: pref)
                bodyJSON = .object(["site": site, "stats": stats])
            }
        }
    }

    private func saveEnv() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            _ = try await SitesStore(client: client).setEnv(pref: pref, key: key, value: value, visibility: visibility)
            bodyJSON = try await SitesStore(client: client).env(pref: pref)
        }
    }

    private func addDomain() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            bodyJSON = try await SitesStore(client: client).addDomain(pref: pref, host: host)
        }
    }

    private func verifyDomain() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            try await SitesStore(client: client).verifyDomain(pref: pref, host: host)
            bodyJSON = try await SitesStore(client: client).domains(pref: pref)
        }
    }

    private func removeDomain() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            try await SitesStore(client: client).deleteDomain(pref: pref, host: host)
            bodyJSON = try await SitesStore(client: client).domains(pref: pref)
        }
    }
}

struct LogsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var query = ""
    @State private var rows: JSONValue = .null
    @State private var selected: JSONValue?

    var body: some View {
        List {
            TextField("Search", text: $query)
            Button("Search") { Task { await load() } }
            ForEach(rows.arrayValue ?? [], id: \.rowKey) { row in
                Button("\(row["kind"]?.stringValue ?? "") \(row["status"]?.intValue.map(String.init) ?? "")") {
                    selected = row
                }
            }
        }
        .task { await load() }
        .sheet(item: item) { row in
            Text(row.value.display).textSelection(.enabled).padding()
        }
    }

    private var item: Binding<LogItem?> {
        Binding(get: { selected.map { LogItem(value: $0) } }, set: { selected = $0?.value })
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { rows = try await LogsStore(client: client).search(pref: pref, query: query) }
    }
}

struct LogItem: Identifiable {
    var id: String { value["id"]?.stringValue ?? value.rowKey }
    var value: JSONValue
}

struct TeamView: View {
    @EnvironmentObject private var model: AppModel
    @State private var members: JSONValue = .null
    @State private var email = ""
    @State private var person = ""
    @State private var password = ""
    @State private var role = "developer"

    var body: some View {
        Form {
            ForEach(members.arrayValue ?? [], id: \.refText) { member in
                LabeledContent(member["email"]?.stringValue ?? "", value: member["role"]?.stringValue ?? "")
            }
            TextField("Email", text: $email)
            TextField("Name", text: $person)
            SecureField("Password", text: $password)
            Picker("Role", selection: $role) {
                Text("Developer").tag("developer")
                Text("Admin").tag("admin")
            }
            Button("Add member") { Task { await add() } }.disabled(!model.drawer.can(.addMember))
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { members = try await TeamStore(client: client).list(pref: pref) }
    }

    private func add() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            _ = try await TeamStore(client: client).add(pref: pref, email: email, name: person, password: password, role: role)
            members = try await TeamStore(client: client).list(pref: pref)
        }
    }
}

struct SettingsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var name = ""
    @State private var confirm = ""

    var body: some View {
        Form {
            TextField("Project name", text: $name)
            Button("Rename") { Task { await rename() } }.disabled(!model.drawer.can(.renameProject))
            Button("Reset database role") { Task { await reset() } }.disabled(!model.drawer.can(.runSql))
            Button("Migrate") {}.disabled(!SettingsAvailability.migrateAvailable)
            Button("Transfer") {}.disabled(!SettingsAvailability.transferAvailable)
            TextField("Type the project name to delete", text: $confirm)
            Button("Delete project", role: .destructive) { Task { await remove() } }
                .disabled(!model.drawer.can(.deleteProject) || confirm != name)
        }
    }

    private func rename() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { _ = try await ProjectsStore(client: client).rename(pref: pref, name: name) }
    }

    private func reset() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run { _ = try await SqlStore(client: client, role: model.role).resetDatabase(pref: pref) }
    }

    private func remove() async {
        guard let client = model.client, let pref = model.projectRef else { return }
        await model.run {
            try await ProjectsStore(client: client).delete(pref: pref)
            model.leaveProject()
            await model.refreshHome()
        }
    }
}

struct ClusterView: View {
    @EnvironmentObject private var model: AppModel
    @State private var info: JSONValue = .null
    @State private var name = ""
    @State private var host = ""
    @State private var port = "587"
    @State private var username = ""
    @State private var password = ""
    @State private var from = ""
    @State private var tls = "starttls"

    var body: some View {
        Form {
            TextField("Cluster name", text: $name)
            Button("Rename") { Task { await rename() } }.disabled(!model.drawer.can(.manageCluster))
            Text(info.display).font(.system(.footnote, design: .monospaced))
            TextField("SMTP host", text: $host)
            TextField("Port", text: $port)
            TextField("Username", text: $username)
            SecureField("Password", text: $password)
            TextField("From", text: $from)
            Picker("TLS", selection: $tls) {
                Text("STARTTLS").tag("starttls")
                Text("TLS").tag("tls")
                Text("None").tag("none")
            }
            Button("Save mail") { Task { await saveMail() } }.disabled(!model.drawer.can(.manageCluster))
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client else { return }
        await model.run {
            let store = ClusterStore(client: client)
            info = try await store.load()
            name = info["name"]?.stringValue ?? ""
            let mail = try await store.email()
            host = mail["host"]?.stringValue ?? ""
            port = mail["port"]?.intValue.map(String.init) ?? "587"
            username = mail["username"]?.stringValue ?? ""
            from = mail["from_address"]?.stringValue ?? ""
            tls = mail["tls"]?.stringValue ?? "starttls"
        }
    }

    private func rename() async {
        guard let client = model.client else { return }
        await model.run { info = try await ClusterStore(client: client).rename(name: name) }
    }

    private func saveMail() async {
        guard let client = model.client else { return }
        await model.run {
            _ = try await ClusterStore(client: client).saveEmail(
                host: host, port: Int(port) ?? 587, username: username, password: password, fromAddress: from, tls: tls
            )
        }
    }
}

struct OperatorsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var people: JSONValue = .null
    @State private var name = ""
    @State private var email = ""
    @State private var password = ""
    @State private var admin = false
    @State private var selected = ""
    @State private var pref = ""
    @State private var role = "developer"

    var body: some View {
        Form {
            TextField("Name", text: $name)
            TextField("Email", text: $email)
            SecureField("Password", text: $password)
            Toggle("Cluster admin", isOn: $admin)
            Button("Create") { Task { await create() } }.disabled(!model.drawer.can(.manageOperators))
            ForEach(people.arrayValue ?? [], id: \.userID) { person in
                Button(person["email"]?.stringValue ?? person.userID) { selected = person.userID }
            }
            if !selected.isEmpty {
                Button("Toggle cluster admin") { Task { await toggle() } }
                TextField("Project ref", text: $pref)
                Picker("Role", selection: $role) {
                    Text("Owner").tag("owner")
                    Text("Admin").tag("admin")
                    Text("Developer").tag("developer")
                }
                Button("Grant") { Task { await grant() } }
                Button("Remove access") { Task { await revoke() } }
                Button("Delete user", role: .destructive) { Task { await remove() } }
                    .disabled(selected == model.operatorID)
            }
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client else { return }
        await model.run { people = try await OperatorsStore(client: client).list() }
    }

    private func create() async {
        guard let client = model.client else { return }
        await model.run {
            _ = try await OperatorsStore(client: client).create(name: name, email: email, password: password, platformAdmin: admin)
            people = try await OperatorsStore(client: client).list()
        }
    }

    private func toggle() async {
        guard let client = model.client else { return }
        let current = people.arrayValue?.first { $0.userID == selected }?["platform_admin"]?.boolValue ?? false
        await model.run {
            try await OperatorsStore(client: client).setPlatformAdmin(id: selected, enabled: !current)
            people = try await OperatorsStore(client: client).list()
        }
    }

    private func grant() async {
        guard let client = model.client else { return }
        await model.run { try await OperatorsStore(client: client).grant(id: selected, pref: pref, role: role) }
    }

    private func revoke() async {
        guard let client = model.client else { return }
        await model.run { try await OperatorsStore(client: client).revoke(id: selected, pref: pref) }
    }

    private func remove() async {
        guard let client = model.client else { return }
        await model.run {
            try await OperatorsStore(client: client).delete(id: selected, selfID: model.operatorID)
            people = try await OperatorsStore(client: client).list()
        }
    }
}

struct ServiceKeysView: View {
    @EnvironmentObject private var model: AppModel
    @State private var keys: JSONValue = .null
    @State private var name = ""
    @State private var scopes = Set(ConsoleKeyScope.all)

    var body: some View {
        Form {
            TextField("Name", text: $name)
            ForEach(ConsoleKeyScope.all, id: \.self) { scope in
                Toggle(scope, isOn: Binding(get: { scopes.contains(scope) }, set: { on in
                    if on { scopes.insert(scope) } else { scopes.remove(scope) }
                }))
            }
            Button("Create key") { Task { await create() } }
            ForEach(keys.arrayValue ?? [], id: \.userID) { key in
                HStack {
                    Text(key["name"]?.stringValue ?? key.userID)
                    Spacer()
                    Button("Revoke", role: .destructive) { Task { await revoke(key.userID) } }
                }
            }
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client else { return }
        await model.run { keys = try await ServiceKeysStore(client: client).list() }
    }

    private func create() async {
        guard let client = model.client else { return }
        await model.run {
            let store = ServiceKeysStore(client: client)
            _ = try await store.create(name: name, scopes: ConsoleKeyScope.all.filter { scopes.contains($0) })
            model.secrets = store.hold
            keys = try await store.list()
        }
    }

    private func revoke(_ id: String) async {
        guard let client = model.client else { return }
        await model.run {
            try await ServiceKeysStore(client: client).revoke(id: id)
            keys = try await ServiceKeysStore(client: client).list()
        }
    }
}

struct AgentView: View {
    @EnvironmentObject private var model: AppModel
    @State private var models: JSONValue = .null
    @State private var threads: JSONValue = .null
    @State private var threadID = ""
    @State private var text = ""
    @State private var modelID = ""
    @State private var transcript: JSONValue = .null

    var body: some View {
        Form {
            Picker("Model", selection: $modelID) {
                Text("Default").tag("")
                ForEach(models["models"]?.arrayValue ?? [], id: \.userID) { item in
                    Text(item["name"]?.stringValue ?? item.userID).tag(item.userID)
                }
            }
            ForEach(threads.arrayValue ?? [], id: \.userID) { item in
                Button(item["preview"]?.stringValue ?? item.userID) {
                    threadID = item.userID
                    Task { await refresh() }
                }
            }
            Button("New conversation") { Task { await create() } }
            TextField("Message", text: $text, axis: .vertical)
            Button("Send") { Task { await send() } }.disabled(text.isEmpty || threadID.isEmpty)
            Text(transcript.display).font(.system(.footnote, design: .monospaced))
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.client else { return }
        await model.run {
            let store = AgentStore(client: client)
            models = (try? await store.models()) ?? .null
            threads = try await store.threads()
        }
    }

    private func create() async {
        guard let client = model.client else { return }
        await model.run {
            let created = try await AgentStore(client: client).createThread(projectRef: model.projectRef)
            threadID = created["id"]?.stringValue ?? ""
            threads = try await AgentStore(client: client).threads()
        }
    }

    private func send() async {
        guard let client = model.client else { return }
        let message = text
        text = ""
        await model.run {
            let store = AgentStore(client: client)
            _ = try await store.post(id: threadID, text: message, model: modelID.isEmpty ? nil : modelID)
            transcript = try await store.poll(id: threadID, maxPolls: 8)
        }
    }

    private func refresh() async {
        guard let client = model.client else { return }
        await model.run { transcript = try await AgentStore(client: client).thread(id: threadID) }
    }
}

extension JSONValue {
    var display: String {
        switch self {
        case .null: return ""
        default:
            return (try? String(decoding: encoded(), as: UTF8.self)) ?? ""
        }
    }
}
