import { PostgreSQL, sql } from "@codemirror/lang-sql"
import { EditorState } from "@codemirror/state"
import { EditorView } from "@codemirror/view"
import { basicSetup } from "codemirror"
import { useEffect, useRef, useState } from "react"
import { useOutletContext, useParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { api, token } from "@/lib/api"
import type { ConsoleContext } from "@/shell"

type TableInfo = { name: string; columns: { name: string }[] }
type SqlResult = {
  command: string
  columns: string[]
  rows: Record<string, unknown>[]
  truncated: boolean
  rows_affected: number
}
type Migration = {
  version: string
  status: string
  source: string | null
  applied_at: string
  revertable: boolean
}

export function Sql() {
  const { ref = "" } = useParams()
  const { role } = useOutletContext<ConsoleContext>()
  const canRun = role === "owner" || role === "admin"
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const [readOnly, setReadOnly] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const [warnings, setWarnings] = useState<string[]>([])
  const [results, setResults] = useState<SqlResult[]>([])
  const [history, setHistory] = useState<Migration[]>([])
  const [version, setVersion] = useState("")
  const [down, setDown] = useState("")
  const [migration, setMigration] = useState(false)

  useEffect(() => {
    let cancelled = false
    api<TableInfo[]>(`/console/v1/projects/${ref}/schema`)
      .then((tables) => {
        if (cancelled || !host.current) return
        const schema = Object.fromEntries(tables.map((table) => [table.name, table.columns.map((column) => column.name)]))
        const state = EditorState.create({
          doc: "SELECT 1;",
          extensions: [basicSetup, sql({ dialect: PostgreSQL, schema }), EditorView.lineWrapping],
        })
        view.current?.destroy()
        view.current = new EditorView({ state, parent: host.current })
      })
      .catch(() => {
        if (!host.current) return
        view.current = new EditorView({
          state: EditorState.create({
            doc: "SELECT 1;",
            extensions: [basicSetup, sql({ dialect: PostgreSQL }), EditorView.lineWrapping],
          }),
          parent: host.current,
        })
      })
    return () => {
      cancelled = true
      view.current?.destroy()
      view.current = null
    }
  }, [ref])

  useEffect(() => {
    if (!canRun) return
    api<Migration[]>(`/console/v1/projects/${ref}/migrations`)
      .then(setHistory)
      .catch(() => setHistory([]))
  }, [ref, canRun, results])

  async function execute(confirm: boolean) {
    const query = view.current?.state.doc.toString() ?? ""
    setBusy(true)
    setError("")
    try {
      const response = await fetch(`/console/v1/projects/${ref}/sql`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          authorization: `Bearer ${token() ?? ""}`,
        },
        body: JSON.stringify({ sql: query, mode: "run", read_only: readOnly, confirm }),
      })
      const body = await response.json()
      if (response.status === 409 && Array.isArray(body.warnings)) {
        setWarnings(body.warnings)
        return
      }
      if (!response.ok) throw new Error(body.error || response.statusText)
      setWarnings([])
      setResults(body.results ?? [])
    } catch (err) {
      setError(err instanceof Error ? err.message : "query failed")
    } finally {
      setBusy(false)
    }
  }

  async function runMigration() {
    const query = view.current?.state.doc.toString() ?? ""
    setBusy(true)
    setError("")
    try {
      await api(`/console/v1/projects/${ref}/sql`, {
        method: "POST",
        body: JSON.stringify({
          sql: query,
          mode: "migration",
          version,
          down_sql: down.trim() ? down : undefined,
        }),
      })
      setMigration(false)
      setVersion("")
      setDown("")
      setResults([])
      const next = await api<Migration[]>(`/console/v1/projects/${ref}/migrations`)
      setHistory(next)
    } catch (err) {
      setError(err instanceof Error ? err.message : "migration failed")
    } finally {
      setBusy(false)
    }
  }

  async function revert(item: string) {
    setBusy(true)
    setError("")
    try {
      await api(`/console/v1/projects/${ref}/migrations/${encodeURIComponent(item)}/revert`, { method: "POST" })
      const next = await api<Migration[]>(`/console/v1/projects/${ref}/migrations`)
      setHistory(next)
    } catch (err) {
      setError(err instanceof Error ? err.message : "revert failed")
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="grid gap-6">
      <div>
        <h1 className="text-2xl font-medium tracking-tight">SQL</h1>
        <p className="text-sm text-muted-foreground">Runs as the project database role. Read-only unless you turn that off.</p>
      </div>
      <div ref={host} className="min-h-40 overflow-hidden rounded-lg border" />
      <div className="flex flex-wrap items-center gap-3">
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={readOnly} onChange={(event) => setReadOnly(event.target.checked)} />
          Read only
        </label>
        <Button type="button" disabled={!canRun || busy} onClick={() => execute(false)}>
          Run
        </Button>
        <Button type="button" variant="outline" disabled={!canRun || busy} onClick={() => setMigration(true)}>
          Run as migration
        </Button>
        {!canRun && <p className="text-sm text-muted-foreground">Owners and admins can run SQL.</p>}
      </div>
      {warnings.length > 0 && (
        <div className="grid gap-2 rounded-lg border border-destructive/40 p-3">
          <p className="text-sm">This query needs confirmation: {warnings.join(", ")}</p>
          <div>
            <Button type="button" variant="destructive" disabled={busy} onClick={() => execute(true)}>
              Confirm and run
            </Button>
          </div>
        </div>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
      {results.map((result, index) => (
        <ResultGrid key={index} result={result} />
      ))}
      <section className="grid gap-2">
        <h2 className="text-sm font-medium">Migrations</h2>
        {history.length === 0 && <p className="text-sm text-muted-foreground">No migrations yet.</p>}
        {history.map((item) => (
          <div key={item.version} className="flex items-center justify-between gap-3 rounded-lg border px-3 py-2">
            <div>
              <div className="font-mono text-sm">{item.version}</div>
              <p className="text-xs text-muted-foreground">
                {item.status}
                {item.source ? ` · ${item.source}` : ""}
              </p>
            </div>
            {item.revertable && item.status === "applied" && (
              <Button type="button" variant="outline" disabled={!canRun || busy} onClick={() => revert(item.version)}>
                Revert
              </Button>
            )}
          </div>
        ))}
      </section>
      {migration && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <form
            className="grid w-full max-w-md gap-3 rounded-lg border bg-background p-4 shadow-lg"
            onSubmit={(event) => {
              event.preventDefault()
              runMigration()
            }}
          >
            <h2 className="text-base font-medium">Run as migration</h2>
            <Input value={version} placeholder="0004_change.sql" onChange={(event) => setVersion(event.target.value)} />
            <textarea
              className="min-h-24 rounded-md border bg-transparent px-3 py-2 text-sm"
              value={down}
              placeholder="Optional down SQL"
              onChange={(event) => setDown(event.target.value)}
            />
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setMigration(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={busy || version.trim() === ""}>
                Apply
              </Button>
            </div>
          </form>
        </div>
      )}
    </div>
  )
}

function ResultGrid({ result }: { result: SqlResult }) {
  if (result.rows.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        {result.command} · {result.rows_affected} row{result.rows_affected === 1 ? "" : "s"}
      </p>
    )
  }
  return (
    <div className="overflow-auto rounded-lg border">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b text-left">
            {result.columns.map((column) => (
              <th key={column} className="px-3 py-2 font-medium">
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {result.rows.map((row, index) => (
            <tr key={index} className="border-b last:border-0">
              {result.columns.map((column) => (
                <td key={column} className="px-3 py-2 font-mono">
                  {formatCell(row[column])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {result.truncated && <p className="px-3 py-2 text-xs text-muted-foreground">Showing the first 1000 rows.</p>}
    </div>
  )
}

function formatCell(value: unknown) {
  if (value == null) return ""
  if (typeof value === "object") return JSON.stringify(value)
  return String(value)
}
