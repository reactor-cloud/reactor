import assert from "node:assert/strict"
import { afterEach, test } from "node:test"
import { createClient } from "../src/index.ts"

type Call = { url: string; method: string; headers: Record<string, string>; body: string }

const calls: Call[] = []
let restore: (() => void) | undefined

function install() {
  calls.length = 0
  const previous = globalThis.fetch
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    const call = {
      url: request.url,
      method: request.method,
      headers: Object.fromEntries(request.headers.entries()),
      body: await request.text(),
    }
    calls.push(call)
    return route(call)
  }) as typeof fetch
  restore = () => {
    globalThis.fetch = previous
  }
}

function route(call: Call): Response {
  const url = new URL(call.url)
  if (url.pathname === "/auth/v1/signup" || url.pathname === "/auth/v1/token") {
    const sent = call.body ? JSON.parse(call.body) : {}
    const refresh = sent.refresh_token ? "refresh-2" : "refresh-1"
    return json({
      access_token: sent.refresh_token ? "access-2" : "access-1",
      refresh_token: refresh,
      user: { id: "user-1", email: sent.email ?? "a@b.co" },
    })
  }
  if (url.pathname === "/auth/v1/logout") return new Response(null, { status: 204 })
  if (url.pathname === "/auth/v1/magic-link" || url.pathname === "/auth/v1/recover" || url.pathname === "/auth/v1/invite") {
    return json({ ok: true })
  }
  if (url.pathname === "/auth/v1/verify" || url.pathname === "/auth/v1/recover/complete" || url.pathname === "/auth/v1/invite/accept") {
    const sent = call.body ? JSON.parse(call.body) : {}
    return json({
      access_token: "access-mail",
      refresh_token: "refresh-mail",
      user: { id: "user-1", email: sent.email ?? "a@b.co" },
    })
  }
  if (url.pathname === "/auth/v1/user") return json({ id: "user-1", email: "a@b.co" })
  if (url.pathname === "/storage/v1/object/presign") {
    return json({ url: "https://signed.example/object", key: "files/note.txt" })
  }
  if (url.hostname === "signed.example") return new Response("ok", { status: 200 })
  if (url.pathname === "/fn/v1/ping") return json({ ok: true })
  if (url.pathname.startsWith("/data/v1")) return json([])
  return new Response("not found", { status: 404 })
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  })
}

afterEach(() => restore?.())

test("uses the anon key before login and the access token after", async () => {
  install()
  const client = createClient("http://reactor.test", "anon-key")
  await client.from("todos").select("*")
  assert.equal(calls[0].headers.authorization, "Bearer anon-key")

  await client.auth.signUp({ email: "a@b.co", password: "password123" })
  await client.from("todos").select("*")
  const dataCall = calls.findLast((call) => call.url.includes("/data/v1/todos"))
  assert.equal(dataCall?.headers.authorization, "Bearer access-1")
})

test("signup and password login post the credentials", async () => {
  install()
  const client = createClient("http://reactor.test", "anon-key")
  await client.auth.signUp({ email: "a@b.co", password: "password123" })
  await client.auth.signInWithPassword({ email: "a@b.co", password: "password123" })
  assert.equal(calls[0].method, "POST")
  assert.equal(new URL(calls[0].url).pathname, "/auth/v1/signup")
  assert.deepEqual(JSON.parse(calls[0].body), { email: "a@b.co", password: "password123" })
  assert.equal(calls[0].headers.authorization, "Bearer anon-key")
  assert.equal(new URL(calls[1].url).pathname, "/auth/v1/token")
  assert.deepEqual(JSON.parse(calls[1].body), { email: "a@b.co", password: "password123" })
})

test("logout posts the refresh token and accepts an empty 204", async () => {
  install()
  const client = createClient("http://reactor.test", "anon-key")
  await client.auth.signUp({ email: "a@b.co", password: "password123" })
  await client.auth.signOut()
  const logout = calls.find((call) => call.url.includes("/auth/v1/logout"))
  assert.ok(logout)
  assert.equal(logout.method, "POST")
  assert.deepEqual(JSON.parse(logout.body), { refresh_token: "refresh-1" })
  assert.equal(client.auth.getSession(), null)
})

