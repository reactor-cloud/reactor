import { useEffect, useRef, useState, type FormEvent } from "react"
import { NavLink, useNavigate, useOutletContext, useParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { api, token } from "@/lib/api"
import { TrafficCard, type Point } from "@/overview"
import type { ConsoleContext } from "@/shell"

type Row = { name: string; version: number; live: boolean; pinned: boolean; created_at: string }
type LogRow = { id: number; kind: string; name: string; status: number; message: string; created_at: string }

const sections = ["overview", "versions", "logs", "variables", "test"] as const

export function Functions() {
  const { name } = useParams()
  if (name) return <FunctionDetail />
  return <FunctionList />
}

function FunctionList() {
  const { ref = "" } = useParams()
  const navigate = useNavigate()
  const { role } = useOutletContext<ConsoleContext>()
  const canDeploy = role === "owner" || role === "admin"
  const [rows, setRows] = useState<Row[]>([])
  const [q, setQ] = useState("")
  const [creating, setCreating] = useState(false)
  const [error, setError] = useState("")

  function load() {
    api<Row[]>(`/console/v1/projects/${ref}/functions`)
      .then(setRows)
      .catch((err) => setError(err.message))
  }
  useEffect(load, [ref])

  const names = [...new Set(rows.map((row) => row.name))].filter((name) => name.toLowerCase().includes(q.trim().toLowerCase()))

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-3 border-b px-3">
        <h1 className="shrink-0 text-base font-semibold">Functions</h1>
        <Input value={q} placeholder="Search functions" className="ml-auto w-56 shrink-0" onChange={(event) => setQ(event.target.value)} />
        {error && <p className="max-w-40 truncate text-sm text-destructive">{error}</p>}
        {canDeploy && (
          <Button size="sm" variant="outline" onClick={() => setCreating(true)}>
            Add function
          </Button>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full text-sm">
          <thead className="sticky top-0 z-10 bg-muted text-left">
            <tr>
              <th className="px-3 py-2 font-medium">Name</th>
              <th className="px-3 py-2 font-medium">Live</th>
              <th className="px-3 py-2 font-medium">Latest</th>
              <th className="px-3 py-2 font-medium">Updated</th>
            </tr>
          </thead>
          <tbody>
            {names.map((name) => {
              const versions = rows.filter((row) => row.name === name)
              const live = versions.find((row) => row.live)
              const latest = versions[0]
              return (
                <tr key={name} className="cursor-pointer border-t hover:bg-muted/40" onClick={() => navigate(`/p/${ref}/functions/${name}`)}>
                  <td className="px-3 py-1.5 font-mono text-xs">{name}</td>
                  <td className="px-3 py-1.5 font-mono text-xs">{live ? `v${live.version}${live.pinned ? " pinned" : ""}` : "—"}</td>
                  <td className="px-3 py-1.5 font-mono text-xs">v{latest.version}</td>
                  <td className="px-3 py-1.5 font-mono text-xs text-muted-foreground">{latest.created_at}</td>
                </tr>
              )
            })}
          </tbody>
        </table>
        {names.length === 0 && <p className="p-4 text-sm text-muted-foreground">No functions yet.</p>}
      </div>
      <div className="flex h-10 shrink-0 items-center border-t px-3 text-xs text-muted-foreground">{names.length} functions</div>
      {creating && (
        <CreateFunction
          refId={ref}
          onClose={() => setCreating(false)}
          onCreated={(name) => {
            setCreating(false)
            navigate(`/p/${ref}/functions/${name}`)
          }}
        />
      )}
    </div>
  )
}

function FunctionDetail() {
  const { ref = "", name = "", section = "overview" } = useParams()
  const { role } = useOutletContext<ConsoleContext>()
  const canDeploy = role === "owner" || role === "admin"
  const [rows, setRows] = useState<Row[]>([])
  const [error, setError] = useState("")
  const current = sections.includes(section as (typeof sections)[number]) ? section : "overview"

  function load() {
    api<Row[]>(`/console/v1/projects/${ref}/functions`)
      .then((next) => setRows(next.filter((row) => row.name === name)))
      .catch((err) => setError(err.message))
  }
  useEffect(load, [ref, name])

  const live = rows.find((row) => row.live)
  const pinned = rows.some((row) => row.pinned)

  async function promote(version: number) {
    setError("")
    try {
      await api(`/console/v1/projects/${ref}/functions/${name}/promote`, { method: "POST", body: JSON.stringify({ version }) })
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "promote failed")
    }
  }

  async function demote() {
    setError("")
    try {
      await api(`/console/v1/projects/${ref}/functions/${name}/demote`, { method: "POST" })
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "demote failed")
    }
  }

  return (
    <div className="absolute inset-0 flex min-h-0">
      <aside className="flex w-64 shrink-0 flex-col border-r">
        <div className="flex h-12 shrink-0 items-center border-b px-3">
          <h1 className="text-base font-semibold">Function</h1>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-1 p-2">
          <div className="mb-1 truncate rounded-full bg-muted px-3 py-2 text-center font-mono text-base">{name}</div>
          {sections.map((id) => (
          <NavLink
            key={id}
            to={id === "overview" ? `/p/${ref}/functions/${name}` : `/p/${ref}/functions/${name}/${id}`}
            end={id === "overview"}
            className={({ isActive }) => `rounded-md px-2 py-1.5 text-sm capitalize ${isActive ? "bg-muted font-medium" : "hover:bg-muted/60"}`}
          >
            {id}
            </NavLink>
          ))}
        </div>
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">
        {(current === "overview" || current === "versions") && (
          <div className="flex h-12 shrink-0 items-center gap-3 border-b px-3">
            <h1 className="shrink-0 text-base font-semibold capitalize">{current}</h1>
            {canDeploy && <ZipButton label="Add version" refId={ref} name={name} onDone={load} />}
            {error && <p className="truncate text-sm text-destructive">{error}</p>}
          </div>
        )}
        {current === "overview" && (
          <div className="min-h-0 flex-1 overflow-auto p-6">
            <div className="grid max-w-xl gap-4 text-sm">
              <p>Requests use the latest version unless one is pinned.</p>
              <p>
                Responding: {live ? <span className="font-mono">v{live.version}</span> : "none"}
                {live?.pinned ? " (pinned)" : live ? " (latest)" : ""}
              </p>
              {canDeploy && pinned && (
                <Button variant="outline" className="w-fit" onClick={demote}>
                  Demote
                </Button>
              )}
              <FunctionStats refId={ref} name={name} />
            </div>
          </div>
        )}
        {current === "versions" && (
          <div className="min-h-0 flex-1 overflow-auto">
            <table className="w-full text-sm">
              <thead className="sticky top-0 bg-muted text-left">
                <tr>
                  <th className="px-3 py-2 font-medium">Version</th>
                  <th className="px-3 py-2 font-medium">Created</th>
                  <th className="px-3 py-2 font-medium">Status</th>
                  <th className="px-3 py-2 font-medium" />
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <tr key={row.version} className="border-t">
                    <td className="px-3 py-1.5 font-mono text-xs">v{row.version}</td>
                    <td className="px-3 py-1.5 font-mono text-xs text-muted-foreground">{row.created_at}</td>
                    <td className="px-3 py-1.5 text-xs">{row.pinned ? "Pinned" : row.live ? "Latest" : ""}</td>
                    <td className="px-3 py-1.5 text-right">
                      {canDeploy && !row.pinned && (
                        <Button size="sm" variant="outline" onClick={() => promote(row.version)}>
                          Promote
                        </Button>
                      )}
                      {canDeploy && row.pinned && (
                        <Button size="sm" variant="outline" onClick={demote}>
                          Demote
                        </Button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {current === "logs" && <FunctionLogs refId={ref} name={name} />}
        {current === "variables" && <FunctionEnv refId={ref} name={name} />}
        {current === "test" && <FunctionTest refId={ref} name={name} rows={rows} />}
      </div>
    </div>
  )
}

function ZipButton({ label, refId, name, onDone }: { label: string; refId: string; name: string; onDone: () => void }) {
  const fileRef = useRef<HTMLInputElement>(null)
  const [promote, setPromote] = useState(true)
  const [error, setError] = useState("")

  async function upload(file: File | undefined) {
    if (!file) return
    setError("")
    try {
      await sendZip(refId, name, file, promote)
      onDone()
    } catch (err) {
      setError(err instanceof Error ? err.message : "upload failed")
    }
  }

  return (
    <div className="ml-auto flex items-center gap-3">
      {error && <p className="max-w-40 truncate text-sm text-destructive">{error}</p>}
      <label className="flex items-center gap-2 text-sm">
        <input type="checkbox" className="size-4" checked={promote} onChange={(event) => setPromote(event.target.checked)} />
        Promote
      </label>
      <Button size="sm" variant="outline" onClick={() => fileRef.current?.click()}>
        {label}
      </Button>
      <input
        ref={fileRef}
        type="file"
        accept=".zip,application/zip"
        className="hidden"
        onChange={(event) => {
          upload(event.target.files?.[0])
          event.target.value = ""
        }}
      />
    </div>
  )
}

function CreateFunction({ refId, onClose, onCreated }: { refId: string; onClose: () => void; onCreated: (name: string) => void }) {
  const fileRef = useRef<HTMLInputElement>(null)
  const [fnName, setFnName] = useState("")
  const [file, setFile] = useState<File | null>(null)
  const [promote, setPromote] = useState(true)
  const [error, setError] = useState("")

  async function submit() {
    if (!file || !fnName.trim()) return
    setError("")
    try {
      await sendZip(refId, fnName.trim(), file, promote)
      onCreated(fnName.trim())
    } catch (err) {
      setError(err instanceof Error ? err.message : "upload failed")
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={onClose}>
      <div className="grid w-full max-w-md gap-3 rounded-lg border bg-background p-4 shadow-lg" onClick={(event) => event.stopPropagation()}>
        <h2 className="text-base font-semibold">Add function</h2>
        <Input value={fnName} placeholder="function_name" onChange={(event) => setFnName(event.target.value)} />
        <Button type="button" variant="outline" onClick={() => fileRef.current?.click()}>
          {file ? file.name : "Choose zip"}
        </Button>
        <input
          ref={fileRef}
          type="file"
          accept=".zip,application/zip"
          className="hidden"
          onChange={(event) => setFile(event.target.files?.[0] || null)}
        />
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" className="size-4" checked={promote} onChange={(event) => setPromote(event.target.checked)} />
          Promote
        </label>
        {error && <p className="text-sm text-destructive">{error}</p>}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button disabled={!file || !fnName.trim()} onClick={submit}>
            Deploy
          </Button>
        </div>
      </div>
    </div>
  )
}

type EnvRow = { key: string; updated_at: string }

function FunctionEnv({ refId, name }: { refId: string; name: string }) {
  const { role } = useOutletContext<ConsoleContext>()
  const canEdit = role === "owner" || role === "admin"
  const [rows, setRows] = useState<EnvRow[]>([])
  const [error, setError] = useState("")

  function load() {
    api<EnvRow[]>(`/console/v1/projects/${refId}/functions/${encodeURIComponent(name)}/env`)
      .then(setRows)
      .catch((err) => setError(err.message))
  }
  useEffect(load, [refId, name])

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const data = new FormData(form)
    setError("")
    try {
      await api(`/console/v1/projects/${refId}/functions/${encodeURIComponent(name)}/env`, {
        method: "POST",
        body: JSON.stringify({ key: data.get("key"), value: data.get("value") }),
      })
      form.reset()
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
    }
  }

  async function remove(key: string) {
    await api(`/console/v1/projects/${refId}/functions/${encodeURIComponent(name)}/env/${encodeURIComponent(key)}`, { method: "DELETE" })
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
            <Input name="value" type="password" placeholder="Value" required className="w-64" />
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
                <td className="px-3 py-1.5 font-mono text-xs">Secret</td>
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

function FunctionLogs({ refId, name }: { refId: string; name: string }) {
  const [rows, setRows] = useState<LogRow[]>([])
  useEffect(() => {
    api<LogRow[]>(`/console/v1/projects/${refId}/logs?kind=function&name=${encodeURIComponent(name)}`).then(setRows)
  }, [refId, name])
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <table className="w-full text-sm">
        <thead className="sticky top-0 bg-muted text-left">
          <tr>
            <th className="px-3 py-2 font-medium">Time</th>
            <th className="px-3 py-2 font-medium">Status</th>
            <th className="px-3 py-2 font-medium">Message</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.id} className="border-t">
              <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">{row.created_at}</td>
              <td className="px-3 py-1.5 font-mono text-xs">{row.status}</td>
              <td className="px-3 py-1.5 font-mono text-xs text-muted-foreground">{row.message}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && <p className="p-4 text-sm text-muted-foreground">No logs yet.</p>}
    </div>
  )
}

function FunctionStats({ refId, name }: { refId: string; name: string }) {
  const [body, setBody] = useState<{ hours: number[]; points: Point[] } | null>(null)
  useEffect(() => {
    api<{ hours: number[]; points: Point[] }>(`/console/v1/projects/${refId}/functions/${name}/stats`).then(setBody)
  }, [refId, name])
  if (!body) return null
  return <TrafficCard label={name} hours={body.hours} points={body.points} />
}

function FunctionTest({ refId, name, rows }: { refId: string; name: string; rows: Row[] }) {
  const [version, setVersion] = useState("live")
  const [payload, setPayload] = useState("{}")
  const [result, setResult] = useState("")
  const [error, setError] = useState("")

  async function invoke() {
    setError("")
    setResult("")
    const query = version === "live" ? "" : `?version=${version}`
    try {
      const body = await api<{ status: number; body: string }>(`/console/v1/projects/${refId}/functions/${name}/invoke${query}`, {
        method: "POST",
        body: payload,
      })
      setResult(`${body.status}\n${body.body}`)
    } catch (err) {
      setError(err instanceof Error ? err.message : "invoke failed")
    }
  }

  return (
    <div className="grid max-w-xl gap-3 p-6">
      <div className="flex h-12 items-center gap-3">
        <h1 className="text-base font-semibold">Test</h1>
      </div>
      <label className="grid gap-1 text-sm">
        Version
        <select className="h-9 rounded-md border bg-background px-2" value={version} onChange={(event) => setVersion(event.target.value)}>
          <option value="live">Live</option>
          {rows.map((row) => (
            <option key={row.version} value={row.version}>
              v{row.version}
            </option>
          ))}
        </select>
      </label>
      <textarea className="min-h-32 rounded-md border bg-background p-2 font-mono text-xs" value={payload} onChange={(event) => setPayload(event.target.value)} />
      <Button className="w-fit" onClick={invoke}>
        Invoke
      </Button>
      {error && <p className="text-sm text-destructive">{error}</p>}
      {result && <pre className="overflow-auto rounded-md border bg-muted p-3 font-mono text-xs">{result}</pre>}
    </div>
  )
}

async function sendZip(refId: string, name: string, file: File, promote: boolean) {
  const headers = new Headers({ "content-type": "application/zip" })
  const current = token()
  if (current) headers.set("authorization", `Bearer ${current}`)
  const response = await fetch(`/console/v1/projects/${refId}/functions/${encodeURIComponent(name)}?promote=${promote}`, {
    method: "POST",
    headers,
    body: file,
  })
  const text = await response.text()
  const body = text ? JSON.parse(text) : null
  if (!response.ok) throw new Error(body?.error || response.statusText)
  return body
}
