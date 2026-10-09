import { useEffect, useRef, useState } from "react"
import { useOutletContext, useParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { api } from "@/lib/api"
import type { ConsoleContext } from "@/shell"
import { ChevronRight, Columns3, File, Folder, ListTree, Plus, Settings, X } from "lucide-react"

type ObjectRow = { key: string; size: number }
type BucketRow = { id: string; public: boolean; public_url_base?: string | null }
type FileItem = { name: string; path: string; row: ObjectRow }
type FolderNode = { name: string; path: string; folders: FolderNode[]; files: FileItem[] }
type View = "columns" | "tree"
type BucketDraft = { name: string; isPublic: boolean; lockedName: boolean }

export function Storage() {
  const { ref = "" } = useParams()
  const { role } = useOutletContext<ConsoleContext>()
  const canDelete = role === "owner" || role === "admin"
  const [rows, setRows] = useState<ObjectRow[]>([])
  const [buckets, setBuckets] = useState<BucketRow[]>([])
  const [bucketModal, setBucketModal] = useState<BucketDraft | null>(null)
  const [savingBucket, setSavingBucket] = useState(false)
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
  function loadBuckets() {
    api<BucketRow[]>(`/console/v1/projects/${ref}/buckets`)
      .then(setBuckets)
      .catch((err) => setError(err.message))
  }
  useEffect(load, [ref])
  useEffect(loadBuckets, [ref])

  const root = filterTree(withBuckets(buildTree(rows, `${ref}/`), buckets), q.trim().toLowerCase())
  const bucketNames = new Set(buckets.map((bucket) => bucket.id))
  const publicNames = new Set(buckets.filter((bucket) => bucket.public).map((bucket) => bucket.id))

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

  function openCreate() {
    setBucketModal({ name: "", isPublic: false, lockedName: false })
    setError("")
  }

  function openSettings(name: string) {
    const bucket = buckets.find((item) => item.id === name)
    if (!bucket) return
    setBucketModal({ name: bucket.id, isPublic: bucket.public, lockedName: true })
    setError("")
  }

  async function saveBucket() {
    if (!bucketModal) return
    const name = bucketModal.name.trim()
    if (!name) return
    setSavingBucket(true)
    setError("")
    try {
      if (bucketModal.lockedName) {
        await api(`/console/v1/projects/${ref}/buckets/${encodeURIComponent(name)}`, {
          method: "PATCH",
          body: JSON.stringify({ public: bucketModal.isPublic }),
        })
      } else {
        await api(`/console/v1/projects/${ref}/buckets`, {
          method: "POST",
          body: JSON.stringify({ name, public: bucketModal.isPublic }),
        })
      }
      setBucketModal(null)
      loadBuckets()
    } catch (err) {
      setError(err instanceof Error ? err.message : "save failed")
    } finally {
      setSavingBucket(false)
    }
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
            const bucket = index === 0 && bucketNames.has(part)
            return (
              <span key={next.join("/")} className="flex min-w-0 items-center">
                <span className="px-1 text-muted-foreground">/</span>
                <button type="button" className="truncate hover:underline" onClick={() => go(next)}>
                  {part}
                </button>
                {bucket && canDelete && (
                  <button
                    type="button"
                    aria-label={`Settings for ${part}`}
                    className="ml-0.5 flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
                    onClick={() => openSettings(part)}
                  >
                    <Settings className="size-3.5" />
                  </button>
                )}
              </span>
            )
          })}
        </nav>
        <Input value={q} placeholder="Search files" className="ml-auto w-56 shrink-0" onChange={(event) => setQ(event.target.value)} />
        {error && !bucketModal && <p className="max-w-40 truncate text-sm text-destructive">{error}</p>}
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
              isRoot={index === 0}
              canCreate={canDelete}
              bucketNames={bucketNames}
              publicNames={publicNames}
              activeFolder={path[index]}
              selectedKey={selected?.key}
              onCreate={openCreate}
              onSettings={openSettings}
              onFolder={(name) => chooseFolder(index, name)}
              onFile={(row) => chooseFile(index, row)}
            />
          ))}
          <div className="w-72 shrink-0 border-l">
            <Details
              file={selected}
              publicUrl={publicUrlFor(ref, buckets, selected?.key)}
              isPublic={fileIsPublic(ref, buckets, selected?.key)}
              canDelete={canDelete}
              onOpen={openFile}
              onDownload={download}
              onDelete={remove}
            />
          </div>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          <div className="min-w-0 flex-1 overflow-auto p-2">
            {root.folders.length === 0 && root.files.length === 0 ? (
              <div className="flex h-full items-center justify-center">
                <Button size="sm" variant="outline" disabled={!canDelete} onClick={openCreate}>
                  <Plus />
                  New bucket
                </Button>
              </div>
            ) : (
              <TreeBranch
                folder={root}
                depth={0}
                open={open}
                canCreate={canDelete}
                bucketNames={bucketNames}
                publicNames={publicNames}
                selectedKey={selected?.key}
                onCreate={openCreate}
                onSettings={openSettings}
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
            <Details
              file={selected}
              publicUrl={publicUrlFor(ref, buckets, selected?.key)}
              isPublic={fileIsPublic(ref, buckets, selected?.key)}
              canDelete={canDelete}
              onOpen={openFile}
              onDownload={download}
              onDelete={remove}
            />
          </aside>
        </div>
      )}
      <div className="flex h-10 shrink-0 items-center border-t px-3 text-xs text-muted-foreground">{rows.length} files</div>
      {bucketModal && (
        <BucketModal
          draft={bucketModal}
          saving={savingBucket}
          error={error}
          onChange={setBucketModal}
          onClose={() => setBucketModal(null)}
          onSave={saveBucket}
        />
      )}
    </div>
  )
}

