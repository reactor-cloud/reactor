import { Link, NavLink, Outlet, useLocation, useNavigate, useParams } from "react-router-dom"
import { Logo } from "@/components/logo"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { api, clearToken, placeLabel, saveKeys, type Project } from "@/lib/api"
import { Operator, OperatorMark } from "@/operator"
import { useEffect, useState, type FormEvent } from "react"
import {
  ChevronDown,
  type LucideIcon,
  Database,
  FolderKanban,
  Globe,
  HardDrive,
  KeyRound,
  LayoutDashboard,
  LogOut,
  Mail,
  Plus,
  ScrollText,
  Server,
  Settings,
  Users,
  UsersRound,
  Zap,
} from "lucide-react"

const sections = [
  { path: "", label: "Overview", icon: LayoutDashboard },
  { path: "keys", label: "API keys", icon: KeyRound },
  { path: "auth", label: "Auth", icon: Users },
  { path: "email", label: "Email", icon: Mail },
  { path: "data", label: "Data", icon: Database },
  { path: "storage", label: "Storage", icon: HardDrive },
  { path: "functions", label: "Functions", icon: Zap },
  { path: "sites", label: "Sites", icon: Globe },
] as const

const projectFoot = [
  { path: "logs", label: "Logs", icon: ScrollText },
  { path: "team", label: "Team", icon: UsersRound },
  { path: "settings", label: "Settings", icon: Settings },
] as const

export type ConsoleContext = {
  role: string
  name: string
  email: string
}

