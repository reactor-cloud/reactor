plugins {
    kotlin("jvm")
    `maven-publish`
    signing
}

group = "sl.atomicollabs.reactor"
version = "1.26.9-beta.1"

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(17))
    }
    withSourcesJar()
}

dependencies {
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.json:json:20240303")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    testImplementation("junit:junit:4.13.2")
}

publishing {
    publications {
        create<MavenPublication>("maven") {
            from(components["java"])
            pom {
                name.set("reactor-client")
                description.set("Reactor client for the JVM")
                url.set("https://github.com/Reactor/reactor-kotlin")
                licenses {
                    license {
                        name.set("Business Source License 1.1")
                        url.set("https://mariadb.com/bsl11/")
                    }
                }
                scm {
                    url.set("https://github.com/Reactor/reactor-kotlin")
                }
            }
        }
    }
    repositories {
        maven {
            name = "central"
            url = uri("https://central.sonatype.com/repository/maven-releases/")
            credentials {
                username = System.getenv("MAVEN_CENTRAL_USERNAME") ?: ""
                password = System.getenv("MAVEN_CENTRAL_TOKEN") ?: ""
            }
        }
    }
}

signing {
    val key = System.getenv("MAVEN_GPG_KEY")
    if (!key.isNullOrBlank()) {
        useInMemoryPgpKeys(key, System.getenv("MAVEN_GPG_PASSPHRASE"))
        sign(publishing.publications["maven"])
    }
}
