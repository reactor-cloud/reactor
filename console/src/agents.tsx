import { useEffect, useState, type FormEvent } from "react"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api } from "@/lib/api"

type Session = {
  family_id: string
  client_id: string
  scope: string
  created_at: string
  expires_at: string
}

type McpKey = {
  id: string
  name: string
  scope: string
  created_at: string
  token?: string
}

export function Agents() {
  const [sessions, setSessions] = useState<Session[]>([])
  const [keys, setKeys] = useState<McpKey[]>([])
  const [fresh, setFresh] = useState<string>("")
  const [error, setError] = useState("")

  function load() {
    Promise.all([
      api<Session[]>("/console/v1/oauth/sessions"),
      api<McpKey[]>("/console/v1/mcp/keys"),
    ])
      .then(([sessions, keys]) => {
        setSessions(sessions)
        setKeys(keys)
      })
      .catch((err) => setError(err instanceof Error ? err.message : "load failed"))
  }

  useEffect(load, [])

  async function revokeSession(family: string) {
    try {
      await api(`/console/v1/oauth/sessions/${family}`, { method: "DELETE" })
      setError("")
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "revoke failed")
    }
  }

  async function createKey(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const data = new FormData(form)
    try {
      const created = await api<McpKey>("/console/v1/mcp/keys", {
        method: "POST",
        body: JSON.stringify({
          name: data.get("name"),
          scope: data.get("scope") || "operator",
        }),
      })
      setFresh(created.token || "")
      setError("")
      form.reset()
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "create failed")
    }
  }

  async function revokeKey(id: string) {
    try {
      await api(`/console/v1/mcp/keys/${id}`, { method: "DELETE" })
      setError("")
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "revoke failed")
    }
  }

  return (
    <div className="grid w-full min-w-0 max-w-3xl gap-4">
      <h1 className="text-2xl font-medium tracking-tight">Agents</h1>
      <p className="text-sm text-muted-foreground">
        MCP clients that signed in with a browser, and bearer keys for clients that cannot.
      </p>
      {error ? <p className="text-sm text-destructive">{error}</p> : null}
      <Card>
        <CardHeader>
          <CardTitle>Signed-in clients</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-3">
          {sessions.length === 0 ? (
            <p className="text-sm text-muted-foreground">No agent sessions.</p>
          ) : (
            sessions.map((session) => (
              <div key={session.family_id} className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="truncate font-mono text-xs">{session.client_id}</div>
                  <div className="text-xs text-muted-foreground">{session.scope}</div>
                </div>
                <Button variant="outline" onClick={() => revokeSession(session.family_id)}>
                  Revoke
                </Button>
              </div>
            ))
          )}
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>MCP keys</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-4">
          <form className="grid gap-3" onSubmit={createKey}>
            <div className="grid gap-2">
              <Label htmlFor="agent-key-name">Name</Label>
              <Input id="agent-key-name" name="name" required />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="agent-key-scope">Scope</Label>
              <select id="agent-key-scope" name="scope" className="h-9 rounded-md border bg-background px-3 text-sm">
                <option value="operator">operator</option>
                <option value="operator:read">operator:read</option>
              </select>
            </div>
            <Button type="submit">Create key</Button>
          </form>
          {fresh ? (
            <p className="break-all font-mono text-xs" data-testid="mcp-key">
              {fresh}
            </p>
          ) : null}
          {keys.map((key) => (
            <div key={key.id} className="flex items-center justify-between gap-3">
              <div>
                <div className="text-sm">{key.name}</div>
                <div className="text-xs text-muted-foreground">{key.scope}</div>
              </div>
              <Button variant="outline" onClick={() => revokeKey(key.id)}>
                Revoke
              </Button>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  )
}
