import { useEffect, useState, type FormEvent } from "react"
import { useNavigate, useOutletContext, useParams, useSearchParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { api } from "@/lib/api"
import type { ConsoleContext } from "@/shell"
import { ChevronLeft, ChevronRight, Database, Plus, X } from "lucide-react"

type Column = { name: string; type: string }
type TableInfo = { name: string; primary_key: string | null; columns: Column[] }
type SchemaInfo = { name: string; tables: number }

const types = ["text", "uuid", "integer", "bigint", "boolean", "timestamptz", "jsonb", "numeric"]

function ownSchema(ref: string) {
  return `proj_${ref}`
}

function schemaSearch(ref: string, schema: string) {
  return schema && schema !== ownSchema(ref) ? `?schema=${encodeURIComponent(schema)}` : ""
}

export function Data() {
  const { ref = "", table = "" } = useParams()
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const { role } = useOutletContext<ConsoleContext>()
  const canEdit = role === "owner" || role === "admin"
  const [schemas, setSchemas] = useState<SchemaInfo[]>([])
  const [tables, setTables] = useState<TableInfo[]>([])
  const [open, setOpen] = useState<string[]>([])
  const [creating, setCreating] = useState(false)
  const [filter, setFilter] = useState("")
  const [columns, setColumns] = useState<{ name: string; type: string }[]>([{ name: "", type: "text" }])
  const [error, setError] = useState("")
  const [listRevision, setListRevision] = useState(0)
  const requested = params.get("schema") || ""
  const selected = schemas.some((item) => item.name === requested)
    ? requested
    : schemas.find((item) => item.name === ownSchema(ref))?.name || schemas[0]?.name || ""

  useEffect(() => {
    api<SchemaInfo[]>(`/console/v1/projects/${ref}/schemas`)
      .then((body) => {
        setSchemas(body)
        setError("")
      })
      .catch((err) => setError(err instanceof Error ? err.message : "schemas failed"))
  }, [ref])

  function load() {
    setListRevision((value) => value + 1)
  }

  useEffect(() => {
    if (!selected) return
    api<TableInfo[]>(`/console/v1/projects/${ref}/schema?schema=${encodeURIComponent(selected)}`)
      .then((body) => {
        setTables(body)
        setError("")
      })
      .catch((err) => setError(err instanceof Error ? err.message : "tables failed"))
  }, [ref, selected, listRevision])

  useEffect(() => {
    if (!selected) {
      setOpen([])
      return
    }
    const stored = tabs(ref, selected)
    if (!table || stored.includes(table)) {
      setOpen(stored)
      return
    }
    const next = [...stored, table]
    setOpen(next)
    sessionStorage.setItem(`reactor.tabs.${ref}.${selected}`, JSON.stringify(next))
  }, [ref, selected, table])

  function openTable(name: string) {
    const next = open.includes(name) ? open : [...open, name]
    setOpen(next)
    sessionStorage.setItem(`reactor.tabs.${ref}.${selected}`, JSON.stringify(next))
    navigate({ pathname: `/p/${ref}/data/${name}`, search: schemaSearch(ref, selected) })
  }

  function closeTable(name: string) {
    const next = open.filter((item) => item !== name)
    setOpen(next)
    sessionStorage.setItem(`reactor.tabs.${ref}.${selected}`, JSON.stringify(next))
    if (table === name) {
      navigate({
        pathname: next[0] ? `/p/${ref}/data/${next[0]}` : `/p/${ref}/data`,
        search: schemaSearch(ref, selected),
      })
    }
  }

  function chooseSchema(name: string) {
    setOpen([])
    setTables([])
    navigate({ pathname: `/p/${ref}/data`, search: schemaSearch(ref, name) })
  }

  async function createTable(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    const spec = columns.filter((column) => column.name.trim())
    try {
      await api(`/console/v1/projects/${ref}/tables?schema=${encodeURIComponent(selected)}`, {
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
        <div className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
          <Database className="size-4 shrink-0" />
          <h1 className="text-base font-semibold">Data</h1>
        </div>
        <div className="grid gap-2 border-b p-3">
          <label className="grid gap-1 text-xs text-muted-foreground">
            Schema
            <select
              className="h-8 truncate rounded-md border bg-background px-2 text-xs text-foreground"
              value={selected}
              onChange={(event) => chooseSchema(event.target.value)}
            >
              {schemas.map((item) => (
                <option key={item.name} value={item.name}>
                  {item.name}
                </option>
              ))}
            </select>
          </label>
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
              onClick={() => navigate({ pathname: `/p/${ref}/data/${name}`, search: schemaSearch(ref, selected) })}
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
          <Grid key={`${selected}.${current.name}`} refId={ref} schema={selected} table={current} canEdit={canEdit} />
        ) : (
          <p className="p-4 text-sm text-muted-foreground">Select a table. This view shows every row, not one user's rows.</p>
        )}
      </section>
    </div>
  )
}

function tabs(ref: string, schema: string) {
  const raw = sessionStorage.getItem(`reactor.tabs.${ref}.${schema}`)
  return raw ? (JSON.parse(raw) as string[]) : []
}

const pageSize = 50

function Grid({ refId, schema, table, canEdit }: { refId: string; schema: string; table: TableInfo; canEdit: boolean }) {
  const [rows, setRows] = useState<Record<string, unknown>[]>([])
  const [total, setTotal] = useState(0)
  const [page, setPage] = useState(0)
  const [q, setQ] = useState("")
  const [draft, setDraft] = useState<Record<string, string> | null>(null)
  const [error, setError] = useState("")
  const [revision, setRevision] = useState(0)
  const [openRow, setOpenRow] = useState<Record<string, unknown> | null>(null)
  const pk = table.primary_key

  useEffect(() => {
    const params = new URLSearchParams({ limit: String(pageSize), offset: String(page * pageSize), schema })
    if (q) params.set("q", q)
    api<{ rows: Record<string, unknown>[]; total: number }>(`/console/v1/projects/${refId}/tables/${table.name}?${params}`)
      .then((body) => {
        setRows(body.rows)
        setTotal(body.total)
        setError("")
      })
      .catch((err) => setError(err.message))
  }, [refId, schema, table.name, q, page, revision])

  function reload() {
    setRevision((value) => value + 1)
  }

  async function insert() {
    if (!draft) return
    try {
      await api(`/console/v1/projects/${refId}/tables/${table.name}?schema=${encodeURIComponent(schema)}`, {
        method: "POST",
        body: JSON.stringify({ values: draft }),
      })
      setDraft(null)
      reload()
    } catch (err) {
      setError(err instanceof Error ? err.message : "insert failed")
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
            </tr>
          </thead>
          <tbody>
            {draft && (
              <tr className="border-t">
                {table.columns.map((column, index) => (
                  <td key={column.name} className="px-2 py-1">
                    {column.name === pk ? (
                      <span className="text-xs text-muted-foreground">auto</span>
                    ) : (
                      <Input
                        value={draft[column.name] || ""}
                        onChange={(event) => setDraft({ ...draft, [column.name]: event.target.value })}
                      />
                    )}
                    {index === table.columns.length - 1 && (
                      <Button size="sm" className="mt-1" onClick={insert}>
                        Save
                      </Button>
                    )}
                  </td>
                ))}
              </tr>
            )}
            {rows.map((row, index) => (
              <tr
                key={String(pk ? row[pk] : index)}
                role="button"
                tabIndex={0}
                className="cursor-pointer border-t hover:bg-muted/40"
                onClick={() => setOpenRow(row)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") setOpenRow(row)
                }}
              >
                {table.columns.map((column) => (
                  <td key={column.name} className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">
                    {fieldText(row[column.name])}
                  </td>
                ))}
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
      {openRow && (
        <RowForm
          refId={refId}
          schema={schema}
          table={table}
          row={openRow}
          canEdit={canEdit}
          onClose={() => setOpenRow(null)}
          onSaved={() => {
            setOpenRow(null)
            reload()
          }}
        />
      )}
    </div>
  )
}

function fieldText(value: unknown) {
  if (value == null) return ""
  if (typeof value === "object") return JSON.stringify(value)
  return String(value)
}

function RowForm({
  refId,
  schema,
  table,
  row,
  canEdit,
  onClose,
  onSaved,
}: {
  refId: string
  schema: string
  table: TableInfo
  row: Record<string, unknown>
  canEdit: boolean
  onClose: () => void
  onSaved: () => void
}) {
  const pk = table.primary_key
  const [form, setForm] = useState<Record<string, string>>(() => {
    const next: Record<string, string> = {}
    for (const column of table.columns) next[column.name] = fieldText(row[column.name])
    return next
  })
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  const original = table.columns.map((column) => fieldText(row[column.name]))
  const dirty = table.columns.some((column, index) => column.name !== pk && form[column.name] !== original[index])

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [onClose])

  async function save() {
    if (!pk || !canEdit || busy) return
    setBusy(true)
    setError("")
    try {
      for (const column of table.columns) {
        if (column.name === pk || form[column.name] === fieldText(row[column.name])) continue
        await api(`/console/v1/projects/${refId}/tables/${table.name}/rows?schema=${encodeURIComponent(schema)}`, {
          method: "POST",
          body: JSON.stringify({ pk: String(row[pk] ?? ""), column: column.name, value: form[column.name] }),
        })
      }
      onSaved()
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
      setBusy(false)
    }
  }

  async function duplicate() {
    if (!canEdit || busy) return
    setBusy(true)
    setError("")
    const values: Record<string, string> = {}
    for (const column of table.columns) {
      if (column.name === pk) continue
      if (form[column.name]) values[column.name] = form[column.name]
    }
    try {
      await api(`/console/v1/projects/${refId}/tables/${table.name}?schema=${encodeURIComponent(schema)}`, {
        method: "POST",
        body: JSON.stringify({ values }),
      })
      onSaved()
    } catch (err) {
      setError(err instanceof Error ? err.message : "duplicate failed")
      setBusy(false)
    }
  }

  async function remove() {
    if (!pk || !canEdit || busy || !confirm("Delete this row?")) return
    setBusy(true)
    setError("")
    try {
      await api(`/console/v1/projects/${refId}/tables/${table.name}/rows?schema=${encodeURIComponent(schema)}`, {
        method: "DELETE",
        body: JSON.stringify({ pk: String(row[pk] ?? "") }),
      })
      onSaved()
    } catch (err) {
      setError(err instanceof Error ? err.message : "delete failed")
      setBusy(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={onClose}>
      <div
        role="dialog"
        aria-label={`${table.name} row`}
        className="flex max-h-[80vh] w-full max-w-lg flex-col rounded-lg border bg-background shadow-lg"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex h-12 shrink-0 items-center justify-between border-b px-3">
          <span className="truncate text-sm font-medium">{table.name}</span>
          <button type="button" aria-label="Close row" className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted" onClick={onClose}>
            <X className="size-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-auto p-4">
          <div className="grid gap-3">
            {table.columns.map((column) => (
              <label key={column.name} className="grid gap-1">
                <span className="text-xs text-muted-foreground">
                  {column.name}
                  <span className="ml-2">{column.type}</span>
                </span>
                <FieldControl
                  column={column}
                  locked={!canEdit || column.name === pk}
                  value={form[column.name] || ""}
                  onChange={(value) => setForm({ ...form, [column.name]: value })}
                />
              </label>
            ))}
          </div>
          {error && <p className="mt-3 text-sm text-destructive">{error}</p>}
        </div>
        {canEdit && (
          <div className="flex shrink-0 items-center gap-2 border-t px-3 py-2">
            {pk && (
              <Button variant="destructive" size="sm" disabled={busy} onClick={remove}>
                Delete
              </Button>
            )}
            <Button variant="outline" size="sm" className="ml-auto" disabled={busy} onClick={duplicate}>
              Duplicate
            </Button>
            {pk && (
              <Button size="sm" disabled={busy || !dirty} onClick={save}>
                Save
              </Button>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

function FieldControl({
  column,
  locked,
  value,
  onChange,
}: {
  column: Column
  locked: boolean
  value: string
  onChange: (value: string) => void
}) {
  if (locked && column.name) {
    return <p className="break-all font-mono text-xs">{value || "—"}</p>
  }
  if (column.type === "boolean") {
    return (
      <select
        className="h-8 rounded-md border bg-background px-2 text-sm"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        <option value="">—</option>
        <option value="true">true</option>
        <option value="false">false</option>
      </select>
    )
  }
  if (column.type === "text" || column.type === "jsonb" || column.type === "json") {
    return (
      <textarea
        className="min-h-16 w-full rounded-md border bg-background px-2 py-1 font-mono text-xs"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    )
  }
  return <Input value={value} onChange={(event) => onChange(event.target.value)} />
}