test("refresh replaces the session", async () => {
  install()
  const client = createClient("http://reactor.test", "anon-key")
  const first = await client.auth.signUp({ email: "a@b.co", password: "password123" })
  const next = await client.auth.refreshSession()
  assert.notEqual(next.refresh_token, first.refresh_token)
  assert.equal(client.auth.getSession()?.access_token, "access-2")
  const refresh = calls.findLast((call) => call.url.includes("/auth/v1/token"))
  assert.deepEqual(JSON.parse(refresh!.body), { refresh_token: "refresh-1" })
})

test("mail flows post to the auth endpoints", async () => {
  install()
  const client = createClient("http://reactor.test", "anon-key")
  await client.auth.signInWithMagicLink("a@b.co")
  const session = await client.auth.verifyMagicLink("mail-token")
  assert.equal(session.access_token, "access-mail")
  await client.auth.recover("a@b.co")
  await client.auth.completeRecovery("mail-token", "password123")
  await client.auth.invite("b@c.co", "service-key")
  await client.auth.acceptInvite("invite-token", "password123")
  const paths = calls.map((call) => new URL(call.url).pathname)
  assert.deepEqual(paths, [
    "/auth/v1/magic-link",
    "/auth/v1/verify",
    "/auth/v1/recover",
    "/auth/v1/recover/complete",
    "/auth/v1/invite",
    "/auth/v1/invite/accept",
  ])
  assert.equal(calls[4].headers.authorization, "Bearer service-key")
  assert.deepEqual(JSON.parse(calls[1].body), { token: "mail-token" })
  assert.deepEqual(JSON.parse(calls[5].body), { token: "invite-token", password: "password123" })
})

test("upload presigns then puts the bytes", async () => {
  install()
  const client = createClient("http://reactor.test", "anon-key")
  await client.storage.from("files").upload("note.txt", "hello")
  const presign = calls[0]
  assert.equal(new URL(presign.url).pathname, "/storage/v1/object/presign")
  assert.deepEqual(JSON.parse(presign.body), { bucket: "files", key: "note.txt", method: "PUT" })
  assert.equal(presign.headers.authorization, "Bearer anon-key")
  assert.equal(calls[1].method, "PUT")
  assert.equal(calls[1].url, "https://signed.example/object")
  assert.equal(calls[1].body, "hello")
})

test("invoke posts to the function path", async () => {
  install()
  const client = createClient("http://reactor.test", "anon-key")
  const body = await client.functions.invoke("ping", { body: {} })
  assert.deepEqual(body, { ok: true })
  assert.equal(new URL(calls[0].url).pathname, "/fn/v1/ping")
  assert.equal(calls[0].method, "POST")
  assert.equal(calls[0].headers.authorization, "Bearer anon-key")
})

test("select builds a postgrest query and writes ask for representation", async () => {
  install()
  const client = createClient("http://reactor.test", "anon-key")
  await client.from("todos").select("*").eq("user_id", "user-1").order("created_at", { ascending: false })
  const url = new URL(calls[0].url)
  assert.equal(url.pathname, "/data/v1/todos")
  assert.equal(url.searchParams.get("select"), "*")
  assert.equal(url.searchParams.get("user_id"), "eq.user-1")
  assert.equal(url.searchParams.get("order"), "created_at.desc")

  await client.from("todos").insert({ title: "a", user_id: "user-1" }).select()
  const insert = calls.findLast((call) => call.method === "POST" && call.url.includes("/data/v1/todos"))
  assert.ok(insert)
  assert.match(insert.headers.prefer ?? "", /return=representation/)
})

test("oauth is unsupported", () => {
  install()
  const client = createClient("http://reactor.test", "anon-key")
  assert.throws(() => client.auth.signInWithOAuth(), /unsupported/)
})
