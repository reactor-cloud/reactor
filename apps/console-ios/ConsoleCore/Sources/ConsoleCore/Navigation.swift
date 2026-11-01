import Foundation

public enum Screen: String, Codable, CaseIterable, Sendable, Hashable {
    case projects
    case overview
    case keys
    case authUsers
    case authProviders
    case authMfa
    case authEmail
    case authTemplates
    case data
    case sql
    case storage
    case functionOverview
    case functionVersions
    case functionLogs
    case functionEnv
    case functionTest
    case queue
    case siteOverview
    case siteDeployments
    case siteLogs
    case siteEnv
    case siteDomains
    case logs
    case team
    case settings
    case cluster
    case operators
    case serviceKeys
    case agent
}

public enum Destination: Equatable, Sendable {
    case section(Screen)
    case user(String)
    case table(String)
    case function(String)
}

public struct DrawerItem: Equatable, Sendable, Identifiable {
    public var id: Screen { screen }
    public var screen: Screen
    public var title: String

    public init(screen: Screen, title: String) {
        self.screen = screen
        self.title = title
    }
}

public enum ProjectRole: String, Codable, Sendable, Equatable {
    case developer
    case admin
    case owner
}

public enum Mutation: Equatable, Sendable {
    case editRows
    case runSql
    case upload
    case deployFunction
    case writeEnv
    case writeDomains
    case manageAuth
    case rotateKeys
    case addMember
    case createQueue
    case renameProject
    case deleteProject
    case manageCluster
    case manageOperators
}

public enum SettingsAvailability {
    public static let migrateAvailable = false
    public static let transferAvailable = false
}

public struct DrawerModel: Equatable, Sendable {
    public var projectRef: String?
    public var role: ProjectRole
    public var platformAdmin: Bool
    public var queueEnabled: Bool
    public var stack: [Destination]

    public init(
        projectRef: String? = nil,
        role: ProjectRole = .developer,
        platformAdmin: Bool = false,
        queueEnabled: Bool = false,
        stack: [Destination] = []
    ) {
        self.projectRef = projectRef
        self.role = role
        self.platformAdmin = platformAdmin
        self.queueEnabled = queueEnabled
        self.stack = stack
    }

    public func items() -> [DrawerItem] {
        guard projectRef != nil else {
            var home = [DrawerItem(screen: .projects, title: "Projects")]
            if platformAdmin {
                home.append(DrawerItem(screen: .cluster, title: "Cluster"))
                home.append(DrawerItem(screen: .operators, title: "Console users"))
            }
            return home
        }
        var items = [
            DrawerItem(screen: .overview, title: "Overview"),
            DrawerItem(screen: .keys, title: "API keys"),
            DrawerItem(screen: .authUsers, title: "Auth"),
            DrawerItem(screen: .data, title: "Data"),
            DrawerItem(screen: .sql, title: "SQL"),
            DrawerItem(screen: .storage, title: "Storage"),
            DrawerItem(screen: .functionOverview, title: "Functions"),
        ]
        if queueEnabled {
            items.append(DrawerItem(screen: .queue, title: "Queue"))
        }
        items.append(DrawerItem(screen: .logs, title: "Logs"))
        items.append(DrawerItem(screen: .team, title: "Team"))
        items.append(DrawerItem(screen: .settings, title: "Settings"))
        return items
    }

    public func subitems(for screen: Screen) -> [DrawerItem] {
        switch screen {
        case .authUsers, .authProviders, .authMfa, .authEmail, .authTemplates:
            return [
                DrawerItem(screen: .authUsers, title: "Users"),
                DrawerItem(screen: .authProviders, title: "Providers"),
                DrawerItem(screen: .authMfa, title: "2FA"),
                DrawerItem(screen: .authEmail, title: "Email"),
                DrawerItem(screen: .authTemplates, title: "Templates"),
            ]
        case .functionOverview, .functionVersions, .functionLogs, .functionEnv, .functionTest:
            return [
                DrawerItem(screen: .functionOverview, title: "Overview"),
                DrawerItem(screen: .functionVersions, title: "Versions"),
                DrawerItem(screen: .functionLogs, title: "Logs"),
                DrawerItem(screen: .functionEnv, title: "Variables"),
                DrawerItem(screen: .functionTest, title: "Test"),
            ]
        case .siteOverview, .siteDeployments, .siteLogs, .siteEnv, .siteDomains:
            return [
                DrawerItem(screen: .siteOverview, title: "Overview"),
                DrawerItem(screen: .siteDeployments, title: "Deployments"),
                DrawerItem(screen: .siteLogs, title: "Logs"),
                DrawerItem(screen: .siteEnv, title: "Variables"),
                DrawerItem(screen: .siteDomains, title: "Domains"),
            ]
        default:
            return []
        }
    }

    public mutating func select(_ screen: Screen) {
        if let name = functionName, isFunction(screen) {
            stack = [.section(screen), .function(name)]
            return
        }
        stack = [.section(screen)]
    }

    public mutating func openUser(_ id: String) {
        stack.append(.user(id))
    }

    public mutating func openTable(_ name: String) {
        stack.append(.table(name))
    }

    public mutating func openFunction(_ name: String) {
        stack.append(.function(name))
    }

    public var functionName: String? {
        for item in stack.reversed() {
            if case .function(let name) = item { return name }
        }
        return nil
    }

    public var section: Screen? {
        for item in stack {
            if case .section(let screen) = item { return screen }
        }
        return nil
    }

    public func can(_ mutation: Mutation) -> Bool {
        switch mutation {
        case .renameProject, .deleteProject:
            return role == .owner
        case .manageCluster, .manageOperators:
            return platformAdmin
        default:
            return role == .admin || role == .owner
        }
    }

    private func isFunction(_ screen: Screen) -> Bool {
        switch screen {
        case .functionOverview, .functionVersions, .functionLogs, .functionEnv, .functionTest:
            return true
        default:
            return false
        }
    }
}

public struct SecretField: Equatable, Sendable {
    public var name: String
    public var value: String

    public init(name: String, value: String) {
        self.name = name
        self.value = value
    }
}

public struct SecretHold: Equatable, Sendable {
    public var title: String
    public var fields: [SecretField]

    public init(title: String, fields: [SecretField]) {
        self.title = title
        self.fields = fields
    }
}

public enum OAuthCatalog {
    public static let providers = [
        "google", "microsoft", "apple", "github", "facebook", "discord", "x", "linkedin", "slack", "gitlab",
    ]
}

public enum ConsoleKeyScope {
    public static let all = [
        "projects.create",
        "projects.migrate",
        "projects.sql",
        "auth.settings",
        "auth.providers",
        "auth.email",
        "auth.users",
    ]
}

public struct MigrationRecord: Equatable, Sendable {
    public var version: String
    public var status: String
    public var revertable: Bool

    public init(version: String, status: String, revertable: Bool) {
        self.version = version
        self.status = status
        self.revertable = revertable
    }
}
