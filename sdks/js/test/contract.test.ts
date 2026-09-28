import assert from "node:assert/strict"
import { test } from "node:test"
import { createClient } from "../src/index.ts"

const url = process.env.REACTOR_URL ?? "http://127.0.0.1:18000"
const anonKey = process.env.REACTOR_ANON_KEY ?? ""

function rows(result: { data: any; error: { message: string } | null }) {
  if (result.error) throw new Error(result.error.message)
  return result.data as Array<{ id: string; title: string; user_id: string }>
}

test("todos project contract", async () => {
  assert.ok(anonKey, "REACTOR_ANON_KEY is required")
  const email = `sdk-${crypto.randomUUID()}@example.com`
  const otherEmail = `sdk-${crypto.randomUUID()}@example.com`
  const password = "password123"
  const client = createClient(url, anonKey)

  const session = await client.auth.signUp({ email, password })
  assert.equal(session.user.email, email)
  assert.equal((await client.auth.getUser()).email, email)

  const title = `todo-${crypto.randomUUID()}`
  const inserted = rows(
    await client.from("todos").insert({ title, user_id: session.user.id }).select(),
  )
  assert.equal(inserted.length, 1)
  const id = inserted[0].id

  const listed = rows(
    await client.from("todos").select("*").eq("user_id", session.user.id).order("created_at", { ascending: false }),
  )
  assert.ok(listed.some((row) => row.id === id && row.title === title))

  const renamed = `${title}-edited`
  rows(await client.from("todos").update({ title: renamed }).eq("id", id).select())
  const after = rows(await client.from("todos").select("*").eq("id", id))
  assert.equal(after[0].title, renamed)

  const path = `note-${crypto.randomUUID()}.txt`
  const bytes = new TextEncoder().encode("reactor-sdk")
  await client.storage.from("files").upload(path, bytes)
  const downloaded = await client.storage.from("files").download(path)
  assert.deepEqual(downloaded, bytes)

  const ping = await client.functions.invoke("ping", { body: {} })
  assert.equal(ping.ok, true)

  const other = createClient(url, anonKey)
  await other.auth.signUp({ email: otherEmail, password })
  const hidden = rows(await other.from("todos").select("*").eq("id", id))
  assert.equal(hidden.length, 0)

  const anon = createClient(url, anonKey)
  const denied = await anon.from("todos").select("*")
  assert.ok(denied.error)

  rows(await client.from("todos").delete().eq("id", id).select())
  const gone = rows(await client.from("todos").select("*").eq("id", id))
  assert.equal(gone.length, 0)

  const previous = session.refresh_token
  const refreshed = await client.auth.refreshSession()
  assert.notEqual(refreshed.refresh_token, previous)
  const stale = await fetch(`${url}/auth/v1/token`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${anonKey}` },
    body: JSON.stringify({ refresh_token: previous }),
  })
  assert.equal(stale.status, 401)

  const current = refreshed.refresh_token
  await client.auth.signOut()
  const revoked = await fetch(`${url}/auth/v1/token`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${anonKey}` },
    body: JSON.stringify({ refresh_token: current }),
  })
  assert.equal(revoked.status, 401)

  const again = await client.auth.signInWithPassword({ email, password })
  assert.equal(again.user.email, email)
})
