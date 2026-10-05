import { useEffect, useState, type FormEvent } from "react"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api } from "@/lib/api"
import { Copy } from "lucide-react"

const SCOPES = [
  ["projects.create", "Create projects"],
  ["projects.migrate", "Apply migrations"],
  ["projects.sql", "Run SQL"],
  ["auth.settings", "Verification and MFA"],
  ["auth.providers", "OAuth providers"],
  ["auth.email", "SMTP and templates"],
  ["auth.users", "Users"],
] as const

type ServiceKey = {
  id: string
  name: string
  scopes: string[]
  created_at: string
}

export function ServiceKeys() {
  const [keys, setKeys] = useState<ServiceKey[]>([])
  const [fresh, setFresh] = useState<{ id: string; token: string } | null>(null)
  const [copied, setCopied] = useState(false)
  const [error, setError] = useState("")

  function load() {
    api<ServiceKey[]>("/console/v1/keys")
      .then(setKeys)
      .catch((err) => setError(err instanceof Error ? err.message : "load failed"))
  }

  useEffect(load, [])

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const data = new FormData(form)
    const scopes = SCOPES.map(([scope]) => scope).filter((scope) => data.get(scope) === "on")
    try {
      const created = await api<ServiceKey & { token: string }>("/console/v1/keys", {
        method: "POST",
        body: JSON.stringify({ name: data.get("name"), scopes }),
      })
      setFresh({ id: created.id, token: created.token })
      setCopied(false)
      setError("")
      form.reset()
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "create failed")
    }
  }

  async function revoke(key: ServiceKey) {
    if (!window.confirm(`Revoke ${key.name}?`)) return
    try {
      await api(`/console/v1/keys/${key.id}`, { method: "DELETE" })
      setFresh((current) => (current?.id === key.id ? null : current))
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "revoke failed")
    }
  }

  async function copy() {
    if (!fresh) return
    await navigator.clipboard.writeText(fresh.token)
    setCopied(true)
  }

  return (
    <div className="grid w-full min-w-0 max-w-3xl gap-4">
      <h1 className="text-2xl font-medium tracking-tight">Service keys</h1>
      <p className="text-sm text-muted-foreground">
        A long-lived key for a function that creates projects. Extra scopes apply only to projects that key creates. The secret is shown once.
      </p>
      {keys.length === 0 ? (
        <p className="text-sm text-muted-foreground">No service keys yet.</p>
      ) : (
        <div className="grid gap-2">
          {keys.map((key) => (
            <div key={key.id} className="flex items-center justify-between gap-3 rounded-md border px-3 py-2">
              <div className="min-w-0">
                <div className="truncate text-sm font-medium">{key.name}</div>
                <div className="truncate text-xs text-muted-foreground">{key.scopes.join(", ")}</div>
              </div>
              <Button variant="outline" onClick={() => revoke(key)}>
                Revoke
              </Button>
            </div>
          ))}
        </div>
      )}
      {fresh && (
        <div className="grid gap-2 rounded-md border p-3">
          <span className="text-xs text-muted-foreground">Token</span>
          <div className="flex gap-2">
            <Input readOnly value={fresh.token} className="font-mono text-xs" />
            <Button variant="outline" onClick={copy}>
              <Copy className="size-4" />
              {copied ? "Copied" : "Copy"}
            </Button>
          </div>
        </div>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">New key</CardTitle>
        </CardHeader>
        <CardContent>
          <form className="grid gap-3" onSubmit={onSubmit}>
            <div className="grid gap-2">
              <Label htmlFor="name">Name</Label>
              <Input id="name" name="name" required />
            </div>
            <div className="grid gap-2">
              {SCOPES.map(([scope, label]) => (
                <label key={scope} className="flex items-center gap-2 text-sm">
                  <input name={scope} type="checkbox" />
                  {label}
                </label>
              ))}
            </div>
            <Button type="submit" variant="outline" className="w-fit">
              Create key
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}
