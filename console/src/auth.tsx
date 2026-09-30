import { useEffect, useState, type FormEvent } from "react"
import { NavLink, Outlet, useOutletContext, useParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api } from "@/lib/api"
import type { ConsoleContext } from "@/shell"

const sections = [
  { id: "", label: "Users" },
  { id: "providers", label: "Providers" },
  { id: "mfa", label: "2FA" },
  { id: "email", label: "Email" },
  { id: "templates", label: "Templates" },
] as const

const catalog = ["google", "microsoft", "apple", "github", "facebook", "discord", "x", "linkedin", "slack", "gitlab"]

type AuthSettings = {
  require_email_verification: boolean
  require_mfa: boolean
  callback_base: string
}

type Provider = {
  provider: string
  client_id: string
  enabled: boolean
  redirects: string[]
  extra: Record<string, string>
  callback: string
}

export function Auth() {
  const { ref = "" } = useParams()
  const context = useOutletContext<ConsoleContext>()
  return (
    <div className="absolute inset-0 flex min-h-0">
      <aside className="flex w-64 shrink-0 flex-col border-r">
        <div className="flex h-12 shrink-0 items-center border-b px-3">
          <h1 className="text-base font-semibold">Auth</h1>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-1 p-2">
          <div className="mb-1 truncate rounded-full bg-muted px-3 py-2 text-center font-mono text-base">{ref}</div>
          {sections.map((section) => (
            <NavLink
              key={section.id || "users"}
              to={section.id ? `/p/${ref}/auth/${section.id}` : `/p/${ref}/auth`}
              end={!section.id}
              className={({ isActive }) => `rounded-md px-2 py-1.5 text-sm ${isActive ? "bg-muted font-medium" : "hover:bg-muted/60"}`}
            >
              {section.label}
            </NavLink>
          ))}
        </div>
      </aside>
      <div className="min-w-0 flex-1 overflow-auto p-6">
        <Outlet context={context} />
      </div>
    </div>
  )
}