export function Shell() {
  const { ref } = useParams()
  const { pathname } = useLocation()
  const navigate = useNavigate()
  const [projects, setProjects] = useState<Project[]>([])
  const [email, setEmail] = useState("")
  const [name, setName] = useState("")
  const [admin, setAdmin] = useState(false)
  const [clusterName, setClusterName] = useState("")
  const [clusterType, setClusterType] = useState("")
  const [clusterOk, setClusterOk] = useState<boolean | null>(null)
  const [wide, setWide] = useState(false)
  const [menuOpen, setMenuOpen] = useState(false)
  const [query, setQuery] = useState("")
  const [creating, setCreating] = useState(false)
  const [error, setError] = useState("")
  const [operatorOpen, setOperatorOpen] = useState(false)

  useEffect(() => {
    function load() {
      api<{ operator: { email: string; name: string; platform_admin: boolean }; cluster_name: string; projects: Project[] }>(
        "/console/v1/me",
      )
        .then((me) => {
          setEmail(me.operator.email)
          setName(me.operator.name)
          setAdmin(me.operator.platform_admin)
          setClusterName(me.cluster_name || "")
          setProjects(me.projects)
          if (!me.operator.platform_admin) return
          api<ClusterStatus>("/console/v1/cluster").then((cluster) => {
            setClusterName(cluster.name)
            setClusterType(placeLabel(cluster.place))
            const parts = [cluster.reactor, cluster.postgres, cluster.postgrest, cluster.blobs]
            const dedicated = cluster.postgrest_dedicated
            const dedicatedOk = !dedicated?.active || dedicated.ok === true
            setClusterOk(parts.every((part) => part.ok) && dedicatedOk)
          })
        })
        .catch(() => navigate("/login"))
    }
    load()
    window.addEventListener("reactor-projects", load)
    return () => window.removeEventListener("reactor-projects", load)
  }, [navigate, ref, pathname])

  const current = projects.find((project) => project.ref === ref)
  const expanded = wide
  const shown = projects.filter((project) => {
    const text = `${project.name} ${project.ref}`.toLowerCase()
    return text.includes(query.trim().toLowerCase())
  })

  async function createProject(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    try {
      const project = await api<Project & { anon_key: string; service_key: string }>("/console/v1/projects", {
        method: "POST",
        body: JSON.stringify({ name: data.get("name") }),
      })
      saveKeys(project.ref, project)
      setMenuOpen(false)
      setCreating(false)
      navigate(`/p/${project.ref}/keys`)
    } catch (err) {
      setError(err instanceof Error ? err.message : "create failed")
    }
  }

  return (
    <div className="flex h-screen overflow-hidden bg-background text-foreground">
      <aside
        className={`flex h-full min-h-0 shrink-0 flex-col overflow-y-auto border-r bg-background transition-[width] duration-150 ${expanded ? "w-60" : "w-14"}`}
        onMouseEnter={() => setWide(true)}
        onMouseLeave={() => setWide(false)}
      >
        <div className={`flex h-14 items-center border-b ${expanded ? "px-3" : "justify-center"}`}>
          <Link to="/" className="min-w-0">
            <Logo className={expanded ? "size-8" : "size-7"} expanded={expanded} />
          </Link>
        </div>
        <nav className="flex min-h-0 flex-1 flex-col gap-1 p-2">
          {ref
            ? <>
                {sections.map((section) => (
                  <ProjectLink key={section.label} pref={ref} expanded={expanded} section={section} />
                ))}
                <div className="mt-auto flex flex-col gap-1">
                  {projectFoot.map((section) => (
                    <ProjectLink key={section.label} pref={ref} expanded={expanded} section={section} />
                  ))}
                </div>
              </>
            : homeNav
                .filter((item) => !item.adminOnly || admin)
                .map((item) => {
                  const Icon = item.icon
                  return (
                    <NavLink
                      key={item.label}
                      to={item.to}
                      end={item.end}
                      title={item.label}
                      className={({ isActive }) =>
                        `flex items-center gap-3 rounded-md px-2 py-2 text-sm ${isActive ? "font-medium text-foreground" : "text-muted-foreground hover:bg-muted/60"}`
                      }
                    >
                      {({ isActive }) => (
                        <>
                          <Icon className={`size-4 shrink-0 ${isActive ? "text-blue-500" : ""}`} />
                          {expanded && <span className="truncate">{item.label}</span>}
                        </>
                      )}
                    </NavLink>
                  )
                })}
        </nav>
      </aside>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <header className="flex h-14 items-center gap-3 border-b px-4">
          {ref && (
            <DropdownMenu
              open={menuOpen}
              onOpenChange={(open) => {
                setMenuOpen(open)
                if (!open) {
                  setCreating(false)
                  setQuery("")
                  setError("")
                }
              }}
            >
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="sm" className="max-w-64">
                  <span className="truncate">{current?.name || ref}</span>
                  <ChevronDown className="size-4 shrink-0 opacity-70" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-64">
                <form
                  className="p-2"
                  onSubmit={(event) => {
                    event.preventDefault()
                  }}
                >
                  <Input
                    value={query}
                    placeholder="Find a project"
                    onChange={(event) => setQuery(event.target.value)}
                    onKeyDown={(event) => event.stopPropagation()}
                  />
                </form>
                <DropdownMenuItem
                  onSelect={(event) => {
                    event.preventDefault()
                    setCreating(true)
                  }}
                >
                  <Plus className="size-4" />
                  New project
                </DropdownMenuItem>
                {creating && (
                  <form className="grid gap-2 p-2" onSubmit={createProject}>
                    <Input name="name" placeholder="Project name" required autoFocus onKeyDown={(event) => event.stopPropagation()} />
                    {error && <p className="text-xs text-destructive">{error}</p>}
                    <Button type="submit" size="sm">
                      Create
                    </Button>
                  </form>
                )}
                <DropdownMenuSeparator />
                {shown.map((project) => (
                  <DropdownMenuItem key={project.ref} onClick={() => navigate(`/p/${project.ref}`)}>
                    <span className="truncate">{project.name || project.ref}</span>
                  </DropdownMenuItem>
                ))}
                {shown.length === 0 && <p className="px-2 py-1.5 text-sm text-muted-foreground">No matches</p>}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          <div className="ml-auto flex items-center gap-3">
          {clusterName && (
            <Link
              to="/cluster"
              className="flex max-w-64 items-center gap-2 rounded-full border px-2.5 py-1 text-sm hover:bg-muted"
            >
              {clusterOk !== null && (
                <span className={`size-2 shrink-0 rounded-full ${clusterOk ? "bg-emerald-500" : "bg-red-500"}`} />
              )}
              <span className="truncate font-medium">{clusterName}</span>
              {clusterType && <span className="text-muted-foreground">{clusterType}</span>}
            </Link>
          )}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button className="flex size-8 items-center justify-center rounded-full bg-foreground text-xs font-medium text-background">
                {initials(name, email)}
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-56">
              <DropdownMenuLabel>
                <div className="font-medium">{name || "Operator"}</div>
                <div className="text-xs font-normal text-muted-foreground">{email}</div>
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onClick={() => {
                  clearToken()
                  navigate("/login")
                }}
              >
                <LogOut className="size-4" />
                Log out
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          {!operatorOpen && (
            <button
              type="button"
              aria-label="Reactor Operator"
              className="outline-none"
              onClick={() => setOperatorOpen(true)}
            >
              <OperatorMark className="size-8 rounded-lg" />
            </button>
          )}
          </div>
        </header>
        <main className="relative min-h-0 min-w-0 flex-1 overflow-auto p-6">
          <Outlet context={{ role: current?.role || "", name, email } satisfies ConsoleContext} />
        </main>
      </div>
      <Operator open={operatorOpen} onOpenChange={setOperatorOpen} projectRef={ref} />
    </div>
  )
}

type ClusterStatus = {
  name: string
  place: string
  type: string
  reactor: { ok: boolean }
  postgres: { ok: boolean }
  postgrest: { ok: boolean }
  postgrest_dedicated: { active: boolean; ok?: boolean } | null
  blobs: { ok: boolean }
}

function ProjectLink({
  pref,
  expanded,
  section,
}: {
  pref: string
  expanded: boolean
  section: { path: string; label: string; icon: LucideIcon }
}) {
  const Icon = section.icon
  return (
    <NavLink
      to={section.path ? `/p/${pref}/${section.path}` : `/p/${pref}`}
      end={!section.path}
      title={section.label}
      className={({ isActive }) =>
        `flex items-center gap-3 rounded-md px-2 py-2 text-sm ${isActive ? "font-medium text-foreground" : "text-muted-foreground hover:bg-muted/60"}`
      }
    >
      {({ isActive }) => (
        <>
          <Icon className={`size-4 shrink-0 ${isActive ? "text-blue-500" : ""}`} />
          {expanded && <span className="truncate">{section.label}</span>}
        </>
      )}
    </NavLink>
  )
}

const homeNav = [
  { to: "/", label: "Projects", icon: FolderKanban, end: true, adminOnly: false },
  { to: "/cluster", label: "Cluster", icon: Server, end: true, adminOnly: true },
  { to: "/users", label: "Console users", icon: Users, end: true, adminOnly: true },
] as const

function initials(name: string, email: string) {
  const source = name.trim() || email
  const parts = source.split(/[\s@]+/).filter(Boolean)
  const first = parts[0]?.[0] || "?"
  const second = parts[1]?.[0] || ""
  return (first + second).toUpperCase()
}
