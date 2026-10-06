import { useEffect, useState, type FormEvent } from "react"
import { NavLink, useOutletContext, useParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { api } from "@/lib/api"
import { TrafficCard, type Point } from "@/overview"
import type { ConsoleContext } from "@/shell"
import { Copy, Globe, X } from "lucide-react"

type Deployment = { id: string; status: string; error: string; file_count: number; created_at: string }
type DeploymentDetail = Deployment & { files: string[] }
type EnvRow = { key: string; visibility: string; updated_at: string; value?: string }
type Domain = { host: string; verified_at: string | null; url: string; txt_name?: string; txt_value?: string }
type LogRow = { id: number; name: string; status: number; message: string; created_at: string }

const sections = ["overview", "deployments", "logs", "variables", "domains"] as const

export function Sites() {
  const { ref = "", section = "overview" } = useParams()
  const current = sections.includes(section as (typeof sections)[number]) ? section : "overview"
  return (
    <div className="absolute inset-0 flex min-h-0">
      <aside className="flex w-64 shrink-0 flex-col border-r">
        <div className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
          <Globe className="size-4 shrink-0" />
          <h1 className="text-base font-semibold">Site</h1>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-1 p-2">
          <div className="mb-1 truncate rounded-full bg-muted px-3 py-2 text-center font-mono text-base">{ref}</div>
          {sections.map((id) => (
            <NavLink
              key={id}
              to={id === "overview" ? `/p/${ref}/sites` : `/p/${ref}/sites/${id}`}
              end={id === "overview"}
              className={({ isActive }) => `rounded-md px-2 py-1.5 text-sm capitalize ${isActive ? "bg-muted font-medium" : "hover:bg-muted/60"}`}
            >
              {id}
            </NavLink>
          ))}
        </div>
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">
        {current === "overview" && <Overview refId={ref} />}
        {current === "deployments" && <Deployments refId={ref} />}
        {current === "logs" && <SiteLogs refId={ref} />}
        {current === "variables" && <Variables refId={ref} />}
        {current === "domains" && <Domains refId={ref} />}
      </div>
    </div>
  )
}

function Overview({ refId }: { refId: string }) {
  const [url, setUrl] = useState("")
  const [stats, setStats] = useState<{ hours: number[]; points: Point[] } | null>(null)
  useEffect(() => {
    api<{ url: string }>(`/console/v1/projects/${refId}/sites`).then((body) => setUrl(body.url))
    api<{ hours: number[]; points: Point[] }>(`/console/v1/projects/${refId}/site/stats`).then(setStats)
  }, [refId])
  return (
    <>
      <div className="flex h-12 shrink-0 items-center border-b px-3">
        <h1 className="text-base font-semibold">Overview</h1>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-6">
        <div className="grid max-w-xl gap-4 text-sm">
          {url && (
            <a className="w-fit font-mono text-xs underline" href={url}>
              {url}
            </a>
          )}
          {stats && <TrafficCard label="Site" hours={stats.hours} points={stats.points} />}
        </div>
      </div>
    </>
  )
}

function Deployments({ refId }: { refId: string }) {
  const [rows, setRows] = useState<Deployment[]>([])
  const [selected, setSelected] = useState<DeploymentDetail | null>(null)
  const [error, setError] = useState("")

  useEffect(() => {
    api<Deployment[]>(`/console/v1/projects/${refId}/site/deployments`)
      .then(setRows)
      .catch((err) => setError(err.message))
  }, [refId])

  async function open(id: string) {
    setSelected(await api<DeploymentDetail>(`/console/v1/projects/${refId}/site/deployments/${id}`))
  }

  return (
    <>
      <div className="flex h-12 shrink-0 items-center border-b px-3">
        <h1 className="text-base font-semibold">Deployments</h1>
        {error && <p className="ml-3 truncate text-sm text-destructive">{error}</p>}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full text-sm">
          <thead className="sticky top-0 bg-muted text-left">
            <tr>
              <th className="px-3 py-2 font-medium">Status</th>
              <th className="px-3 py-2 font-medium">Created</th>
              <th className="px-3 py-2 font-medium">Files</th>
              <th className="px-3 py-2 font-medium">Id</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.id} className="cursor-pointer border-t hover:bg-muted/40" onClick={() => open(row.id)}>
                <td className={`px-3 py-1.5 text-xs font-medium ${statusClass(row.status)}`}>{label(row.status)}</td>
                <td className="px-3 py-1.5 font-mono text-xs text-muted-foreground">{row.created_at}</td>
                <td className="px-3 py-1.5 font-mono text-xs">{row.file_count}</td>
                <td className="px-3 py-1.5 font-mono text-xs">{row.id.slice(0, 8)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 && <p className="p-4 text-sm text-muted-foreground">No deployments yet.</p>}
      </div>
      {selected && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={() => setSelected(null)}>
          <div className="flex max-h-[80vh] w-full max-w-lg flex-col rounded-lg border bg-background shadow-lg" onClick={(event) => event.stopPropagation()}>
            <div className="flex h-12 items-center justify-between border-b px-3">
              <span className={`text-sm font-medium ${statusClass(selected.status)}`}>{label(selected.status)}</span>
              <button type="button" aria-label="Close deployment" className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted" onClick={() => setSelected(null)}>
                <X className="size-4" />
              </button>
            </div>
            <div className="min-h-0 overflow-auto p-4 text-sm">
              <p className="font-mono text-xs text-muted-foreground">{selected.created_at}</p>
              {selected.error && <p className="mt-3 text-destructive">{selected.error}</p>}
              <ul className="mt-3 font-mono text-xs">
                {selected.files.map((file) => (
                  <li key={file}>{file}</li>
                ))}
              </ul>
            </div>
          </div>
        </div>
      )}
    </>
  )
}

