package lab.reactor.todos

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONObject
import reactor.ReactorClient

class TodosState(private val client: ReactorClient, private val onPick: () -> Unit) {
    private val scope = CoroutineScope(Dispatchers.Main)
    var email by mutableStateOf("")
    var password by mutableStateOf("")
    var draft by mutableStateOf("")
    var signedIn by mutableStateOf(false)
    var items by mutableStateOf(listOf<Pair<String, String>>())
    var edits by mutableStateOf(mapOf<String, String>())
    var error by mutableStateOf("")
    var ping by mutableStateOf("")
    var fileNote by mutableStateOf("")

    fun signUp() = run { client.auth.signUp(email, password); signedIn = true; load() }
    fun signIn() = run { client.auth.signInWithPassword(email, password); signedIn = true; load() }
    fun signOut() = run { client.auth.signOut(); signedIn = false; items = emptyList() }
    fun add() = run {
        val user = client.auth.getSession()!!.user
        client.from("todos").insert(JSONObject().put("title", draft).put("user_id", user.id)).select().execute()
        draft = ""
        load()
    }
    fun save(id: String) = run {
        client.from("todos").update(JSONObject().put("title", edits[id] ?: "")).eq("id", id).select().execute()
        load()
    }
    fun remove(id: String) = run {
        client.from("todos").delete().eq("id", id).select().execute()
        load()
    }
    fun ping() = run { ping = if (client.functions.invoke("ping").optBoolean("ok")) "ok" else "failed" }
    fun pick() = onPick()

    fun upload(name: String, bytes: ByteArray) = run {
        client.storage.from("files").upload(name, bytes)
        val back = client.storage.from("files").download(name)
        fileNote = "Uploaded $name (${back.size} bytes)"
    }

    private suspend fun load() {
        val user = client.auth.getSession()?.user ?: return
        val rows = client.from("todos").select().eq("user_id", user.id).order("created_at", ascending = false).execute()
        val next = (0 until rows.length()).map { index ->
            val row = rows.getJSONObject(index)
            row.getString("id") to row.getString("title")
        }
        items = next
        edits = next.associate { it.first to it.second }
    }

    private fun run(block: suspend () -> Unit) {
        scope.launch {
            try {
                error = ""
                block()
            } catch (err: Exception) {
                error = err.message ?: err.toString()
            }
        }
    }
}

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val client = ReactorClient(BuildConfig.REACTOR_URL, BuildConfig.REACTOR_ANON_KEY)
        lateinit var state: TodosState
        val picker = registerForActivityResult(ActivityResultContracts.GetContent()) { uri ->
            if (uri == null) return@registerForActivityResult
            val name = uri.lastPathSegment ?: "upload.bin"
            val bytes = contentResolver.openInputStream(uri)?.use { it.readBytes() } ?: return@registerForActivityResult
            state.upload(name, bytes)
        }
        state = TodosState(client) { picker.launch("*/*") }
        setContent {
            MaterialTheme { TodosScreen(state) }
        }
    }
}

@Composable
private fun TodosScreen(state: TodosState) {
    Column(Modifier.verticalScroll(rememberScrollState()).padding(16.dp)) {
        Text("Todos", style = MaterialTheme.typography.headlineMedium)
        if (!state.signedIn) {
            OutlinedTextField(state.email, { state.email = it }, label = { Text("Email") }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(state.password, { state.password = it }, label = { Text("Password") }, modifier = Modifier.fillMaxWidth())
            Button(onClick = { state.signUp() }) { Text("Sign up") }
            Button(onClick = { state.signIn() }) { Text("Log in") }
        } else {
            OutlinedTextField(state.draft, { state.draft = it }, label = { Text("New todo") }, modifier = Modifier.fillMaxWidth())
            Button(onClick = { state.add() }) { Text("Add") }
            state.items.forEach { (id, title) ->
                Text(title, modifier = Modifier.padding(top = 12.dp))
                OutlinedTextField(
                    state.edits[id] ?: title,
                    { state.edits = state.edits + (id to it) },
                    label = { Text("Edit title") },
                    modifier = Modifier.fillMaxWidth(),
                )
                Button(onClick = { state.save(id) }) { Text("Save") }
                Button(onClick = { state.remove(id) }) { Text("Delete") }
            }
            Button(onClick = { state.pick() }, modifier = Modifier.padding(top = 12.dp)) { Text("Upload") }
            if (state.fileNote.isNotEmpty()) Text(state.fileNote)
            Button(onClick = { state.ping() }) { Text("Call ping function") }
            if (state.ping.isNotEmpty()) Text(state.ping)
            Button(onClick = { state.signOut() }) { Text("Log out") }
        }
        if (state.error.isNotEmpty()) {
            Text(state.error, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(top = 12.dp))
        }
    }
}
