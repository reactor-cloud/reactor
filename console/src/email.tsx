import { useEffect, useState, type FormEvent } from "react"
import { useOutletContext, useParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api } from "@/lib/api"
import type { ConsoleContext } from "@/shell"

type Settings = {
  host: string
  port: number
  username: string
  password_set: boolean
  from_address: string
  tls: string
  link_base: string
  cluster_smtp: boolean
  source: "project" | "cluster" | "none"
  cluster_from: string
}

type TemplateSummary = { name: string; subject: string; reserved: boolean }

type Template = TemplateSummary & { body_text: string; body_html: string }

const area =
  "min-h-40 w-full rounded-lg border border-input bg-transparent px-2.5 py-2 font-mono text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"

export function EmailSettings() {
  return <EmailPane mode="server" />
}

export function Templates() {
  return <EmailPane mode="templates" />
}

function EmailPane({ mode }: { mode: "server" | "templates" }) {
  const { ref = "" } = useParams()
  const { role, platformAdmin } = useOutletContext<ConsoleContext>()
  const admin = role === "owner" || role === "admin"
  const [settings, setSettings] = useState<Settings | null>(null)
  const [password, setPassword] = useState("")
  const [templates, setTemplates] = useState<TemplateSummary[]>([])
  const [current, setCurrent] = useState<Template | null>(null)
  const [testTo, setTestTo] = useState("")
  const [error, setError] = useState("")
  const [note, setNote] = useState("")

  useEffect(() => {
    let cancelled = false
    Promise.all([
      api<Settings>(`/console/v1/projects/${ref}/email`),
      api<TemplateSummary[]>(`/console/v1/projects/${ref}/email/templates`),
    ])
      .then(([next, list]) => {
        if (cancelled) return
        setSettings(next)
        setTemplates(list)
        const first = list[0]
        if (!first) return
        api<Template>(`/console/v1/projects/${ref}/email/templates/${first.name}`).then((template) => {
          if (!cancelled) setCurrent((existing) => existing ?? template)
        })
      })
      .catch((err) => {
        if (!cancelled) setError(err instanceof Error ? err.message : "email is unavailable")
      })
    return () => {
      cancelled = true
    }
  }, [ref])

  function open(name: string) {
    api<Template>(`/console/v1/projects/${ref}/email/templates/${name}`)
      .then(setCurrent)
      .catch((err) => setError(err instanceof Error ? err.message : "template is unavailable"))
  }

  async function setCluster(enabled: boolean) {
    setError("")
    setNote("")
    try {
      await api(`/console/v1/projects/${ref}/email/cluster`, {
        method: "PUT",
        body: JSON.stringify({ enabled }),
      })
      setSettings(await api<Settings>(`/console/v1/projects/${ref}/email`))
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
    }
  }

  async function saveSettings(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!settings) return
    setError("")
    setNote("")
    try {
      const next = await api<Settings>(`/console/v1/projects/${ref}/email`, {
        method: "PUT",
        body: JSON.stringify({ ...settings, password: password || undefined }),
      })
      setSettings(next)
      setPassword("")
      setNote("Mail server saved.")
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
    }
  }

  async function sendTest() {
    setError("")
    setNote("")
    try {
      await api(`/console/v1/projects/${ref}/email/test`, {
        method: "POST",
        body: JSON.stringify({ to: testTo }),
      })
      setNote("Test message sent.")
    } catch (err) {
      setError(err instanceof Error ? err.message : "test failed")
    }
  }

  async function saveTemplate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!current) return
    setError("")
    setNote("")
    try {
      const next = await api<Template>(`/console/v1/projects/${ref}/email/templates/${current.name}`, {
        method: "PUT",
        body: JSON.stringify({
          subject: current.subject,
          body_text: current.body_text,
          body_html: current.body_html,
        }),
      })
      setCurrent(next)
      setTemplates((list) => list.map((item) => (item.name === next.name ? { ...item, subject: next.subject } : item)))
      setNote("Template saved.")
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
    }
  }

  async function resetTemplate() {
    if (!current) return
    setError("")
    const next = await api<Template>(`/console/v1/projects/${ref}/email/templates/${current.name}/reset`, { method: "POST" })
    setCurrent(next)
    setNote("Template reset.")
  }

  async function addTemplate() {
    const name = window.prompt("Template name")?.trim().toLowerCase()
    if (!name) return
    setError("")
    try {
      const next = await api<Template>(`/console/v1/projects/${ref}/email/templates/${name}`, {
        method: "PUT",
        body: JSON.stringify({ subject: name, body_text: "", body_html: "" }),
      })
      setTemplates((list) => [...list, { name: next.name, subject: next.subject, reserved: false }].sort((a, b) => a.name.localeCompare(b.name)))
      setCurrent(next)
    } catch (err) {
      setError(err instanceof Error ? err.message : "could not add template")
    }
  }

  if (!admin) {
    return <p className="text-sm text-muted-foreground">Owners and admins can configure email.</p>
  }

  return (
    <div className="grid max-w-3xl gap-8">
      <div>
        <h1 className="text-2xl font-medium tracking-tight">{mode === "server" ? "Email" : "Templates"}</h1>
        <p className="text-sm text-muted-foreground">
          {mode === "server"
            ? "SMTP for this project. Auth mail uses the confirm, magic link, recovery, and invite templates."
            : "Templates can use {{email}}, {{token}}, {{link}}, and {{code}}."}
        </p>
      </div>
      {mode === "server" && settings && (
        <form className="grid gap-3" onSubmit={saveSettings}>
          {platformAdmin && (
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={settings.cluster_smtp}
                onChange={(event) => setCluster(event.target.checked)}
              />
              Auth email may use the cluster server
            </label>
          )}
          {settings.source === "cluster" && (
            <p className="text-sm text-muted-foreground">
              Auth mail uses the cluster server. From {settings.cluster_from}.
            </p>
          )}
          <div className="grid gap-3 sm:grid-cols-3">
            <Field label="Host" value={settings.host} onChange={(host) => setSettings({ ...settings, host })} />
            <Field label="Port" value={String(settings.port)} onChange={(port) => setSettings({ ...settings, port: Number(port) })} />
            <label className="grid gap-2 text-sm">
              <Label>TLS</Label>
              <select
                className="h-8 rounded-lg border border-input bg-transparent px-2"
                value={settings.tls}
                onChange={(event) => setSettings({ ...settings, tls: event.target.value })}
              >
                <option value="starttls">STARTTLS</option>
                <option value="tls">TLS</option>
                <option value="none">None</option>
              </select>
            </label>
          </div>
          <Field label="Username" value={settings.username} onChange={(username) => setSettings({ ...settings, username })} />
          <label className="grid gap-2">
            <Label>Password</Label>
            <Input
              type="password"
              value={password}
              placeholder={settings.password_set ? "Saved. Enter a new password to replace it." : ""}
              onChange={(event) => setPassword(event.target.value)}
            />
          </label>
          <Field label="From address" value={settings.from_address} onChange={(from_address) => setSettings({ ...settings, from_address })} />
          <Field label="Link base" value={settings.link_base} onChange={(link_base) => setSettings({ ...settings, link_base })} />
          <p className="text-sm text-muted-foreground">Templates can use {"{{email}}"}, {"{{token}}"}, and {"{{link}}"}. Link base is where the token is appended.</p>
          <Button type="submit">Save server</Button>
        </form>
      )}
      {mode === "server" && (
      <div className="flex items-end gap-2">
        <label className="grid flex-1 gap-2">
          <Label>Send a test</Label>
          <Input value={testTo} onChange={(event) => setTestTo(event.target.value)} placeholder="you@example.com" />
        </label>
        <Button type="button" variant="outline" onClick={sendTest}>
          Send
        </Button>
      </div>
      )}
      {mode === "templates" && (
      <section className="grid gap-3">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-medium">Templates</h2>
          <Button type="button" variant="outline" size="sm" onClick={addTemplate}>
            Add
          </Button>
        </div>
        <div className="flex flex-wrap gap-2">
          {templates.map((template) => (
            <Button key={template.name} type="button" size="sm" variant={current?.name === template.name ? "default" : "outline"} onClick={() => open(template.name)}>
              {template.name}
            </Button>
          ))}
        </div>
        {current && (
          <form className="grid gap-3" onSubmit={saveTemplate}>
            <Field label="Subject" value={current.subject} onChange={(subject) => setCurrent({ ...current, subject })} />
            <label className="grid gap-2">
              <Label>Text</Label>
              <textarea className={area} value={current.body_text} onChange={(event) => setCurrent({ ...current, body_text: event.target.value })} />
            </label>
            <label className="grid gap-2">
              <Label>HTML</Label>
              <textarea className={area} value={current.body_html} onChange={(event) => setCurrent({ ...current, body_html: event.target.value })} />
            </label>
            <div className="flex gap-2">
              <Button type="submit">Save template</Button>
              {current.reserved && (
                <Button type="button" variant="outline" onClick={resetTemplate}>
                  Reset
                </Button>
              )}
            </div>
          </form>
        )}
      </section>
      )}
      {note && <p className="text-sm text-muted-foreground">{note}</p>}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}

function Field({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return (
    <label className="grid gap-2">
      <Label>{label}</Label>
      <Input value={value} onChange={(event) => onChange(event.target.value)} />
    </label>
  )
}
