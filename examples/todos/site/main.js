import { createClient } from "../../../sdks/js/src/index.ts"

const anon = window.REACTOR_ANON_KEY || ""
const sessionKey = "todos.session"

function saved() {
  const raw = localStorage.getItem(sessionKey)
  return raw ? JSON.parse(raw) : null
}

function persist(session) {
  if (session) localStorage.setItem(sessionKey, JSON.stringify(session))
  else localStorage.removeItem(sessionKey)
}

function showError(error) {
  document.getElementById("error").textContent = error.message || String(error)
}

function clearError() {
  document.getElementById("error").textContent = ""
}

function finishAuth(result) {
  if (result?.access_token) {
    persist(result)
    render()
    return
  }
  if (result?.verification_required) {
    showError(new Error("Check your email to verify this account."))
    return
  }
  if (result?.mfa_required) {
    showError(new Error("A second factor is required."))
    return
  }
  if (result?.enrollment_required) {
    showError(new Error("Enroll a second factor to finish sign-in."))
    return
  }
  showError(new Error("Sign-in did not return a session."))
}

const reactor = createClient(window.location.origin, anon, { session: saved() })

function render() {
  const current = reactor.auth.getSession()
  document.getElementById("signed-out").hidden = Boolean(current)
  document.getElementById("signed-in").hidden = !current
  if (current) loadTodos()
}

async function loadTodos() {
  const current = reactor.auth.getSession()
  const { data, error } = await reactor
    .from("todos")
    .select("*")
    .eq("user_id", current.user.id)
    .order("created_at", { ascending: false })
  if (error) throw new Error(error.message)
  const list = document.getElementById("list")
  list.innerHTML = ""
  for (const row of data) {
    const item = document.createElement("li")
    const title = document.createElement("span")
    title.textContent = row.title
    const edit = document.createElement("input")
    edit.value = row.title
    edit.setAttribute("aria-label", "Edit title")
    const save = document.createElement("button")
    save.type = "button"
    save.textContent = "Save"
    save.addEventListener("click", async () => {
      try {
        clearError()
        const result = await reactor.from("todos").update({ title: edit.value }).eq("id", row.id).select()
        if (result.error) throw new Error(result.error.message)
        await loadTodos()
      } catch (error) {
        showError(error)
      }
    })
    const remove = document.createElement("button")
    remove.type = "button"
    remove.textContent = "Delete"
    remove.addEventListener("click", async () => {
      try {
        clearError()
        const result = await reactor.from("todos").delete().eq("id", row.id).select()
        if (result.error) throw new Error(result.error.message)
        await loadTodos()
      } catch (error) {
        showError(error)
      }
    })
    item.append(title, edit, save, remove)
    list.appendChild(item)
  }
}

document.getElementById("signup").addEventListener("submit", async (event) => {
  event.preventDefault()
  const data = new FormData(event.target)
  try {
    clearError()
    const result = await reactor.auth.signUp({
      email: data.get("email"),
      password: data.get("password"),
    })
    finishAuth(result)
  } catch (error) {
    showError(error)
  }
})

document.getElementById("login").addEventListener("submit", async (event) => {
  event.preventDefault()
  const data = new FormData(event.target)
  try {
    clearError()
    const result = await reactor.auth.signInWithPassword({
      email: data.get("email"),
      password: data.get("password"),
    })
    finishAuth(result)
  } catch (error) {
    showError(error)
  }
})

document.getElementById("add").addEventListener("submit", async (event) => {
  event.preventDefault()
  const current = reactor.auth.getSession()
  const data = new FormData(event.target)
  try {
    clearError()
    const result = await reactor.from("todos").insert({ title: data.get("title"), user_id: current.user.id }).select()
    if (result.error) throw new Error(result.error.message)
    event.target.reset()
    await loadTodos()
  } catch (error) {
    showError(error)
  }
})

document.getElementById("upload").addEventListener("submit", async (event) => {
  event.preventDefault()
  const file = event.target.file.files[0]
  try {
    clearError()
    const bytes = new Uint8Array(await file.arrayBuffer())
    await reactor.storage.from("files").upload(file.name, bytes, { contentType: file.type || "application/octet-stream" })
    const downloaded = await reactor.storage.from("files").download(file.name)
    document.getElementById("file-note").textContent = `Uploaded ${file.name} (${downloaded.byteLength} bytes)`
  } catch (error) {
    showError(error)
  }
})

document.getElementById("ping").addEventListener("click", async () => {
  try {
    clearError()
    const body = await reactor.functions.invoke("ping", { body: {} })
    document.getElementById("ping-out").textContent = JSON.stringify(body, null, 2)
  } catch (error) {
    showError(error)
  }
})

document.getElementById("logout").addEventListener("click", async () => {
  try {
    clearError()
    await reactor.auth.signOut()
    persist(null)
    render()
  } catch (error) {
    showError(error)
  }
})

render()
