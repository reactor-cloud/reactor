const TOKEN = "reactor.console.token"

export type Operator = {
  id: string
  email: string
  name: string
  platform_admin: boolean
}

export type Project = {
  ref: string
  name: string
  role: string
}

export function placeLabel(place: string) {
  if (place === "docker") return "Local Docker"
  if (place === "aws") return "AWS"
  if (place === "fly") return "Fly"
  return place
}

export function token() {
  return localStorage.getItem(TOKEN)
}

export function setToken(value: string) {
  localStorage.setItem(TOKEN, value)
}

export function clearToken() {
  localStorage.removeItem(TOKEN)
}

export function savedKeys(ref: string): { anon_key: string; service_key: string } | null {
  const raw = sessionStorage.getItem(`reactor.keys.${ref}`)
  return raw ? JSON.parse(raw) : null
}

export function saveKeys(ref: string, keys: { anon_key: string; service_key: string }) {
  sessionStorage.setItem(`reactor.keys.${ref}`, JSON.stringify(keys))
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers)
  if (init.body) headers.set("content-type", "application/json")
  const current = token()
  if (current && !headers.has("authorization")) headers.set("authorization", `Bearer ${current}`)
  const response = await fetch(path, { ...init, headers })
  const text = await response.text()
  const body = text ? JSON.parse(text) : null
  if (!response.ok) throw new Error(body?.error || response.statusText)
  return body as T
}