function SiteLogs({ refId }: { refId: string }) {
  const [rows, setRows] = useState<LogRow[]>([])
  useEffect(() => {
    api<LogRow[]>(`/console/v1/projects/${refId}/logs?kind=site`).then(setRows)
  }, [refId])
  return (
    <>
      <div className="flex h-12 shrink-0 items-center border-b px-3">
        <h1 className="text-base font-semibold">Logs</h1>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full text-sm">
          <thead className="sticky top-0 bg-muted text-left">
            <tr>
              <th className="px-3 py-2 font-medium">Time</th>
              <th className="px-3 py-2 font-medium">Path</th>
              <th className="px-3 py-2 font-medium">Status</th>
              <th className="px-3 py-2 font-medium">Message</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.id} className="border-t">
                <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">{row.created_at}</td>
                <td className="px-3 py-1.5 font-mono text-xs">{row.name}</td>
                <td className={`px-3 py-1.5 font-mono text-xs ${row.status >= 500 ? "text-red-500" : row.status >= 400 ? "text-amber-500" : "text-emerald-600"}`}>{row.status}</td>
                <td className="px-3 py-1.5 font-mono text-xs text-muted-foreground">{row.message}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 && <p className="p-4 text-sm text-muted-foreground">No logs yet.</p>}
      </div>
    </>
  )
}

