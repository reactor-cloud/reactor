import { useEffect, useState, type FormEvent, type ReactNode } from "react"
import { Link, useNavigate, useOutletContext, useParams } from "react-router-dom"
import { Logo } from "@/components/logo"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { api, saveKeys, savedKeys, setToken, token, type Project } from "@/lib/api"
import type { ConsoleContext } from "@/shell"
import { Copy, Eye, EyeOff } from "lucide-react"
import { Enroll, SecondFactor } from "@/mfa"

type ConsoleLogin = {
  access_token?: string
  setup_token?: string
  mfa_token?: string
  enrollment_required?: boolean
  mfa_required?: boolean
}

function Gate({ children }: { children: ReactNode }) {
  const navigate = useNavigate()
  useEffect(() => {
    if (!token()) navigate("/login")
  }, [navigate])
  return token() ? children : null
}

export function Setup() {
  const navigate = useNavigate()
  const [error, setError] = useState("")
  const [setupToken, setSetupToken] = useState("")

  useEffect(() => {
    api<{ needs_setup: boolean }>("/console/v1/setup").then((status) => {
      if (!status.needs_setup && !setupToken) navigate("/login")
    })
  }, [navigate, setupToken])

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    try {
      const result = await api<ConsoleLogin>("/console/v1/setup", {
        method: "POST",
        body: JSON.stringify({
          cluster_name: data.get("cluster_name"),
          name: data.get("name"),
          email: data.get("email"),
          password: data.get("password"),
        }),
      })
      if (result.setup_token) {
        setSetupToken(result.setup_token)
        return
      }
      if (result.access_token) {
        setToken(result.access_token)
        navigate("/")
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "setup failed")
    }
  }

  return (
    <Center title="Set up this cluster" subtitle="This account administers the cluster. It is not a user inside a project.">
      {setupToken ? (
        <Enroll
          token={setupToken}
          onDone={(access) => {
            setToken(access)
            navigate("/")
          }}
          onRestart={() => setSetupToken("")}
        />
      ) : (
        <form className="grid gap-4" onSubmit={onSubmit}>
          <Field name="cluster_name" label="Cluster name" />
          <Field name="name" label="Your name" />
          <Field name="email" label="Admin email" type="email" />
          <Field name="password" label="Password" type="password" />
          {error && <p className="text-sm text-destructive">{error}</p>}
          <Button type="submit">Create cluster</Button>
        </form>
      )}
    </Center>
  )
}

export function Login() {
  const navigate = useNavigate()
  const [error, setError] = useState("")
  const [pending, setPending] = useState<ConsoleLogin | null>(null)

  useEffect(() => {
    if (pending) return
    api<{ needs_setup: boolean }>("/console/v1/setup").then((status) => {
      if (status.needs_setup) navigate("/setup")
    })
  }, [navigate, pending])

  function enter(access: string) {
    setToken(access)
    navigate("/")
  }

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    try {
      const result = await api<ConsoleLogin>("/console/v1/login", {
        method: "POST",
        body: JSON.stringify({ email: data.get("email"), password: data.get("password") }),
      })
      if (result.access_token) {
        enter(result.access_token)
        return
      }
      setPending(result)
    } catch (err) {
      setError(err instanceof Error ? err.message : "login failed")
    }
  }

  return (
    <Center title="Log in" subtitle="Console access. Project users sign in on the project itself.">
      {pending?.setup_token ? (
        <Enroll token={pending.setup_token} onDone={enter} onRestart={() => navigate("/setup")} />
      ) : pending?.mfa_token ? (
        <SecondFactor token={pending.mfa_token} onDone={enter} />
      ) : (
        <form className="grid gap-4" onSubmit={onSubmit}>
          <Field name="email" label="Email" type="email" />
          <Field name="password" label="Password" type="password" />
          {error && <p className="text-sm text-destructive">{error}</p>}
          <Button type="submit">Continue</Button>
        </form>
      )}
    </Center>
  )
}

function Center({ title, subtitle, children }: { title: string; subtitle: string; children: ReactNode }) {
  return (
    <div className="flex min-h-screen items-center justify-center bg-muted/40 p-6">
      <Card className="w-full max-w-md">
        <CardHeader>
          <Logo className="mb-2 size-12 rounded-lg" />
          <CardTitle>{title}</CardTitle>
          <p className="text-sm text-muted-foreground">{subtitle}</p>
        </CardHeader>
        <CardContent>{children}</CardContent>
      </Card>
    </div>
  )
}

function Field({ name, label, type = "text" }: { name: string; label: string; type?: string }) {
  return (
    <div className="grid gap-2">
      <Label htmlFor={name}>{label}</Label>
      <Input id={name} name={name} type={type} required />
    </div>
  )
}

export function Home() {
  return (
    <Gate>
      <HomeBody />
    </Gate>
  )
}

