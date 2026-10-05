import { useEffect, useState, type FormEvent } from "react"
import { useNavigate, useOutletContext, useParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api, type Project } from "@/lib/api"
import type { ConsoleContext } from "@/shell"

export function Settings() {
  const { ref = "" } = useParams()
  const { role } = useOutletContext<ConsoleContext>()
  const navigate = useNavigate()
  const [name, setName] = useState("")
  const [error, setError] = useState("")
  const [saved, setSaved] = useState(false)
  const [confirm, setConfirm] = useState(false)
  const [typed, setTyped] = useState("")
  const [busy, setBusy] = useState(false)
  const [resetOpen, setResetOpen] = useState(false)
  const [resetDone, setResetDone] = useState(false)
  const owner = role === "owner"
  const canReset = role === "owner" || role === "admin"

  useEffect(() => {
    api<Project[]>("/console/v1/projects").then((projects) => {
      const project = projects.find((item) => item.ref === ref)
      if (project) setName(project.name)
    })
  }, [ref])

  async function rename(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setError("")
    setSaved(false)
    try {
      const body = await api<{ name: string }>(`/console/v1/projects/${ref}`, {
        method: "POST",
        body: JSON.stringify({ name }),
      })
      setName(body.name)
      setSaved(true)
      window.dispatchEvent(new Event("reactor-projects"))
    } catch (err) {
      setError(err instanceof Error ? err.message : "rename failed")
    }
  }

  async function resetRole() {
    setBusy(true)
    setError("")
    setResetDone(false)
    try {
      await api(`/console/v1/projects/${ref}/database/reset`, { method: "POST" })
      setResetOpen(false)
      setResetDone(true)
    } catch (err) {
      setError(err instanceof Error ? err.message : "reset failed")
    } finally {
      setBusy(false)
    }
  }

  async function remove() {
    setBusy(true)
    setError("")
    try {
      await api(`/console/v1/projects/${ref}`, { method: "DELETE" })
      window.dispatchEvent(new Event("reactor-projects"))
      navigate("/")
    } catch (err) {
      setBusy(false)
      setError(err instanceof Error ? err.message : "delete failed")
    }
  }

  return (
    <div className="grid max-w-xl gap-8">
      <div>
        <h1 className="text-2xl font-medium tracking-tight">Settings</h1>
        <p className="font-mono text-sm text-muted-foreground">{ref}</p>
      </div>

      <section className="grid gap-3">
        <h2 className="text-sm font-medium">Name</h2>
        <form className="flex items-end gap-2" onSubmit={rename}>
          <div className="grid min-w-0 flex-1 gap-2">
            <Label htmlFor="project-name">Project name</Label>
            <Input id="project-name" value={name} disabled={!owner} onChange={(event) => setName(event.target.value)} />
          </div>
          <Button type="submit" disabled={!owner || name.trim() === ""}>
            Save
          </Button>
        </form>
        {saved && <p className="text-sm text-muted-foreground">Name saved.</p>}
        {!owner && <p className="text-sm text-muted-foreground">Only the owner can change these.</p>}
      </section>

      <section className="grid gap-3">
        <h2 className="text-sm font-medium">Database role</h2>
        <p className="text-sm text-muted-foreground">
          Resets the project database role password. In-flight SQL and migrations on this project are cancelled.
        </p>
        <div>
          <Button type="button" variant="outline" disabled={!canReset || busy} onClick={() => setResetOpen(true)}>
            Reset database role
          </Button>
        </div>
        {resetDone && <p className="text-sm text-muted-foreground">Database role reset.</p>}
      </section>

      <section className="grid gap-3">
        <h2 className="text-sm font-medium">Later</h2>
        <Later title="Migrate" detail="Move this project to another Reactor cluster." />
        <Later title="Transfer" detail="Give this project to another owner." />
      </section>

      <section className="grid gap-3 rounded-lg border border-destructive/30 p-4">
        <h2 className="text-sm font-medium">Delete project</h2>
        <p className="text-sm text-muted-foreground">
          Removes the database, files, functions, and site. This cannot be undone.
        </p>
        <div>
          <Button variant="destructive" disabled={!owner} onClick={() => setConfirm(true)}>
            Delete project
          </Button>
        </div>
      </section>
      {error && <p className="text-sm text-destructive">{error}</p>}

      {resetOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <div className="grid w-full max-w-md gap-3 rounded-lg border bg-background p-4 shadow-lg">
            <h2 className="text-base font-medium">Reset the database role?</h2>
            <p className="text-sm text-muted-foreground">
              In-flight SQL and migrations on this project are cancelled. The new password stays on the server.
            </p>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="outline" onClick={() => setResetOpen(false)}>
                Cancel
              </Button>
              <Button type="button" disabled={busy} onClick={resetRole}>
                Reset database role
              </Button>
            </div>
          </div>
        </div>
      )}

      {confirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
          <form
            className="grid w-full max-w-md gap-3 rounded-lg border bg-background p-4 shadow-lg"
            onSubmit={(event) => {
              event.preventDefault()
              if (typed === name) remove()
            }}
          >
            <h2 className="text-base font-medium">Delete {name || ref}?</h2>
            <p className="text-sm text-muted-foreground">Type the project name to confirm.</p>
            <Input value={typed} autoFocus onChange={(event) => setTyped(event.target.value)} />
            <div className="flex justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setConfirm(false)
                  setTyped("")
                }}
              >
                Cancel
              </Button>
              <Button type="submit" variant="destructive" disabled={typed !== name || busy}>
                Delete project
              </Button>
            </div>
          </form>
        </div>
      )}
    </div>
  )
}

function Later({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border px-4 py-3">
      <div>
        <div className="text-sm font-medium">{title}</div>
        <p className="text-sm text-muted-foreground">{detail}</p>
      </div>
      <Button type="button" variant="outline" disabled>
        Later
      </Button>
    </div>
  )
}