function Variables({ refId }: { refId: string }) {
  const { role } = useOutletContext<ConsoleContext>()
  const canEdit = role === "owner" || role === "admin"
  const [rows, setRows] = useState<EnvRow[]>([])
  const [visible, setVisible] = useState(false)
  const [error, setError] = useState("")

  function load() {
    api<EnvRow[]>(`/console/v1/projects/${refId}/site/env`)
      .then(setRows)
      .catch((err) => setError(err.message))
  }
  useEffect(load, [refId])

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const data = new FormData(form)
    setError("")
    try {
      await api(`/console/v1/projects/${refId}/site/env`, {
        method: "POST",
        body: JSON.stringify({
          key: data.get("key"),
          value: data.get("value"),
          visibility: visible ? "visible" : "secret",
        }),
      })
      form.reset()
      setVisible(false)
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
    }
  }

  async function remove(key: string) {
    await api(`/console/v1/projects/${refId}/site/env/${encodeURIComponent(key)}`, { method: "DELETE" })
    load()
  }

  return (
    <>
      <div className="flex h-12 shrink-0 items-center border-b px-3">
        <h1 className="text-base font-semibold">Variables</h1>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-4">
        {canEdit && (
          <form className="mb-4 flex flex-wrap items-center gap-2" onSubmit={onSubmit}>
            <Input name="key" placeholder="KEY" required className="w-40" />
            <Input name="value" placeholder="Value" required className="w-64" />
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" className="size-4" checked={visible} onChange={(event) => setVisible(event.target.checked)} />
              Visible
            </label>
            <Button type="submit" size="sm">
              Save
            </Button>
          </form>
        )}
        {error && <p className="mb-3 text-sm text-destructive">{error}</p>}
        <table className="w-full text-sm">
          <thead className="text-left">
            <tr>
              <th className="px-3 py-2 font-medium">Key</th>
              <th className="px-3 py-2 font-medium">Value</th>
              <th className="px-3 py-2 font-medium">Updated</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.key} className="border-t">
                <td className="px-3 py-1.5 font-mono text-xs">{row.key}</td>
                <td className="px-3 py-1.5 font-mono text-xs">{row.visibility === "secret" ? "Secret" : row.value || "Visible"}</td>
                <td className="px-3 py-1.5 font-mono text-xs text-muted-foreground">{row.updated_at}</td>
                <td className="px-3 py-1.5 text-right">
                  {canEdit && (
                    <Button size="sm" variant="outline" onClick={() => remove(row.key)}>
                      Delete
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  )
}

function Domains({ refId }: { refId: string }) {
  const { role } = useOutletContext<ConsoleContext>()
  const canEdit = role === "owner" || role === "admin"
  const [url, setUrl] = useState("")
  const [rows, setRows] = useState<Domain[]>([])
  const [error, setError] = useState("")

  function load() {
    api<{ url: string }>(`/console/v1/projects/${refId}/sites`).then((body) => setUrl(body.url))
    api<Domain[]>(`/console/v1/projects/${refId}/site/domains`)
      .then(setRows)
      .catch((err) => setError(err.message))
  }
  useEffect(load, [refId])

  async function add(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const data = new FormData(form)
    setError("")
    try {
      await api(`/console/v1/projects/${refId}/site/domains`, {
        method: "POST",
        body: JSON.stringify({ host: data.get("host") }),
      })
      form.reset()
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "add failed")
    }
  }

  async function verify(host: string) {
    setError("")
    try {
      await api(`/console/v1/projects/${refId}/site/domains/${encodeURIComponent(host)}/verify`, { method: "POST" })
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "verification failed")
    }
  }

  async function remove(host: string) {
    await api(`/console/v1/projects/${refId}/site/domains/${encodeURIComponent(host)}`, { method: "DELETE" })
    load()
  }

  const target = platformHost(url)

  return (
    <>
      <div className="flex h-12 shrink-0 items-center border-b px-3">
        <h1 className="text-base font-semibold">Domains</h1>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-4">
        <div className="mb-4 rounded-lg border p-3 text-sm">
          <div className="text-xs font-medium text-muted-foreground">Platform</div>
          <a className="font-mono text-xs underline" href={url}>
            {url}
          </a>
          <p className="mt-1 text-xs text-muted-foreground">Covered by the wildcard certificate.</p>
        </div>
        {canEdit && (
          <form className="mb-4 flex gap-2" onSubmit={add}>
            <Input name="host" placeholder="example.com" required className="max-w-sm" />
            <Button type="submit" size="sm">
              Add domain
            </Button>
          </form>
        )}
        {error && <p className="mb-3 text-sm text-destructive">{error}</p>}
        <div className="grid gap-3">
          {rows.map((row) => (
            <div key={row.host} className="rounded-lg border p-3 text-sm">
              <div className="flex items-center justify-between gap-3">
                <span className="font-mono text-xs">{row.host}</span>
                <span className={row.verified_at ? "text-xs text-emerald-600" : "text-xs text-amber-500"}>{row.verified_at ? "Verified" : "Pending"}</span>
              </div>
              {row.verified_at && (
                <>
                  <a className="mt-2 block font-mono text-xs underline" href={row.url}>
                    {row.url}
                  </a>
                  <p className="mt-2 text-xs text-muted-foreground">The proxy issues the certificate. This server does not hold the key.</p>
                </>
              )}
              {!row.verified_at && row.txt_name && row.txt_value && (
                <DnsRecord
                  title="Prove you control this domain"
                  note="Add this TXT record at your DNS provider. If the host field should not include the domain, enter only _reactor-verify. Then press Verify."
                  type="TXT"
                  name={row.txt_name}
                  value={row.txt_value}
                />
              )}
              {target && (
                <DnsRecord
                  title="Send traffic to this site"
                  note={
                    apex(row.host)
                      ? "At the zone apex, add an ALIAS or ANAME record with this target."
                      : "Add this CNAME so requests for this host reach the platform hostname."
                  }
                  type={apex(row.host) ? "ALIAS" : "CNAME"}
                  name={apex(row.host) ? "@" : row.host}
                  value={target}
                />
              )}
              {canEdit && (
                <div className="mt-3 flex gap-2">
                  {!row.verified_at && (
                    <Button size="sm" variant="outline" onClick={() => verify(row.host)}>
                      Verify
                    </Button>
                  )}
                  <Button size="sm" variant="outline" onClick={() => remove(row.host)}>
                    Remove
                  </Button>
                </div>
              )}
            </div>
          ))}
        </div>
      </div>
    </>
  )
}