function HomeBody() {
  const [projects, setProjects] = useState<Project[]>([])
  const [error, setError] = useState("")
  const navigate = useNavigate()

  function load() {
    api<{ projects: Project[] }>("/console/v1/me").then((me) => setProjects(me.projects))
  }

  useEffect(load, [])

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    try {
      const project = await api<Project & { anon_key: string; service_key: string }>("/console/v1/projects", {
        method: "POST",
        body: JSON.stringify({ name: data.get("name") }),
      })
      saveKeys(project.ref, project)
      navigate(`/p/${project.ref}`)
    } catch (err) {
      setError(err instanceof Error ? err.message : "create failed")
    }
  }

  return (
    <div className="grid gap-6">
      <div>
        <h1 className="text-2xl font-medium tracking-tight">Projects</h1>
        <p className="text-sm text-muted-foreground">Each project has its own users, data, and keys.</p>
      </div>
      <div className="grid gap-3 md:grid-cols-2">
        {projects.map((project) => (
          <Link key={project.ref} to={`/p/${project.ref}`}>
            <Card className="hover:bg-muted/40">
              <CardHeader>
                <CardTitle className="flex items-center justify-between text-base">
                  {project.name || "Untitled"}
                  <Badge variant="secondary">{project.role}</Badge>
                </CardTitle>
                <p className="font-mono text-xs text-muted-foreground">{project.ref}</p>
              </CardHeader>
            </Card>
          </Link>
        ))}
        {projects.length === 0 && <p className="text-sm text-muted-foreground">No projects yet.</p>}
      </div>
      <Card className="max-w-md">
        <CardHeader>
          <CardTitle className="text-base">New project</CardTitle>
        </CardHeader>
        <CardContent>
          <form className="grid gap-3" onSubmit={onSubmit}>
            <Field name="name" label="Name" />
            {error && <p className="text-sm text-destructive">{error}</p>}
            <Button type="submit">Create project</Button>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}

