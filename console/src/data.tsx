import { useEffect, useState, type FormEvent } from "react"
import { useNavigate, useOutletContext, useParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { api } from "@/lib/api"
import type { ConsoleContext } from "@/shell"
import { ChevronLeft, ChevronRight, Plus, X } from "lucide-react"

type Column = { name: string; type: string }
type TableInfo = { name: string; primary_key: string | null; columns: Column[] }

const types = ["text", "uuid", "integer", "bigint", "boolean", "timestamptz", "jsonb", "numeric"]

export function Data() {
  const { ref = "", table = "" } = useParams()
  const navigate = useNavigate()
  const { role } = useOutletContext<ConsoleContext>()
  const canEdit = role === "owner" || role === "admin"
  const [tables, setTables] = useState<TableInfo[]>([])
  const [open, setOpen] = useState<string[]>(() => tabs(ref))
  const [creating, setCreating] = useState(false)
  const [filter, setFilter] = useState("")
  const [columns, setColumns] = useState<{ name: string; type: string }[]>([{ name: "", type: "text" }])
  const [error, setError] = useState("")

  function load() {
    api<TableInfo[]>(`/console/v1/projects/${ref}/schema`).then(setTables)
  }
  useEffect(load, [ref])
  useEffect(() => {
    if (!table || open.includes(table)) return
    const next = [...open, table]
    setOpen(next)
    sessionStorage.setItem(`reactor.tabs.${ref}`, JSON.stringify(next))
  }, [table, ref, open])

  function openTable(name: string) {
    const next = open.includes(name) ? open : [...open, name]
    setOpen(next)
    sessionStorage.setItem(`reactor.tabs.${ref}`, JSON.stringify(next))
    navigate(`/p/${ref}/data/${name}`)
  }

  function closeTable(name: string) {
    const next = open.filter((item) => item !== name)
    setOpen(next)
    sessionStorage.setItem(`reactor.tabs.${ref}`, JSON.stringify(next))
    if (table === name) navigate(next[0] ? `/p/${ref}/data/${next[0]}` : `/p/${ref}/data`)
  }

  async function createTable(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    const spec = columns.filter((column) => column.name.trim())
    try {
      await api(`/console/v1/projects/${ref}/tables`, {
        method: "POST",
        body: JSON.stringify({ name: data.get("name"), columns: spec }),
      })
      setCreating(false)
      setColumns([{ name: "", type: "text" }])
      load()
      openTable(String(data.get("name")))
    } catch (err) {
      setError(err instanceof Error ? err.message : "create failed")
    }
  }

  const shown = tables.filter((item) => item.name.includes(filter.trim().toLowerCase()))
  const current = tables.find((item) => item.name === table)

  return (
    <div className="absolute inset-0 flex min-h-0">
      <aside className="flex w-56 shrink-0 flex-col border-r">
        <div className="grid gap-2 border-b p-3">
          <Input value={filter} placeholder="Search tables" onChange={(event) => setFilter(event.target.value)} />
          {canEdit && (
            <Button variant="outline" size="sm" onClick={() => setCreating((value) => !value)}>
              <Plus className="size-4" />
              New table
            </Button>
          )}
        </div>
        {creating && (
          <form className="grid gap-2 border-b p-3" onSubmit={createTable}>
            <Input name="name" placeholder="table_name" required pattern="[A-Za-z][A-Za-z0-9_]*" />
            {columns.map((column, index) => (
              <div key={index} className="flex gap-1">
                <Input
                  value={column.name}
                  placeholder="column"
                  onChange={(event) => setColumns(columns.map((item, i) => (i === index ? { ...item, name: event.target.value } : item)))}
                />
                <select
                  className="h-9 rounded-md border bg-background px-1 text-xs"
                  value={column.type}
                  onChange={(event) => setColumns(columns.map((item, i) => (i === index ? { ...item, type: event.target.value } : item)))}
                >
                  {types.map((type) => (
                    <option key={type}>{type}</option>
                  ))}
                </select>
              </div>
            ))}
            <Button type="button" variant="ghost" size="sm" onClick={() => setColumns([...columns, { name: "", type: "text" }])}>
              Add column
            </Button>
            {error && <p className="text-xs text-destructive">{error}</p>}
            <Button type="submit" size="sm">
              Create
            </Button>
          </form>
        )}
        <div className="min-h-0 flex-1 overflow-auto p-2">
          {shown.map((item) => (
            <button
              key={item.name}
              className={`flex w-full rounded-md px-2 py-1.5 text-left text-sm ${item.name === table ? "bg-muted font-medium" : "hover:bg-muted/60"}`}
              onClick={() => openTable(item.name)}
            >
              {item.name}
            </button>
          ))}
        </div>
      </aside>
      <section className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-10 items-end gap-1 border-b px-2">
          {open.map((name) => (
            <button
              key={name}
              className={`flex items-center gap-1 rounded-t-md border border-b-0 px-3 py-1.5 text-sm ${name === table ? "bg-background" : "bg-muted/50 text-muted-foreground"}`}
              onClick={() => navigate(`/p/${ref}/data/${name}`)}
            >
              {name}
              <X
                className="size-3"
                onClick={(event) => {
                  event.stopPropagation()
                  closeTable(name)
                }}
              />
            </button>
          ))}
        </div>
        {current ? (
          <Grid key={current.name} refId={ref} table={current} canEdit={canEdit} />
        ) : (
          <p className="p-4 text-sm text-muted-foreground">Select a table. This view shows every row, not one user's rows.</p>
        )}
      </section>
    </div>
  )
}

function tabs(ref: string) {
  const raw = sessionStorage.getItem(`reactor.tabs.${ref}`)
  return raw ? (JSON.parse(raw) as string[]) : []
}

const pageSize = 50

function Grid({ refId, table, canEdit }: { refId: string; table: TableInfo; canEdit: boolean }) {
  const [rows, setRows] = useState<Record<string, unknown>[]>([])
  const [total, setTotal] = useState(0)
  const [page, setPage] = useState(0)
  const [q, setQ] = useState("")
  const [draft, setDraft] = useState<Record<string, string> | null>(null)
  const [error, setError] = useState("")
  const [revision, setRevision] = useState(0)
  const pk = table.primary_key

  useEffect(() => {
    const params = new URLSearchParams({ limit: String(pageSize), offset: String(page * pageSize) })
    if (q) params.set("q", q)
    api<{ rows: Record<string, unknown>[]; total: number }>(`/console/v1/projects/${refId}/tables/${table.name}?${params}`)
      .then((body) => {
        setRows(body.rows)
        setTotal(body.total)
        setError("")
      })
      .catch((err) => setError(err.message))
  }, [refId, table.name, q, page, revision])

  function reload() {
    setRevision((value) => value + 1)
  }

  async function saveCell(row: Record<string, unknown>, column: string, value: string) {
    if (!pk || !canEdit) return
    try {
      await api(`/console/v1/projects/${refId}/tables/${table.name}/rows`, {
        method: "POST",
        body: JSON.stringify({ pk: String(row[pk] ?? ""), column, value }),
      })
      reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
    }
  }

  async function insert() {
    if (!draft) return
    try {
      await api(`/console/v1/projects/${refId}/tables/${table.name}`, {
        method: "POST",
        body: JSON.stringify({ values: draft }),
      })
      setDraft(null)
      reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : "insert failed")
    }
  }

  async function remove(row: Record<string, unknown>) {
    if (!pk || !confirm("Delete this row?")) return
    try {
      await api(`/console/v1/projects/${refId}/tables/${table.name}/rows`, {
        method: "DELETE",
        body: JSON.stringify({ pk: String(row[pk] ?? "") }),
      })
      reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : "delete failed")
    }
  }

  const from = total === 0 ? 0 : page * pageSize + 1
  const to = Math.min(total, page * pageSize + rows.length)

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
        <Input
          value={q}
          placeholder="Search rows"
          className="max-w-xs"
          onChange={(event) => {
            setQ(event.target.value)
            setPage(0)
          }}
        />
        {error && <p className="truncate text-sm text-destructive">{error}</p>}
        {canEdit && (
          <Button variant="outline" size="sm" className="ml-auto" onClick={() => setDraft({})}>
            Insert
          </Button>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full text-sm">
          <thead className="sticky top-0 z-10 bg-muted text-left">
            <tr>
              {table.columns.map((column) => (
                <th key={column.name} className="whitespace-nowrap px-3 py-2 font-medium">
                  {column.name}
                  <span className="ml-2 font-normal text-muted-foreground">{column.type}</span>
                </th>
              ))}
              {canEdit && pk && <th className="px-3 py-2" />}
            </tr>
          </thead>
          <tbody>
            {draft && (
              <tr className="border-t">
                {table.columns.map((column) => (
                  <td key={column.name} className="px-2 py-1">
                    {column.name === pk ? (
                      <span className="text-xs text-muted-foreground">auto</span>
                    ) : (
                      <Input
                        value={draft[column.name] || ""}
                        onChange={(event) => setDraft({ ...draft, [column.name]: event.target.value })}
                      />
                    )}
                  </td>
                ))}
                <td className="px-2">
                  <Button size="sm" onClick={insert}>
                    Save
                  </Button>
                </td>
              </tr>
            )}
            {rows.map((row, index) => (
              <tr key={String(pk ? row[pk] : index)} className="border-t">
                {table.columns.map((column) => (
                  <td key={column.name} className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">
                    {canEdit && column.name !== pk ? (
                      <input
                        className="w-full bg-transparent outline-none"
                        defaultValue={String(row[column.name] ?? "")}
                        onBlur={(event) => {
                          if (event.target.value !== String(row[column.name] ?? "")) saveCell(row, column.name, event.target.value)
                        }}
                      />
                    ) : (
                      String(row[column.name] ?? "")
                    )}
                  </td>
                ))}
                {canEdit && pk && (
                  <td className="px-2">
                    <button className="text-xs text-muted-foreground hover:text-foreground" onClick={() => remove(row)}>
                      Delete
                    </button>
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex h-10 shrink-0 items-center justify-between border-t px-3 text-xs text-muted-foreground">
        <span>
          {from}–{to} of {total}
        </span>
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="sm" disabled={page === 0} onClick={() => setPage((value) => value - 1)}>
            <ChevronLeft className="size-4" />
          </Button>
          <Button variant="ghost" size="sm" disabled={to >= total} onClick={() => setPage((value) => value + 1)}>
            <ChevronRight className="size-4" />
          </Button>
        </div>
      </div>
    </div>
  )
}
