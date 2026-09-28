import { useEffect, useRef, useState } from "react"
import { useOutletContext, useParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { api } from "@/lib/api"
import type { ConsoleContext } from "@/shell"
import { ChevronRight, Columns3, File, Folder, ListTree } from "lucide-react"

type ObjectRow = { key: string; size: number }
type FileItem = { name: string; path: string; row: ObjectRow }
type FolderNode = { name: string; path: string; folders: FolderNode[]; files: FileItem[] }
type View = "columns" | "tree"

export function Storage() {
  const { ref = "" } = useParams()
  const { role } = useOutletContext<ConsoleContext>()
  const canDelete = role === "owner" || role === "admin"
  const [rows, setRows] = useState<ObjectRow[]>([])
  const [q, setQ] = useState("")
  const [view, setView] = useState<View>("columns")
  const [path, setPath] = useState<string[]>([])
  const [selected, setSelected] = useState<ObjectRow | null>(null)
  const [open, setOpen] = useState<Set<string>>(() => new Set([""]))
  const [error, setError] = useState("")
  const fileRef = useRef<HTMLInputElement>(null)

  function load() {
    api<ObjectRow[]>(`/console/v1/projects/${ref}/objects`)
      .then((next) => {
        setRows(next)
        setError("")
        setSelected((current) => next.find((row) => row.key === current?.key) || null)
      })
      .catch((err) => setError(err.message))
  }
  useEffect(load, [ref])

  const root = filterTree(buildTree(rows, `${ref}/`), q.trim().toLowerCase())

  function chooseFolder(column: number, name: string) {
    setPath((current) => [...current.slice(0, column), name])
    setSelected(null)
  }

  function chooseFile(column: number, row: ObjectRow) {
    setPath((current) => current.slice(0, column))
    setSelected(row)
  }

  async function signed(row: ObjectRow) {
    const body = await api<{ url: string }>(`/console/v1/projects/${ref}/objects/url`, {
      method: "POST",
      body: JSON.stringify({ key: row.key }),
    })
    return body.url
  }

  async function openFile() {
    if (!selected) return
    window.open(await signed(selected), "_blank")
  }

  async function download() {
    if (!selected) return
    const url = await signed(selected)
    const response = await fetch(url)
    const blob = await response.blob()
    const href = URL.createObjectURL(blob)
    const link = document.createElement("a")
    link.href = href
    link.download = selected.key.split("/").pop() || "file"
    link.click()
    URL.revokeObjectURL(href)
  }

  function go(next: string[]) {
    setPath(next)
    setSelected(null)
  }

  async function upload(list: FileList | null) {
    if (!list || list.length === 0) return
    const folder = path.join("/")
    setError("")
    try {
      for (const file of list) {
        const rel = folder ? `${folder}/${file.name}` : file.name
        const body = await api<{ url: string; key: string }>(`/console/v1/projects/${ref}/objects/upload`, {
          method: "POST",
          body: JSON.stringify({ path: rel }),
        })
        const put = await fetch(body.url, {
          method: "PUT",
          body: file,
          headers: { "content-type": file.type || "application/octet-stream" },
        })
        if (!put.ok) throw new Error(`upload failed (${put.status})`)
      }
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "upload failed")
    }
  }

  async function remove() {
    if (!selected || !confirm(`Delete ${selected.key}?`)) return
    try {
      await api(`/console/v1/projects/${ref}/objects`, { method: "DELETE", body: JSON.stringify({ key: selected.key }) })
      setSelected(null)
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "delete failed")
    }
  }

  const columns = [root]
  let cursor = root
  for (const name of path) {
    const next = cursor.folders.find((folder) => folder.name === name)
    if (!next) break
    columns.push(next)
    cursor = next
  }

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-3 border-b px-3">
        <h1 className="shrink-0 text-base font-semibold">Storage</h1>
        <nav className="flex min-w-0 items-center overflow-hidden text-sm">
          <button type="button" className="shrink-0 truncate hover:underline" onClick={() => go([])}>
            {ref}
          </button>
          {path.map((part, index) => {
            const next = path.slice(0, index + 1)
            return (
              <span key={next.join("/")} className="flex min-w-0 items-center">
                <span className="px-1 text-muted-foreground">/</span>
                <button type="button" className="truncate hover:underline" onClick={() => go(next)}>
                  {part}
                </button>
              </span>
            )
          })}
        </nav>
        <Input value={q} placeholder="Search files" className="ml-auto w-56 shrink-0" onChange={(event) => setQ(event.target.value)} />
        {error && <p className="max-w-40 truncate text-sm text-destructive">{error}</p>}
        <Button size="sm" variant="outline" disabled={!canDelete} onClick={() => fileRef.current?.click()}>
          Upload
        </Button>
        <input
          ref={fileRef}
          type="file"
          multiple
          className="hidden"
          onChange={(event) => {
            upload(event.target.files)
            event.target.value = ""
          }}
        />
        <div className="flex shrink-0 overflow-hidden rounded-md border">
          <button
            type="button"
            aria-label="Columns"
            aria-pressed={view === "columns"}
            className={`flex size-8 items-center justify-center ${view === "columns" ? "bg-muted text-foreground" : "text-muted-foreground hover:bg-muted/60"}`}
            onClick={() => setView("columns")}
          >
            <Columns3 className="size-4" />
          </button>
          <button
            type="button"
            aria-label="Tree"
            aria-pressed={view === "tree"}
            className={`flex size-8 items-center justify-center border-l ${view === "tree" ? "bg-muted text-foreground" : "text-muted-foreground hover:bg-muted/60"}`}
            onClick={() => setView("tree")}
          >
            <ListTree className="size-4" />
          </button>
        </div>
      </div>
      {view === "columns" ? (
        <div className="flex min-h-0 flex-1 overflow-x-auto">
          {columns.map((folder, index) => (
            <Column
              key={folder.path || "root"}
              folder={folder}
              activeFolder={path[index]}
              selectedKey={selected?.key}
              onFolder={(name) => chooseFolder(index, name)}
              onFile={(row) => chooseFile(index, row)}
            />
          ))}
          <div className="w-72 shrink-0 border-l">
            <Details file={selected} canDelete={canDelete} onOpen={openFile} onDownload={download} onDelete={remove} />
          </div>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          <div className="min-w-0 flex-1 overflow-auto p-2">
            {root.folders.length === 0 && root.files.length === 0 ? (
              <p className="p-2 text-sm text-muted-foreground">Nothing here yet.</p>
            ) : (
              <TreeBranch
                folder={root}
                depth={0}
                open={open}
                selectedKey={selected?.key}
                onToggle={(folderPath) => {
                  setPath(folderPath.split("/").filter(Boolean))
                  setSelected(null)
                  setOpen((current) => {
                    const next = new Set(current)
                    if (next.has(folderPath)) next.delete(folderPath)
                    else next.add(folderPath)
                    return next
                  })
                }}
                onFile={(row) => {
                  const rel = row.key.startsWith(`${ref}/`) ? row.key.slice(ref.length + 1) : row.key
                  setPath(rel.split("/").filter(Boolean).slice(0, -1))
                  setSelected(row)
                }}
              />
            )}
          </div>
          <aside className="w-72 shrink-0 border-l">
            <Details file={selected} canDelete={canDelete} onOpen={openFile} onDownload={download} onDelete={remove} />
          </aside>
        </div>
      )}
      <div className="flex h-10 shrink-0 items-center border-t px-3 text-xs text-muted-foreground">{rows.length} files</div>
    </div>
  )
}