function platformHost(url: string) {
  try {
    return new URL(url).hostname
  } catch {
    return ""
  }
}

function apex(host: string) {
  return host.split(".").filter(Boolean).length <= 2
}

function DnsRecord({ title, note, type, name, value }: { title: string; note: string; type: string; name: string; value: string }) {
  const [copied, setCopied] = useState("")

  async function copy(label: string, text: string) {
    await navigator.clipboard.writeText(text)
    setCopied(label)
  }

  return (
    <div className="mt-3 grid gap-2 border-t pt-3">
      <div>
        <div className="text-sm font-medium">{title}</div>
        <p className="mt-1 text-xs text-muted-foreground">{note}</p>
      </div>
      <RecordField label="Type" value={type} />
      <RecordField label="Name" value={name} copied={copied === "name"} onCopy={() => copy("name", name)} />
      <RecordField label="Value" value={value} copied={copied === "value"} onCopy={() => copy("value", value)} />
    </div>
  )
}

function RecordField({ label, value, copied, onCopy }: { label: string; value: string; copied?: boolean; onCopy?: () => void }) {
  return (
    <div className="grid grid-cols-[4.5rem_minmax(0,1fr)_auto] items-center gap-2">
      <span className="text-xs text-muted-foreground">{label}</span>
      <code className="truncate rounded-md bg-muted px-2 py-1 font-mono text-xs">{value}</code>
      {onCopy && (
        <Button type="button" variant="outline" size="sm" onClick={onCopy}>
          <Copy className="size-3.5" />
          {copied ? "Copied" : "Copy"}
        </Button>
      )}
    </div>
  )
}

function label(status: string) {
  if (status === "ready") return "Ready"
  if (status === "error") return "Error"
  return "Building"
}

function statusClass(status: string) {
  if (status === "ready") return "text-emerald-600"
  if (status === "error") return "text-red-500"
  return "text-amber-500"
}
