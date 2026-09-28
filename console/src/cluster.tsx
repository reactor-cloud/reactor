import { useEffect, useState, type FormEvent } from "react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
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

export function Cluster() {
  const [info, setInfo] = useState<ClusterInfo | null>(null)
  const [name, setName] = useState("")
  const [error, setError] = useState("")

  useEffect(() => {
    api<ClusterInfo>("/console/v1/cluster")
      .then((cluster) => {
        setInfo(cluster)
        setName(cluster.name)
      })
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