export function Keys() {
  const { ref = "" } = useParams()
  const [keys, setKeys] = useState(savedKeys(ref))
  const [showService, setShowService] = useState(false)
  const [error, setError] = useState("")
  const [copied, setCopied] = useState("")

  async function rotate() {
    try {
      const next = await api<{ anon_key: string; service_key: string }>(`/console/v1/projects/${ref}/keys`, { method: "POST" })
      saveKeys(ref, next)
      setKeys(next)
      setShowService(false)
    } catch (err) {
      setError(err instanceof Error ? err.message : "rotate failed")
    }
  }

  async function copy(label: string, value: string) {
    await navigator.clipboard.writeText(value)
    setCopied(label)
  }

  return (
    <div className="grid w-full min-w-0 max-w-3xl gap-4">
      <h1 className="text-2xl font-medium tracking-tight">API keys</h1>
      <p className="text-sm text-muted-foreground">Project keys, not your console login. The service key stays hidden until you reveal it.</p>
      {keys ? (
        <div className="grid gap-3">
          <KeyRow label="anon" value={keys.anon_key} revealed copied={copied === "anon"} onCopy={() => copy("anon", keys.anon_key)} />
          <KeyRow
            label="service"
            value={keys.service_key}
            revealed={showService}
            copied={copied === "service"}
            onCopy={() => copy("service", keys.service_key)}
            onReveal={() => setShowService((value) => !value)}
          />
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">Keys from an earlier session are not stored. Rotate to issue a new pair.</p>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
      <Button variant="outline" className="w-fit" onClick={rotate}>
        Rotate keys
      </Button>
    </div>
  )
}

function KeyRow({
  label,
  value,
  revealed,
  copied,
  onCopy,
  onReveal,
}: {
  label: string
  value: string
  revealed: boolean
  copied: boolean
  onCopy: () => void
  onReveal?: () => void
}) {
  return (
    <div className="grid gap-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      <div className="flex min-w-0 items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded-md bg-muted px-2 py-1 font-mono text-xs">
          {revealed || !onReveal ? value : "••••••••••••••••"}
        </code>
        {onReveal && (
          <Button type="button" variant="outline" size="sm" className="shrink-0" onClick={onReveal}>
            {revealed ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
            {revealed ? "Hide" : "Reveal"}
          </Button>
        )}
        <Button type="button" variant="outline" size="sm" className="shrink-0" onClick={onCopy}>
          <Copy className="size-4" />
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
    </div>
  )
}

export function Users() {
  const { ref = "" } = useParams()
  const [rows, setRows] = useState<{ id: string; email: string; created_at: string }[]>([])
  useEffect(() => {
    api<{ id: string; email: string; created_at: string }[]>(`/console/v1/projects/${ref}/users`).then(setRows)
  }, [ref])
  return (
    <div className="grid gap-4">
      <h1 className="text-2xl font-medium tracking-tight">Project users</h1>
      <p className="text-sm text-muted-foreground">People who signed up to this project. They cannot open this console.</p>
      {rows.length === 0 ? (
        <p className="text-sm text-muted-foreground">Nothing here yet.</p>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Email</TableHead>
              <TableHead>Created</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={row.id}>
                <TableCell>
                  <Link className="underline" to={`/p/${ref}/auth/${row.id}`}>
                    {row.email}
                  </Link>
                </TableCell>
                <TableCell className="font-mono text-xs">{row.created_at}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  )
}

export function UserDetail() {
  const { ref = "", userId = "" } = useParams()
  const { role } = useOutletContext<ConsoleContext>()
  const navigate = useNavigate()
  const [user, setUser] = useState<{ id: string; email: string; created_at: string } | null>(null)
  const [error, setError] = useState("")
  const [notice, setNotice] = useState("")
  const canManage = role === "owner" || role === "admin"

  useEffect(() => {
    api<{ id: string; email: string; created_at: string }>(`/console/v1/projects/${ref}/users/${userId}`)
      .then(setUser)
      .catch((err) => setError(err.message))
  }, [ref, userId])

  async function changePassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const data = new FormData(form)
    try {
      await api(`/console/v1/projects/${ref}/users/${userId}/password`, {
        method: "POST",
        body: JSON.stringify({ password: data.get("password") }),
      })
      form.reset()
      setNotice("Password updated")
      setError("")
    } catch (err) {
      setError(err instanceof Error ? err.message : "update failed")
    }
  }

  async function remove() {
    if (!user || !confirm(`Delete ${user.email}?`)) return
    try {
      await api(`/console/v1/projects/${ref}/users/${userId}`, { method: "DELETE" })
      navigate(`/p/${ref}/auth`)
    } catch (err) {
      setError(err instanceof Error ? err.message : "delete failed")
    }
  }

  return (
    <div className="grid max-w-xl gap-4">
      <div>
        <Link className="text-sm text-muted-foreground underline" to={`/p/${ref}/auth`}>
          Auth
        </Link>
        <h1 className="text-2xl font-medium tracking-tight">{user?.email || "User"}</h1>
      </div>
      {user && (
        <Card>
          <CardContent className="grid gap-1 pt-6 text-sm">
            <div>Email: {user.email}</div>
            <div className="font-mono text-xs text-muted-foreground">{user.id}</div>
            <div className="text-muted-foreground">Created {user.created_at}</div>
          </CardContent>
        </Card>
      )}
      {canManage && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Password</CardTitle>
          </CardHeader>
          <CardContent>
            <form className="grid gap-3" onSubmit={changePassword}>
              <Field name="password" label="New password" type="password" />
              <Button type="submit" className="w-fit">
                Update password
              </Button>
            </form>
          </CardContent>
        </Card>
      )}
      {canManage && (
        <Button variant="outline" className="w-fit" onClick={remove}>
          Delete user
        </Button>
      )}
      {notice && <p className="text-sm text-muted-foreground">{notice}</p>}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}

export function Team() {
  const { ref = "" } = useParams()
  const [rows, setRows] = useState<{ email: string; name: string; role: string }[]>([])
  const [error, setError] = useState("")

  function load() {
    api<{ email: string; name: string; role: string }[]>(`/console/v1/projects/${ref}/members`).then(setRows)
  }
  useEffect(load, [ref])

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const data = new FormData(form)
    try {
      await api(`/console/v1/projects/${ref}/members`, {
        method: "POST",
        body: JSON.stringify({
          email: data.get("email"),
          name: data.get("name"),
          password: data.get("password"),
          role: data.get("role"),
        }),
      })
      form.reset()
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "invite failed")
    }
  }

  return (
    <div className="grid max-w-xl gap-4">
      <h1 className="text-2xl font-medium tracking-tight">Team</h1>
      <SimpleTable columns={["Email", "Name", "Role"]} rows={rows.map((row) => [row.email, row.name, row.role])} />
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Add a member</CardTitle>
        </CardHeader>
        <CardContent>
          <form className="grid gap-3" onSubmit={onSubmit}>
            <Field name="name" label="Name" />
            <Field name="email" label="Email" type="email" />
            <Field name="password" label="Password" type="password" />
            <div className="grid gap-2">
              <Label htmlFor="role">Role</Label>
              <select id="role" name="role" className="h-9 rounded-md border bg-background px-2 text-sm">
                <option value="developer">developer</option>
                <option value="admin">admin</option>
              </select>
            </div>
            {error && <p className="text-sm text-destructive">{error}</p>}
            <Button type="submit">Add</Button>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}

function SimpleTable({ columns, rows }: { columns: string[]; rows: string[][] }) {
  if (rows.length === 0) return <p className="text-sm text-muted-foreground">Nothing here yet.</p>
  return (
    <Table>
      <TableHeader>
        <TableRow>
          {columns.map((column) => (
            <TableHead key={column}>{column}</TableHead>
          ))}
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map((row, index) => (
          <TableRow key={index}>
            {row.map((cell, cellIndex) => (
              <TableCell key={cellIndex} className="max-w-md truncate font-mono text-xs">
                {cell}
              </TableCell>
            ))}
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}
