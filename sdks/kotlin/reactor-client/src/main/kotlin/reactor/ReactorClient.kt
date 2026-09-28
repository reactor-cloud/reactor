package reactor

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

class ReactorException(val status: Int, message: String) : Exception(message)

data class User(val id: String, val email: String)

data class Session(val accessToken: String, val refreshToken: String, val user: User)

class ReactorClient(url: String, private val anonKey: String) {
    private val base = url.trimEnd('/')
    private var session: Session? = null
    private val http = OkHttpClient.Builder()
        .callTimeout(20, TimeUnit.SECONDS)
        .build()

    val auth = Auth()
    val storage = Storage()
    val functions = Functions()

    fun from(table: String) = Query(table)

    private fun token() = session?.accessToken ?: anonKey

    private suspend fun call(
        path: String,
        method: String = "GET",
        token: String? = null,
        json: JSONObject? = null,
        prefer: String? = null,
    ): Pair<Int, String> = withContext(Dispatchers.IO) {
        val body = json?.toString()?.toRequestBody(JSON)
        val builder = Request.Builder().url(base + path).method(method, if (method == "GET") null else body ?: ByteArray(0).toRequestBody(null))
        if (token != null) builder.header("Authorization", "Bearer $token")
        if (prefer != null) builder.header("Prefer", prefer)
        builder.header("Expect", "")
        http.newCall(builder.build()).execute().use { response ->
            response.code to (response.body?.string() ?: "")
        }
    }

    private fun parse(status: Int, text: String): JSONObject {
        if (text.isEmpty()) {
            if (status >= 400) throw ReactorException(status, "request failed")
            return JSONObject()
        }
        val trimmed = text.trim()
        if (trimmed.startsWith("[")) {
            if (status >= 400) throw ReactorException(status, "request failed")
            return JSONObject().put("rows", JSONArray(trimmed))
        }
        val body = JSONObject(trimmed)
        if (status >= 400) {
            throw ReactorException(status, body.optString("error", body.optString("message", "request failed")))
        }
        return body
    }

    private fun readSession(body: JSONObject) = Session(
        body.getString("access_token"),
        body.getString("refresh_token"),
        User(body.getJSONObject("user").getString("id"), body.getJSONObject("user").getString("email")),
    )

    inner class Auth {
        fun getSession() = session

        suspend fun signUp(email: String, password: String) =
            issue("/auth/v1/signup", JSONObject().put("email", email).put("password", password))

        suspend fun signInWithPassword(email: String, password: String) =
            issue("/auth/v1/token", JSONObject().put("email", email).put("password", password))

        suspend fun getUser(): User {
            if (session == null) throw ReactorException(401, "not signed in")
            val (status, text) = call("/auth/v1/user", token = token())
            val body = parse(status, text)
            return User(body.getString("id"), body.getString("email"))
        }

        suspend fun refreshSession(): Session {
            val refresh = session?.refreshToken ?: throw ReactorException(401, "not signed in")
            return issue("/auth/v1/token", JSONObject().put("refresh_token", refresh))
        }

        suspend fun signOut() {
            val refresh = session?.refreshToken ?: return
            val (status, text) = call("/auth/v1/logout", "POST", json = JSONObject().put("refresh_token", refresh))
            parse(status, text)
            session = null
        }

        fun signInWithOAuth(): Nothing = throw ReactorException(0, "unsupported")
    }

    private suspend fun issue(path: String, json: JSONObject): Session {
        val (status, text) = call(path, "POST", anonKey, json)
        val next = readSession(parse(status, text))
        session = next
        return next
    }

    inner class Storage {
        fun from(bucket: String) = Bucket(bucket)
    }