function Column({
  folder,
  isRoot,
  canCreate,
  bucketNames,
  publicNames,
  activeFolder,
  selectedKey,
  onCreate,
  onSettings,
  onFolder,
  onFile,
}: {
  folder: FolderNode
  isRoot: boolean
  canCreate: boolean
  bucketNames: Set<string>
  publicNames: Set<string>
  activeFolder?: string
  selectedKey?: string
  onCreate: () => void
  onSettings: (name: string) => void
  onFolder: (name: string) => void
  onFile: (row: ObjectRow) => void
}) {
  const empty = folder.folders.length === 0 && folder.files.length === 0
  return (
    <div className={`h-full w-56 shrink-0 overflow-auto border-r ${empty && isRoot ? "flex flex-col" : ""}`}>
      {empty && isRoot && (
        <div className="flex flex-1 items-center justify-center p-3">
          <Button size="sm" variant="outline" disabled={!canCreate} onClick={onCreate}>
            <Plus />
            New bucket
          </Button>
        </div>
      )}
      {empty && !isRoot && <p className="p-3 text-sm text-muted-foreground">Empty</p>}
      {!empty && isRoot && (
        <button
          type="button"
          className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-sm text-muted-foreground hover:bg-muted/60 hover:text-foreground disabled:opacity-50"
          disabled={!canCreate}
          onClick={onCreate}
        >
          <Plus className="size-4 shrink-0" />
          <span>New bucket</span>
        </button>
      )}
      {folder.folders.map((child) => (
        <div
          key={child.path}
          className={`flex w-full items-center ${activeFolder === child.name ? "bg-muted font-medium" : "hover:bg-muted/60"}`}
        >
          <button type="button" className="flex min-w-0 flex-1 items-center gap-2 px-2 py-1.5 text-left text-sm" onClick={() => onFolder(child.name)}>
            <FolderMark isPublic={isRoot && publicNames.has(child.name)} />
            <span className="min-w-0 flex-1 truncate">{child.name}</span>
          </button>
          {isRoot && bucketNames.has(child.name) && canCreate && (
            <button
              type="button"
              aria-label={`Settings for ${child.name}`}
              className="flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-background hover:text-foreground"
              onClick={() => onSettings(child.name)}
            >
              <Settings className="size-3.5" />
            </button>
          )}
          <ChevronRight className="mr-2 size-3.5 shrink-0 text-muted-foreground" />
        </div>
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
  canCreate,
  bucketNames,
  publicNames,
  selectedKey,
  onCreate,
  onSettings,
  onToggle,
  onFile,
}: {
  folder: FolderNode
  depth: number
  open: Set<string>
  canCreate: boolean
  bucketNames: Set<string>
  publicNames: Set<string>
  selectedKey?: string
  onCreate: () => void
  onSettings: (name: string) => void
  onToggle: (path: string) => void
  onFile: (row: ObjectRow) => void
}) {
  return (
    <>
      {depth === 0 && (
        <button
          type="button"
          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-muted-foreground hover:bg-muted/60 hover:text-foreground disabled:opacity-50"
          disabled={!canCreate}
          onClick={onCreate}
        >
          <Plus className="size-4 shrink-0" />
          <span>New bucket</span>
        </button>
      )}
      {folder.folders.map((child) => {
        const expanded = open.has(child.path)
        const settings = depth === 0 && bucketNames.has(child.name) && canCreate
        return (
          <div key={child.path}>
            <div className="flex w-full items-center rounded-md hover:bg-muted/60" style={{ paddingLeft: 8 + depth * 16 }}>
              <button type="button" className="flex min-w-0 flex-1 items-center gap-1 py-1.5 text-left text-sm" onClick={() => onToggle(child.path)}>
                <ChevronRight className={`size-3.5 shrink-0 text-muted-foreground transition-transform ${expanded ? "rotate-90" : ""}`} />
                <FolderMark isPublic={depth === 0 && publicNames.has(child.name)} />
                <span className="truncate">{child.name}</span>
              </button>
              {settings && (
                <button
                  type="button"
                  aria-label={`Settings for ${child.name}`}
                  className="mr-1 flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-background hover:text-foreground"
                  onClick={() => onSettings(child.name)}
                >
                  <Settings className="size-3.5" />
                </button>
              )}
            </div>
            {expanded && (
              <TreeBranch
                folder={child}
                depth={depth + 1}
                open={open}
                canCreate={canCreate}
                bucketNames={bucketNames}
                publicNames={publicNames}
                selectedKey={selectedKey}
                onCreate={onCreate}
                onSettings={onSettings}
                onToggle={onToggle}
                onFile={onFile}
              />
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
  publicUrl,
  isPublic,
  canDelete,
  onOpen,
  onDownload,
  onDelete,
}: {
  file: ObjectRow | null
  publicUrl: string
  isPublic: boolean
  canDelete: boolean
  onOpen: () => void
  onDownload: () => void
  onDelete: () => void
}) {
  if (!file) return <p className="p-4 text-sm text-muted-foreground">Select a file.</p>
  const name = file.key.split("/").pop() || file.key
  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <span className="min-w-0 truncate text-sm font-medium">{name}</span>
        {isPublic && <PublicPill />}
      </div>
      <dl className="grid gap-3 p-3 text-sm">
        <div>
          <dt className="text-xs text-muted-foreground">Path</dt>
          <dd className="mt-1 break-all font-mono text-xs">{file.key}</dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Size</dt>
          <dd className="mt-1 font-mono text-xs">{sizeLabel(file.size)}</dd>
        </div>
        {publicUrl && (
          <div>
            <dt className="text-xs text-muted-foreground">Public URL</dt>
            <dd className="mt-1 break-all font-mono text-xs">{publicUrl}</dd>
            <Button size="sm" variant="outline" className="mt-2" onClick={() => navigator.clipboard.writeText(publicUrl)}>
              Copy
            </Button>
          </div>
        )}
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

function withBuckets(tree: FolderNode, buckets: BucketRow[]): FolderNode {
  const folders = [...tree.folders]
  for (const bucket of buckets) {
    if (!folders.some((folder) => folder.name === bucket.id)) {
      folders.push({ name: bucket.id, path: bucket.id, folders: [], files: [] })
    }
  }
  folders.sort((a, b) => a.name.localeCompare(b.name))
  return { ...tree, folders }
}

function BucketModal({
  draft,
  saving,
  error,
  onChange,
  onClose,
  onSave,
}: {
  draft: BucketDraft
  saving: boolean
  error: string
  onChange: (draft: BucketDraft) => void
  onClose: () => void
  onSave: () => void
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={onClose}>
      <div
        role="dialog"
        aria-label={draft.lockedName ? "Bucket settings" : "Create file bucket"}
        className="w-full max-w-lg rounded-lg border bg-background shadow-lg"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex h-12 items-center justify-between border-b px-4">
          <span className="text-sm font-medium">{draft.lockedName ? "Bucket settings" : "Create file bucket"}</span>
          <button type="button" aria-label="Close" className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted" onClick={onClose}>
            <X className="size-4" />
          </button>
        </div>
        <div className="grid gap-4 p-4">
          <label className="grid gap-2">
            <span className="flex items-baseline justify-between gap-3 text-sm">
              Bucket name
              <span className="text-xs text-muted-foreground">Cannot be changed after creation</span>
            </span>
            <Input
              autoFocus={!draft.lockedName}
              value={draft.name}
              placeholder="Enter bucket name"
              disabled={draft.lockedName || saving}
              onChange={(event) => onChange({ ...draft, name: event.target.value })}
              onKeyDown={(event) => {
                if (event.key === "Enter") onSave()
              }}
            />
          </label>
          <div className="flex items-start gap-3 border-t pt-4">
            <button
              type="button"
              role="switch"
              aria-checked={draft.isPublic}
              aria-label="Public bucket"
              disabled={saving}
              className={`mt-0.5 flex h-5 w-9 shrink-0 items-center rounded-full p-0.5 transition-colors ${draft.isPublic ? "bg-primary" : "bg-muted"}`}
              onClick={() => onChange({ ...draft, isPublic: !draft.isPublic })}
            >
              <span className={`size-4 rounded-full bg-background shadow-sm transition-transform ${draft.isPublic ? "translate-x-4" : ""}`} />
            </button>
            <div className="grid gap-1">
              <span className="text-sm">Public bucket</span>
              <p className="text-sm text-muted-foreground">
                {draft.isPublic
                  ? "Anyone can read objects at a stable URL, with no token and no expiry. Writes still need a storage policy."
                  : "Only callers allowed by a storage policy can read or write. Turn this on to publish a stable URL."}
              </p>
            </div>
          </div>
          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>
        <div className="flex justify-end gap-2 border-t px-4 py-3">
          <Button size="sm" variant="outline" onClick={onClose} disabled={saving}>
            Cancel
          </Button>
          <Button size="sm" onClick={onSave} disabled={saving || !draft.name.trim()}>
            {draft.lockedName ? "Save" : "Create"}
          </Button>
        </div>
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

function PublicPill() {
  return <span className="shrink-0 rounded-full bg-primary/15 px-1.5 py-0.5 text-[10px] font-medium leading-none text-primary">public</span>
}

function FolderMark({ isPublic }: { isPublic: boolean }) {
  return (
    <span className={`inline-flex shrink-0 items-center ${isPublic ? "gap-1 rounded-full bg-primary/15 py-0.5 pr-1.5 pl-1 text-primary" : ""}`}>
      <Folder className={`size-4 ${isPublic ? "text-primary" : "text-muted-foreground"}`} />
      {isPublic && <span className="text-[10px] font-medium leading-none">public</span>}
    </span>
  )
}

function fileIsPublic(ref: string, buckets: BucketRow[], key?: string) {
  if (!key || !key.startsWith(`${ref}/`)) return false
  const rest = key.slice(ref.length + 1)
  const slash = rest.indexOf("/")
  if (slash <= 0) return false
  return buckets.some((item) => item.id === rest.slice(0, slash) && item.public)
}

function publicUrlFor(ref: string, buckets: BucketRow[], key?: string) {
  if (!key || !key.startsWith(`${ref}/`)) return ""
  const rest = key.slice(ref.length + 1)
  const slash = rest.indexOf("/")
  if (slash <= 0) return ""
  const bucket = buckets.find((item) => item.id === rest.slice(0, slash))
  if (!bucket?.public || !bucket.public_url_base) return ""
  return `${bucket.public_url_base}/${rest.slice(slash + 1)}`
}

function sizeLabel(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