function Column({
  folder,
  activeFolder,
  selectedKey,
  onFolder,
  onFile,
}: {
  folder: FolderNode
  activeFolder?: string
  selectedKey?: string
  onFolder: (name: string) => void
  onFile: (row: ObjectRow) => void
}) {
  const empty = folder.folders.length === 0 && folder.files.length === 0
  return (
    <div className="h-full w-56 shrink-0 overflow-auto border-r">
      {empty && <p className="p-3 text-sm text-muted-foreground">Empty</p>}
      {folder.folders.map((child) => (
        <button
          key={child.path}
          className={`flex w-full items-center gap-2 px-2 py-1.5 text-left text-sm ${activeFolder === child.name ? "bg-muted font-medium" : "hover:bg-muted/60"}`}
          onClick={() => onFolder(child.name)}
        >
          <Folder className="size-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate">{child.name}</span>
          <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" />
        </button>
      ))}
      {folder.files.map((file) => (
        <button
          key={file.row.key}
          className={`flex w-full items-center gap-2 px-2 py-1.5 text-left text-sm ${selectedKey === file.row.key ? "bg-muted font-medium" : "hover:bg-muted/60"}`}
          onClick={() => onFile(file.row)}
        >
          <File className="size-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate">{file.name}</span>
        </button>
      ))}
    </div>
  )
}

