import { useEffect, useState } from "react"
import { useParams } from "react-router-dom"
import { Input } from "@/components/ui/input"
import { api } from "@/lib/api"
import { X } from "lucide-react"

type LogRow = {
  id: number
  kind: string
  name: string
  status: number
  message: string
  created_at: string
}

export function Logs() {
  const { ref = "" } = useParams()
  const [q, setQ] = useState("")
  const [rows, setRows] = useState<LogRow[]>([])
  const [selected, setSelected] = useState<LogRow | null>(null)
  const [error, setError] = useState("")

  useEffect(() => {
    const query = q ? `?q=${encodeURIComponent(q)}` : ""
    api<LogRow[]>(`/console/v1/projects/${ref}/logs${query}`)
      .then((next) => {
        setRows(next)
        setError("")
        setSelected((current) => next.find((row) => row.id === current?.id) || null)
      })
      .catch((err) => setError(err.message))
  }, [ref, q])

  return (
    <div className="absolute inset-0 flex min-h-0">
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-12 shrink-0 items-center gap-3 border-b px-3">
          <h1 className="shrink-0 text-base font-semibold">Logs</h1>
          <Input value={q} placeholder="Search kind, name, status, or message" className="max-w-sm" onChange={(event) => setQ(event.target.value)} />
          {error && <p className="truncate text-sm text-destructive">{error}</p>}
        </div>
        <div className="min-h-0 flex-1 overflow-auto">
          <table className="w-full text-sm">
            <thead className="sticky top-0 z-10 bg-muted text-left">
              <tr>
                <th className="whitespace-nowrap px-3 py-2 font-medium">Time</th>
                <th className="whitespace-nowrap px-3 py-2 font-medium">Kind</th>
                <th className="whitespace-nowrap px-3 py-2 font-medium">Name</th>
                <th className="whitespace-nowrap px-3 py-2 font-medium">Status</th>
                <th className="px-3 py-2 font-medium">Message</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr
                  key={row.id}
                  className={`cursor-pointer border-t hover:bg-muted/40 ${selected?.id === row.id ? "bg-muted" : ""}`}
                  onClick={() => setSelected(row)}
                >
                  <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">{row.created_at}</td>
                  <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">{row.kind}</td>
                  <td className="max-w-48 truncate px-3 py-1.5 font-mono text-xs">{row.name}</td>
                  <td className={`whitespace-nowrap px-3 py-1.5 font-mono text-xs ${statusClass(row.status)}`}>{row.status}</td>
                  <td className="max-w-md truncate px-3 py-1.5 font-mono text-xs text-muted-foreground">{row.message}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length === 0 && <p className="p-4 text-sm text-muted-foreground">Nothing here yet.</p>}
        </div>
        <div className="flex h-10 shrink-0 items-center border-t px-3 text-xs text-muted-foreground">{rows.length} lines</div>
      </div>
      {selected && <Detail row={selected} onClose={() => setSelected(null)} />}
    </div>
  )
}

function Detail({ row, onClose }: { row: LogRow; onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={onClose}>
      <div className="flex max-h-[80vh] w-full max-w-lg flex-col rounded-lg border bg-background shadow-lg" onClick={(event) => event.stopPropagation()}>
      <div className="flex h-12 shrink-0 items-center justify-between border-b px-3">
        <span className="text-sm font-medium">Log {row.id}</span>
        <button type="button" aria-label="Close log" className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted" onClick={onClose}>
          <X className="size-4" />
        </button>
      </div>
      <dl className="min-h-0 overflow-auto p-4">
        <Field label="Time" value={row.created_at} />
        <Field label="Kind" value={row.kind} />
        <Field label="Name" value={row.name} />
        <div className="mt-3">
          <dt className="text-xs text-muted-foreground">Status</dt>
          <dd className={`mt-1 font-mono text-sm ${statusClass(row.status)}`}>{row.status}</dd>
        </div>
        <div className="mt-3">
          <dt className="text-xs text-muted-foreground">Message</dt>
          <dd className="mt-1 whitespace-pre-wrap break-all font-mono text-xs">{row.message || "—"}</dd>
        </div>
      </dl>
      </div>
    </div>
  )
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div className="mt-3 first:mt-0">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="mt-1 break-all font-mono text-xs">{value}</dd>
    </div>
  )
}

function statusClass(status: number) {
  if (status >= 500) return "text-red-600"
  if (status >= 400) return "text-amber-600"
  return "text-emerald-600"
}
