pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "reactor"
include(":reactor-client")
if (System.getenv("REACTOR_CLIENT_ONLY") != "1") {
    include(":app")
    project(":app").projectDir = file("../../examples/todos-android/app")
}