function TreeBranch({
  folder,
  depth,
  open,
  selectedKey,
  onToggle,
  onFile,
}: {
  folder: FolderNode
  depth: number
  open: Set<string>
  selectedKey?: string
  onToggle: (path: string) => void
  onFile: (row: ObjectRow) => void
}) {
  return (
    <>
      {folder.folders.map((child) => {
        const expanded = open.has(child.path)
        return (
          <div key={child.path}>
            <button
              className="flex w-full items-center gap-1 rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted/60"
              style={{ paddingLeft: 8 + depth * 16 }}
              onClick={() => onToggle(child.path)}
            >
              <ChevronRight className={`size-3.5 shrink-0 text-muted-foreground transition-transform ${expanded ? "rotate-90" : ""}`} />
              <Folder className="size-4 shrink-0 text-muted-foreground" />
              <span className="truncate">{child.name}</span>
            </button>
            {expanded && (
              <TreeBranch folder={child} depth={depth + 1} open={open} selectedKey={selectedKey} onToggle={onToggle} onFile={onFile} />
            )}
          </div>
        )
      })}
      {folder.files.map((file) => (
        <button
          key={file.row.key}
          className={`flex w-full items-center gap-2 rounded-md py-1.5 pr-2 text-left text-sm ${selectedKey === file.row.key ? "bg-muted font-medium" : "hover:bg-muted/60"}`}
          style={{ paddingLeft: 8 + depth * 16 + 18 }}
          onClick={() => onFile(file.row)}
        >
          <File className="size-4 shrink-0 text-muted-foreground" />
          <span className="truncate">{file.name}</span>
        </button>
      ))}
    </>
  )
}

function Details({
  file,
  canDelete,
  onOpen,
  onDownload,
  onDelete,
}: {
  file: ObjectRow | null
  canDelete: boolean
  onOpen: () => void
  onDownload: () => void
  onDelete: () => void
}) {
  if (!file) return <p className="p-4 text-sm text-muted-foreground">Select a file.</p>
  const name = file.key.split("/").pop() || file.key
  return (
    <div className="flex h-full flex-col">
      <div className="truncate border-b px-3 py-2 text-sm font-medium">{name}</div>
      <dl className="grid gap-3 p-3 text-sm">
        <div>
          <dt className="text-xs text-muted-foreground">Path</dt>
          <dd className="mt-1 break-all font-mono text-xs">{file.key}</dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Size</dt>
          <dd className="mt-1 font-mono text-xs">{sizeLabel(file.size)}</dd>
        </div>
      </dl>
      <div className="mt-auto flex flex-wrap gap-2 p-3">
        <Button size="sm" variant="outline" onClick={onOpen}>
          Open
        </Button>
        <Button size="sm" variant="outline" onClick={onDownload}>
          Download
        </Button>
        {canDelete && (
          <Button size="sm" variant="outline" onClick={onDelete}>
            Delete
          </Button>
        )}
      </div>
    </div>
  )
}

function buildTree(rows: ObjectRow[], prefix: string): FolderNode {
  const root: FolderNode = { name: "", path: "", folders: [], files: [] }
  for (const row of rows) {
    const rel = row.key.startsWith(prefix) ? row.key.slice(prefix.length) : row.key
    const parts = rel.split("/").filter(Boolean)
    let cursor = root
    parts.forEach((name, index) => {
      const path = parts.slice(0, index + 1).join("/")
      if (index === parts.length - 1) {
        cursor.files.push({ name, path, row })
        return
      }
      let next = cursor.folders.find((folder) => folder.name === name)
      if (!next) {
        next = { name, path, folders: [], files: [] }
        cursor.folders.push(next)
      }
      cursor = next
    })
  }
  sortFolder(root)
  return root
}

function sortFolder(folder: FolderNode) {
  folder.folders.sort((a, b) => a.name.localeCompare(b.name))
  folder.files.sort((a, b) => a.name.localeCompare(b.name))
  folder.folders.forEach(sortFolder)
}

function filterTree(folder: FolderNode, q: string): FolderNode {
  if (!q) return folder
  const folders = folder.folders
    .map((child) => filterTree(child, q))
    .filter((child) => child.name.toLowerCase().includes(q) || child.folders.length > 0 || child.files.length > 0)
  const files = folder.files.filter((file) => file.name.toLowerCase().includes(q) || file.path.toLowerCase().includes(q))
  return { ...folder, folders, files }
}

function sizeLabel(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
