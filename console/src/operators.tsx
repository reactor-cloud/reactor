import { useEffect, useState, type FormEvent } from "react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api } from "@/lib/api"

type CatalogProject = { ref: string; name: string }
type Access = { ref: string; name: string; role: string }
type ConsoleUser = {
  id: string
  email: string
  name: string
  platform_admin: boolean
  projects: Access[]
}

const roles = ["owner", "admin", "developer"]

export function Operators() {
  const [projects, setProjects] = useState<CatalogProject[]>([])
  const [users, setUsers] = useState<ConsoleUser[]>([])
  const [selfId, setSelfId] = useState("")
  const [selected, setSelected] = useState("")
  const [creating, setCreating] = useState(false)
  const [projectQuery, setProjectQuery] = useState("")
  const [error, setError] = useState("")

  function loadUsers(selectId?: string) {
    return api<ConsoleUser[]>("/console/v1/operators").then((rows) => {
      setUsers(rows)
      setSelected((current) => selectId || (rows.some((user) => user.id === current) ? current : rows[0]?.id || ""))
    })
  }

  useEffect(() => {
    api<{ projects: CatalogProject[] }>("/console/v1/cluster")
      .then((cluster) => setProjects(cluster.projects || []))
      .catch((err) => setError(err.message))
    api<{ operator: { id: string } }>("/console/v1/me").then((me) => setSelfId(me.operator.id))
    loadUsers().catch((err) => setError(err.message))
  }, [])

  async function createUser(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const data = new FormData(form)
    try {
      const created = await api<ConsoleUser>("/console/v1/operators", {
        method: "POST",
        body: JSON.stringify({
          name: data.get("name"),
          email: data.get("email"),
          password: data.get("password"),
          platform_admin: data.get("platform_admin") === "on",
        }),
      })
      form.reset()
      setCreating(false)
      setError("")
      await loadUsers(created.id)
    } catch (err) {
      setError(err instanceof Error ? err.message : "create failed")
    }
  }

  async function setAdmin(user: ConsoleUser, platform_admin: boolean) {
    try {
      await api(`/console/v1/operators/${user.id}`, {
        method: "POST",
        body: JSON.stringify({ platform_admin }),
      })
      setError("")
      await loadUsers(user.id)
    } catch (err) {
      setError(err instanceof Error ? err.message : "update failed")
    }
  }

  async function setRole(user: ConsoleUser, pref: string, role: string) {
    try {
      if (role) {
        await api(`/console/v1/operators/${user.id}/projects/${pref}`, {
          method: "POST",
          body: JSON.stringify({ role }),
        })
      } else {
        await api(`/console/v1/operators/${user.id}/projects/${pref}`, { method: "DELETE" })
      }
      setError("")
      await loadUsers(user.id)
    } catch (err) {
      setError(err instanceof Error ? err.message : "access update failed")
    }
  }

  async function remove(user: ConsoleUser) {
    if (!confirm(`Delete ${user.email}? They lose access to every project.`)) return
    try {
      await api(`/console/v1/operators/${user.id}`, { method: "DELETE" })
      setError("")
      await loadUsers()
    } catch (err) {
      setError(err instanceof Error ? err.message : "delete failed")
    }
  }

  const current = users.find((user) => user.id === selected)
  const shown = projects.filter((project) => `${project.name} ${project.ref}`.toLowerCase().includes(projectQuery.trim().toLowerCase()))

  return (
    <div className="mx-auto grid max-w-5xl gap-4">
      <div className="flex items-end justify-between gap-3">
        <div>
          <h1 className="text-2xl font-medium tracking-tight">Console users</h1>
          <p className="text-sm text-muted-foreground">People who can open this console. Cluster admins can manage the cluster and every project.</p>
        </div>
        <Button variant="outline" size="sm" onClick={() => setCreating((value) => !value)}>
          New user
        </Button>
      </div>
      {error && <p className="text-sm text-destructive">{error}</p>}
      <div className="grid min-h-[32rem] overflow-hidden rounded-lg border md:grid-cols-[16rem_1fr]">
        <div className="border-b md:border-r md:border-b-0">
          {users.map((user) => (
            <button
              key={user.id}
              className={`flex w-full flex-col items-start px-3 py-2 text-left text-sm ${user.id === selected && !creating ? "bg-muted" : "hover:bg-muted/60"}`}
              onClick={() => {
                setCreating(false)
                setSelected(user.id)
              }}
            >
              <span className="font-medium">{user.name || user.email}</span>
              <span className="truncate text-xs text-muted-foreground">{user.email}</span>
            </button>
          ))}
          {users.length === 0 && <p className="p-3 text-sm text-muted-foreground">No console users.</p>}
        </div>
        <div className="min-w-0 p-4">
          {creating ? (
            <form className="grid max-w-sm gap-3" onSubmit={createUser}>
              <div className="grid gap-1">
                <Label htmlFor="op-name">Name</Label>
                <Input id="op-name" name="name" required />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="op-email">Email</Label>
                <Input id="op-email" name="email" type="email" required />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="op-password">Password</Label>
                <Input id="op-password" name="password" type="password" minLength={8} required />
              </div>
              <label className="flex items-center gap-2 text-sm">
                <input name="platform_admin" type="checkbox" />
                Cluster admin
              </label>
              <Button type="submit" className="w-fit">
                Create
              </Button>
            </form>
          ) : current ? (
            <div className="grid gap-4">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <div className="font-medium">{current.name || current.email}</div>
                  <div className="text-sm text-muted-foreground">{current.email}</div>
                </div>
                <Button variant="outline" size="sm" disabled={current.id === selfId} onClick={() => remove(current)}>
                  Delete
                </Button>
              </div>
              <label className="flex items-center gap-2 text-sm">
                <input type="checkbox" checked={current.platform_admin} onChange={(event) => setAdmin(current, event.target.checked)} />
                Cluster admin
              </label>
              <div className="grid gap-2">
                <div className="text-xs font-medium text-muted-foreground">Project access</div>
                <Input value={projectQuery} placeholder="Find a project" onChange={(event) => setProjectQuery(event.target.value)} />
                <div className="grid max-h-80 gap-2 overflow-auto">
                  {shown.map((project) => {
                    const role = current.projects.find((item) => item.ref === project.ref)?.role || ""
                    return (
                      <div key={project.ref} className="flex items-center justify-between gap-3">
                        <span className="truncate text-sm">{project.name || project.ref}</span>
                        <select
                          className="h-8 shrink-0 rounded-md border bg-background px-2 text-sm"
                          value={role}
                          onChange={(event) => setRole(current, project.ref, event.target.value)}
                        >
                          <option value="">No access</option>
                          {roles.map((item) => (
                            <option key={item} value={item}>
                              {item}
                            </option>
                          ))}
                        </select>
                      </div>
                    )
                  })}
                  {shown.length === 0 && <p className="text-sm text-muted-foreground">No matching projects.</p>}
                </div>
              </div>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">Select a console user.</p>
          )}
        </div>
      </div>
    </div>
  )
}
