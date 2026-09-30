import { useEffect, useState, type FormEvent } from "react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api, placeLabel } from "@/lib/api"

type ClusterInfo = {
  name: string
  place: string
  type: string
  functions_runtime: string
  reactor: { ok: boolean }
  postgres: { ok: boolean; bytes?: number; connections?: number }
  postgrest: { ok: boolean }
  postgrest_dedicated: { active: boolean; ok?: boolean } | null
  blobs: { ok: boolean; backend?: string }
}

type ClusterMail = {
  host: string
  port: number
  username: string
  password_set: boolean
  from_address: string
  tls: string
}

export function Cluster() {
  const [info, setInfo] = useState<ClusterInfo | null>(null)
  const [name, setName] = useState("")
  const [mail, setMail] = useState<ClusterMail | null>(null)
  const [password, setPassword] = useState("")
  const [error, setError] = useState("")
  const [note, setNote] = useState("")

  useEffect(() => {
    api<ClusterInfo>("/console/v1/cluster")
      .then((cluster) => {
        setInfo(cluster)
        setName(cluster.name)
      })
      .catch((err) => setError(err.message))
    api<ClusterMail>("/console/v1/cluster/email")
      .then(setMail)
      .catch((err) => setError(err.message))
  }, [])

  async function saveName(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    try {
      const saved = await api<{ name: string }>("/console/v1/cluster", {
        method: "POST",
        body: JSON.stringify({ cluster_name: name }),
      })
      setName(saved.name)
      setInfo((current) => (current ? { ...current, name: saved.name } : current))
      setError("")
    } catch (err) {
      setError(err instanceof Error ? err.message : "rename failed")
    }
  }

  async function saveMail(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!mail) return
    try {
      const saved = await api<ClusterMail>("/console/v1/cluster/email", {
        method: "PUT",
        body: JSON.stringify({ ...mail, password: password || undefined }),
      })
      setMail(saved)
      setPassword("")
      setNote("Mail server saved.")
      setError("")
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
    }
  }

  const typeLabel = info ? placeLabel(info.place) : ""

  return (
    <div className="mx-auto grid max-w-3xl gap-8">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-2xl font-medium tracking-tight">Cluster</h1>
          <p className="text-sm text-muted-foreground">
            {info ? `Running on ${typeLabel}. Functions run on ${info.functions_runtime}.` : "Cluster settings."}
          </p>
        </div>
        {info && <Badge variant="outline">{typeLabel}</Badge>}
      </div>
      {error && <p className="text-sm text-destructive">{error}</p>}

      <section className="grid gap-3">
        <h2 className="text-sm font-medium">Name</h2>
        <form className="flex max-w-md items-center gap-2" onSubmit={saveName}>
          <Input value={name} onChange={(event) => setName(event.target.value)} required />
          <Button type="submit" variant="outline">
            Save
          </Button>
        </form>
      </section>

      {mail && (
        <section className="grid gap-3">
          <h2 className="text-sm font-medium">Mail</h2>
          <p className="text-sm text-muted-foreground">
            Auth mail for a project uses this server when a platform admin allows it and the project has no server of its own.
          </p>
          <form className="grid gap-3" onSubmit={saveMail}>
            <div className="grid gap-3 sm:grid-cols-3">
              <MailField label="Host" value={mail.host} onChange={(host) => setMail({ ...mail, host })} />
              <MailField label="Port" value={String(mail.port)} onChange={(port) => setMail({ ...mail, port: Number(port) })} />
              <label className="grid gap-2 text-sm">
                <Label>TLS</Label>
                <select
                  className="h-8 rounded-lg border border-input bg-transparent px-2"
                  value={mail.tls}
                  onChange={(event) => setMail({ ...mail, tls: event.target.value })}
                >
                  <option value="starttls">STARTTLS</option>
                  <option value="tls">TLS</option>
                  <option value="none">None</option>
                </select>
              </label>
            </div>
            <MailField label="Username" value={mail.username} onChange={(username) => setMail({ ...mail, username })} />
            <label className="grid gap-2">
              <Label>Password</Label>
              <Input
                type="password"
                value={password}
                placeholder={mail.password_set ? "Saved. Enter a new password to replace it." : ""}
                onChange={(event) => setPassword(event.target.value)}
              />
            </label>
            <MailField label="From address" value={mail.from_address} onChange={(from_address) => setMail({ ...mail, from_address })} />
            <Button type="submit" variant="outline" className="w-fit">
              Save mail server
            </Button>
          </form>
          {note && <p className="text-sm text-muted-foreground">{note}</p>}
        </section>
      )}

      {info && (
        <section className="grid gap-3">
          <h2 className="text-sm font-medium">Health</h2>
          <ul className="divide-y rounded-lg border">
            <HealthRow name="Reactor" detail="API" ok={info.reactor.ok} />
            <HealthRow
              name="Postgres"
              detail={[info.postgres.connections != null ? `${info.postgres.connections} connections` : "", size(info.postgres.bytes)]
                .filter(Boolean)
                .join(" · ")}
              ok={info.postgres.ok}
            />
            <HealthRow name="PostgREST" detail="Project data API" ok={info.postgrest.ok} />
            {info.postgrest_dedicated && (
              <HealthRow
                name="Dedicated PostgREST"
                detail="Projects on their own database"
                ok={info.postgrest_dedicated.ok === true}
                active={info.postgrest_dedicated.active}
              />
            )}
            <HealthRow name="Storage" detail={info.blobs.backend || ""} ok={info.blobs.ok} />
          </ul>
        </section>
      )}
    </div>
  )
}

function HealthRow({ name, detail, ok, active = true }: { name: string; detail: string; ok: boolean; active?: boolean }) {
  const tone = !active
    ? "bg-muted text-muted-foreground"
    : ok
      ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400"
      : "bg-red-500/15 text-red-700 dark:text-red-400"
  const dot = !active ? "bg-muted-foreground" : ok ? "bg-emerald-500" : "bg-red-500"
  const label = !active ? "Inactive" : ok ? "Healthy" : "Down"
  return (
    <li className="flex items-center justify-between gap-3 px-4 py-3">
      <div className="min-w-0">
        <div className="text-sm font-medium">{name}</div>
        {detail && <div className="truncate text-xs text-muted-foreground">{detail}</div>}
      </div>
      <span className={`inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium ${tone}`}>
        <span className={`size-1.5 rounded-full ${dot}`} />
        {label}
      </span>
    </li>
  )
}

function size(bytes?: number) {
  if (bytes == null) return ""
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function MailField({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return (
    <label className="grid gap-2">
      <Label>{label}</Label>
      <Input value={value} onChange={(event) => onChange(event.target.value)} />
    </label>
  )
}