export function Providers() {
  const { ref = "" } = useParams()
  const { role } = useOutletContext<ConsoleContext>()
  const admin = role === "owner" || role === "admin"
  const [settings, setSettings] = useState<AuthSettings | null>(null)
  const [rows, setRows] = useState<Provider[]>([])
  const [provider, setProvider] = useState("google")
  const [clientId, setClientId] = useState("")
  const [secret, setSecret] = useState("")
  const [redirects, setRedirects] = useState("")
  const [teamId, setTeamId] = useState("")
  const [keyId, setKeyId] = useState("")
  const [tenant, setTenant] = useState("common")
  const [privateKey, setPrivateKey] = useState("")
  const [error, setError] = useState("")
  const [note, setNote] = useState("")

  function load() {
    Promise.all([
      api<AuthSettings>(`/console/v1/projects/${ref}/auth`),
      api<Provider[]>(`/console/v1/projects/${ref}/auth/providers`),
    ])
      .then(([next, list]) => {
        setSettings(next)
        setRows(list)
      })
      .catch((err) => setError(err instanceof Error ? err.message : "auth is unavailable"))
  }

  useEffect(load, [ref])

  async function saveVerification(require_email_verification: boolean) {
    if (!settings) return
    setError("")
    const next = await api<AuthSettings>(`/console/v1/projects/${ref}/auth`, {
      method: "PUT",
      body: JSON.stringify({ ...settings, require_email_verification }),
    })
    setSettings(next)
  }

  async function saveProvider(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setError("")
    setNote("")
    const extra: Record<string, string> = {}
    if (provider === "apple") {
      extra.team_id = teamId
      extra.key_id = keyId
      if (privateKey) extra.private_key = privateKey
    }
    if (provider === "microsoft" && tenant) extra.tenant = tenant
    try {
      const list = await api<Provider[]>(`/console/v1/projects/${ref}/auth/providers`, {
        method: "POST",
        body: JSON.stringify({
          provider,
          client_id: clientId,
          client_secret: secret || undefined,
          redirects: redirects.split("\n").map((line) => line.trim()).filter(Boolean),
          enabled: true,
          extra,
        }),
      })
      setRows(list)
      setSecret("")
      setPrivateKey("")
      setNote("Provider saved.")
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
    }
  }

  async function remove(id: string) {
    setError("")
    await api(`/console/v1/projects/${ref}/auth/providers/${id}`, { method: "DELETE" })
    setRows((list) => list.filter((row) => row.provider !== id))
  }

  if (!admin) return <p className="text-sm text-muted-foreground">Owners and admins can configure providers.</p>

  return (
    <div className="grid max-w-3xl gap-8">
      <div>
        <h1 className="text-2xl font-medium tracking-tight">Providers</h1>
        <p className="text-sm text-muted-foreground">Email is always available. OAuth providers use credentials you supply.</p>
      </div>
      {settings && (
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={settings.require_email_verification}
            onChange={(event) => saveVerification(event.target.checked)}
          />
          Require email verification
        </label>
      )}
      {rows.length > 0 && (
        <ul className="grid gap-2 text-sm">
          {rows.map((row) => (
            <li key={row.provider} className="flex items-center justify-between rounded-md border px-3 py-2">
              <span>
                <span className="capitalize">{row.provider}</span>
                <span className="ml-2 font-mono text-xs text-muted-foreground">{row.callback}</span>
              </span>
              <Button type="button" variant="outline" size="sm" onClick={() => remove(row.provider)}>
                Remove
              </Button>
            </li>
          ))}
        </ul>
      )}
      <form className="grid gap-3" onSubmit={saveProvider}>
        <label className="grid gap-2 text-sm">
          <Label>Provider</Label>
          <select className="h-8 rounded-lg border border-input bg-transparent px-2" value={provider} onChange={(event) => setProvider(event.target.value)}>
            {catalog.map((id) => (
              <option key={id} value={id}>
                {id}
              </option>
            ))}
          </select>
        </label>
        {settings && (
          <p className="font-mono text-xs text-muted-foreground">
            Register this callback: {settings.callback_base}/{provider}
          </p>
        )}
        <label className="grid gap-2">
          <Label>Client id</Label>
          <Input value={clientId} onChange={(event) => setClientId(event.target.value)} />
        </label>
        {provider !== "apple" && (
          <label className="grid gap-2">
            <Label>Client secret</Label>
            <Input type="password" value={secret} onChange={(event) => setSecret(event.target.value)} />
          </label>
        )}
        {provider === "microsoft" && (
          <label className="grid gap-2">
            <Label>Tenant</Label>
            <Input value={tenant} onChange={(event) => setTenant(event.target.value)} />
          </label>
        )}
        {provider === "apple" && (
          <>
            <label className="grid gap-2">
              <Label>Team id</Label>
              <Input value={teamId} onChange={(event) => setTeamId(event.target.value)} />
            </label>
            <label className="grid gap-2">
              <Label>Key id</Label>
              <Input value={keyId} onChange={(event) => setKeyId(event.target.value)} />
            </label>
            <label className="grid gap-2">
              <Label>Private key</Label>
              <textarea
                className="min-h-28 w-full rounded-lg border border-input bg-transparent px-2.5 py-2 font-mono text-sm"
                value={privateKey}
                onChange={(event) => setPrivateKey(event.target.value)}
              />
            </label>
          </>
        )}
        <label className="grid gap-2">
          <Label>Redirect URLs</Label>
          <textarea
            className="min-h-20 w-full rounded-lg border border-input bg-transparent px-2.5 py-2 font-mono text-sm"
            value={redirects}
            placeholder="https://app.example.com/callback"
            onChange={(event) => setRedirects(event.target.value)}
          />
        </label>
        <Button type="submit" className="w-fit">
          Save provider
        </Button>
      </form>
      {note && <p className="text-sm text-muted-foreground">{note}</p>}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}

export function MfaSettings() {
  const { ref = "" } = useParams()
  const { role } = useOutletContext<ConsoleContext>()
  const admin = role === "owner" || role === "admin"
  const [settings, setSettings] = useState<AuthSettings | null>(null)
  const [error, setError] = useState("")

  useEffect(() => {
    api<AuthSettings>(`/console/v1/projects/${ref}/auth`)
      .then(setSettings)
      .catch((err) => setError(err instanceof Error ? err.message : "auth is unavailable"))
  }, [ref])

  async function save(require_mfa: boolean) {
    if (!settings) return
    setError("")
    try {
      const next = await api<AuthSettings>(`/console/v1/projects/${ref}/auth`, {
        method: "PUT",
        body: JSON.stringify({ ...settings, require_mfa }),
      })
      setSettings(next)
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
    }
  }

  if (!admin) return <p className="text-sm text-muted-foreground">Owners and admins can configure 2FA.</p>

  return (
    <div className="grid max-w-xl gap-4">
      <div>
        <h1 className="text-2xl font-medium tracking-tight">2FA</h1>
        <p className="text-sm text-muted-foreground">A second factor applies to password sign-in. OAuth is not challenged.</p>
      </div>
      {settings && (
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={settings.require_mfa} onChange={(event) => save(event.target.checked)} />
          Require a second factor for password sign-in
        </label>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}
