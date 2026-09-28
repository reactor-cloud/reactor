package reactor

import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.util.UUID

class ContractTest {
    @Test
    fun todosProject() = runBlocking {
        val url = System.getenv("REACTOR_URL") ?: "http://127.0.0.1:18000"
        val anonKey = System.getenv("REACTOR_ANON_KEY") ?: ""
        assertTrue("REACTOR_ANON_KEY is required", anonKey.isNotEmpty())

        val email = "sdk-${UUID.randomUUID()}@example.com"
        val otherEmail = "sdk-${UUID.randomUUID()}@example.com"
        val password = "password123"
        val client = ReactorClient(url, anonKey)

        val session = client.auth.signUp(email, password)
        assertEquals(email, session.user.email)
        assertEquals(email, client.auth.getUser().email)

        val title = "todo-${UUID.randomUUID()}"
        val inserted = client.from("todos").insert(JSONObject().put("title", title).put("user_id", session.user.id)).select().execute()
        assertEquals(1, inserted.length())
        val id = inserted.getJSONObject(0).getString("id")

        val listed = client.from("todos").select().eq("user_id", session.user.id).order("created_at", ascending = false).execute()
        assertTrue((0 until listed.length()).any {
            val row = listed.getJSONObject(it)
            row.getString("id") == id && row.getString("title") == title
        })

        val renamed = "$title-edited"
        client.from("todos").update(JSONObject().put("title", renamed)).eq("id", id).select().execute()
        val after = client.from("todos").select().eq("id", id).execute()
        assertEquals(renamed, after.getJSONObject(0).getString("title"))

        val path = "note-${UUID.randomUUID()}.txt"
        val bytes = "reactor-sdk".toByteArray()
        client.storage.from("files").upload(path, bytes)
        assertArrayEquals(bytes, client.storage.from("files").download(path))

        val ping = client.functions.invoke("ping")
        assertEquals(true, ping.getBoolean("ok"))

        val other = ReactorClient(url, anonKey)
        other.auth.signUp(otherEmail, password)
        assertEquals(0, other.from("todos").select().eq("id", id).execute().length())

        val anon = ReactorClient(url, anonKey)
        try {
            anon.from("todos").select().execute()
            fail("anon read should fail")
        } catch (error: ReactorException) {
            assertTrue(error.status >= 400)
        }

        client.from("todos").delete().eq("id", id).select().execute()
        assertEquals(0, client.from("todos").select().eq("id", id).execute().length())

        val previous = session.refreshToken
        val refreshed = client.auth.refreshSession()
        assertNotEquals(previous, refreshed.refreshToken)
        assertEquals(401, postRefresh(url, anonKey, previous))

        val current = refreshed.refreshToken
        client.auth.signOut()
        assertEquals(401, postRefresh(url, anonKey, current))

        val again = client.auth.signInWithPassword(email, password)
        assertEquals(email, again.user.email)
    }
}

private fun postRefresh(url: String, anonKey: String, token: String): Int {
    val client = HttpClient.newHttpClient()
    val request = HttpRequest.newBuilder(URI.create("$url/auth/v1/token"))
        .header("content-type", "application/json")
        .header("authorization", "Bearer $anonKey")
        .POST(HttpRequest.BodyPublishers.ofString("""{"refresh_token":"$token"}"""))
        .build()
    return client.send(request, HttpResponse.BodyHandlers.discarding()).statusCode()
}