    inner class Bucket(private val bucket: String) {
        suspend fun upload(path: String, bytes: ByteArray, contentType: String = "application/octet-stream") {
            val signed = presign(path, "PUT")
            withContext(Dispatchers.IO) {
                val request = Request.Builder()
                    .url(signed)
                    .put(bytes.toRequestBody(contentType.toMediaType()))
                    .header("Expect", "")
                    .build()
                http.newCall(request).execute().use { response ->
                    if (!response.isSuccessful) throw ReactorException(response.code, response.body?.string() ?: "upload failed")
                }
            }
        }

        suspend fun download(path: String): ByteArray {
            val signed = presign(path, "GET")
            return withContext(Dispatchers.IO) {
                val request = Request.Builder().url(signed).header("Expect", "").build()
                http.newCall(request).execute().use { response ->
                    if (!response.isSuccessful) throw ReactorException(response.code, response.body?.string() ?: "download failed")
                    response.body?.bytes() ?: ByteArray(0)
                }
            }
        }

        private suspend fun presign(path: String, method: String): String {
            val json = JSONObject().put("bucket", bucket).put("key", path).put("method", method)
            val (status, text) = call("/storage/v1/object/presign", "POST", token(), json)
            return parse(status, text).getString("url")
        }
    }

    inner class Functions {
        suspend fun invoke(name: String, body: JSONObject = JSONObject()): JSONObject {
            val (status, text) = call("/fn/v1/$name", "POST", token(), body)
            return parse(status, text)
        }
    }

    inner class Query(private val table: String) {
        private var method = "GET"
        private var columns: String? = null
        private var body: JSONObject? = null
        private val filters = mutableListOf<Pair<String, String>>()
        private var orderBy: String? = null
        private var rowLimit: Int? = null

        fun select(columns: String = "*"): Query {
            this.columns = columns
            return this
        }

        fun insert(row: JSONObject): Query {
            method = "POST"
            body = row
            return this
        }

        fun update(row: JSONObject): Query {
            method = "PATCH"
            body = row
            return this
        }

        fun delete(): Query {
            method = "DELETE"
            return this
        }

        fun eq(column: String, value: String): Query {
            filters += column to value
            return this
        }

        fun order(column: String, ascending: Boolean = true): Query {
            orderBy = if (ascending) "$column.asc" else "$column.desc"
            return this
        }

        fun limit(count: Int): Query {
            rowLimit = count
            return this
        }

        suspend fun execute(): JSONArray = withContext(Dispatchers.IO) {
            val url = okhttp3.HttpUrl.Builder()
                .scheme(if (base.startsWith("https")) "https" else "http")
                .host(host())
                .port(port())
                .addPathSegments("data/v1/$table".trim('/'))
            columns?.let { url.addQueryParameter("select", it) }
            filters.forEach { (column, value) -> url.addQueryParameter(column, "eq.$value") }
            orderBy?.let { url.addQueryParameter("order", it) }
            rowLimit?.let { url.addQueryParameter("limit", it.toString()) }
            val payload = body?.toString()?.toRequestBody(JSON)
            val requestBody = if (method == "GET") null else payload ?: ByteArray(0).toRequestBody(null)
            val request = Request.Builder()
                .url(url.build())
                .method(method, requestBody)
                .header("Authorization", "Bearer ${token()}")
                .header("Accept", "application/json")
                .header("Expect", "")
                .apply { if (method != "GET") header("Prefer", "return=representation") }
                .build()
            http.newCall(request).execute().use { response ->
                val text = response.body?.string() ?: ""
                if (!response.isSuccessful) {
                    val message = runCatching { JSONObject(text).optString("message", text) }.getOrDefault(text)
                    throw ReactorException(response.code, message.ifEmpty { "request failed" })
                }
                if (text.isEmpty()) JSONArray() else JSONArray(text)
            }
        }
    }

    private fun host(): String = java.net.URI(base).host
    private fun port(): Int {
        val uri = java.net.URI(base)
        return if (uri.port == -1) if (uri.scheme == "https") 443 else 80 else uri.port
    }

    companion object {
        private val JSON = "application/json".toMediaType()
    }
}
