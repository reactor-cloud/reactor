import { useEffect, useState } from "react"
import { useParams } from "react-router-dom"
import { api } from "@/lib/api"
import { Database, Globe, HardDrive, Users, Zap, type LucideIcon } from "lucide-react"

export type Point = { ok: number; warn: number; error: number }
type OverviewBody = {
  name: string
  counts: { users: number; tables: number; files: number; functions: number; sites: number }
  hours: number[]
  series: Record<string, Point[]>
}

export function Overview() {
  const { ref = "" } = useParams()
  const [body, setBody] = useState<OverviewBody | null>(null)
  const [error, setError] = useState("")

  useEffect(() => {
    api<OverviewBody>(`/console/v1/projects/${ref}/overview`)
      .then(setBody)
      .catch((err) => setError(err.message))
  }, [ref])

  const stats: { label: string; value: number; icon: LucideIcon }[] = body
    ? [
        { label: "Users", value: body.counts.users, icon: Users },
        { label: "Tables", value: body.counts.tables, icon: Database },
        { label: "Files", value: body.counts.files, icon: HardDrive },
        { label: "Functions", value: body.counts.functions, icon: Zap },
        { label: "Sites", value: body.counts.sites, icon: Globe },
      ]
    : []
  const charts = body ? trafficCharts(body.series) : []
  const totals = charts.map((chart) => sum(chart.points))
  const requests = totals.reduce((sum, item) => sum + item.total, 0)
  const ok = totals.reduce((sum, item) => sum + item.ok, 0)

  return (
    <div className="grid gap-8">
      <div>
        <h1 className="text-2xl font-medium tracking-tight">{body?.name || ref}</h1>
        <p className="font-mono text-sm text-muted-foreground">{ref}</p>
      </div>
      {error && <p className="text-sm text-destructive">{error}</p>}
      <div className="grid grid-cols-5 gap-3">
        {stats.map((stat) => {
          const Icon = stat.icon
          return (
            <div key={stat.label} className="rounded-lg border px-4 py-3">
              <div className="flex items-center gap-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
                <Icon className="size-3.5" />
                {stat.label}
              </div>
              <div className="mt-1 text-2xl font-medium tabular-nums">{stat.value}</div>
            </div>
          )
        })}
      </div>
      <section className="grid gap-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-baseline gap-3">
            <span className="text-sm font-medium tabular-nums">{requests} requests</span>
            <span className="text-sm text-muted-foreground tabular-nums">
              {requests === 0 ? "No traffic" : `${((ok / requests) * 100).toFixed(1)}% success`}
            </span>
          </div>
          <span className="rounded-md border px-2 py-1 text-xs text-muted-foreground">Last 24 hours</span>
        </div>
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          {charts.map((chart) => (
            <TrafficCard key={chart.label} label={chart.label} hours={body?.hours || []} points={chart.points} />
          ))}
        </div>
      </section>
    </div>
  )
}

export function TrafficCard({ label, hours, points }: { label: string; hours: number[]; points: Point[] }) {
  const totals = sum(points)
  const peak = Math.max(1, ...points.map((point) => point.ok + point.warn + point.error))
  const marks = [0, Math.floor((hours.length - 1) / 2), hours.length - 1].filter((index, place, all) => all.indexOf(index) === place)
  const [hover, setHover] = useState<number | null>(null)
  const point = hover == null ? null : points[hover]
  const left = hover == null || points.length === 0 ? 0 : ((hover + 0.5) / points.length) * 100

  return (
    <div className="rounded-lg border p-4">
      <div className="flex items-start justify-between gap-2">
        <div className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{label}</div>
        <div className="flex gap-3 text-[11px] text-muted-foreground">
          <span className="inline-flex items-center gap-1">
            <span className="size-1.5 rounded-full bg-amber-500" />
            {totals.warn}
          </span>
          <span className="inline-flex items-center gap-1">
            <span className="size-1.5 rounded-full bg-red-500" />
            {totals.error}
          </span>
        </div>
      </div>
      <div className="mt-1 text-lg font-medium tabular-nums">{totals.total}</div>
      <div className="relative mt-3">
        {point && hover != null && (
          <div
            className="pointer-events-none absolute bottom-full z-20 mb-2 w-44 rounded-md border bg-popover px-2.5 py-2 text-xs text-popover-foreground shadow-md"
            style={{ left: `${left}%`, transform: left < 25 ? "translateX(0)" : left > 75 ? "translateX(-100%)" : "translateX(-50%)" }}
          >
            <div className="font-medium">{label}</div>
            <div className="text-muted-foreground">{hours[hover] ? hourRange(hours[hover]) : ""}</div>
            <div className="mt-1.5 grid gap-0.5">
              <Tip swatch="bg-emerald-500" label="Succeeded" value={point.ok} />
              <Tip swatch="bg-amber-500" label="Warnings" value={point.warn} />
              <Tip swatch="bg-red-500" label="Errors" value={point.error} />
            </div>
          </div>
        )}
        <div className="flex h-24 items-end gap-px">
          {points.map((item, index) => {
            const total = item.ok + item.warn + item.error
            const height = total === 0 ? 0 : Math.max(4, Math.round((total / peak) * 96))
            return (
              <div
                key={hours[index] ?? index}
                className="flex h-full flex-1 items-end"
                onMouseEnter={() => setHover(index)}
                onMouseLeave={() => setHover(null)}
              >
                <div className="flex w-full flex-col justify-end overflow-hidden rounded-sm" style={{ height }}>
                  {item.error > 0 && <div className="bg-red-500" style={{ height: `${(item.error / total) * 100}%` }} />}
                  {item.warn > 0 && <div className="bg-amber-500" style={{ height: `${(item.warn / total) * 100}%` }} />}
                  {item.ok > 0 && <div className="bg-emerald-500" style={{ height: `${(item.ok / total) * 100}%` }} />}
                </div>
              </div>
            )
          })}
        </div>
      </div>
      <div className="mt-2 flex justify-between text-[10px] text-muted-foreground">
        {marks.map((index) => (
          <span key={index}>{hours[index] ? hourLabel(hours[index]) : ""}</span>
        ))}
      </div>
    </div>
  )
}

function Tip({ swatch, label, value }: { swatch: string; label: string; value: number }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="inline-flex items-center gap-1.5 text-muted-foreground">
        <span className={`size-1.5 rounded-full ${swatch}`} />
        {label}
      </span>
      <span className="tabular-nums">{value}</span>
    </div>
  )
}

const chartOrder = ["auth", "database", "function", "site"]

function trafficCharts(series: Record<string, Point[]>) {
  const extra = Object.keys(series)
    .filter((kind) => !chartOrder.includes(kind))
    .sort()
  return [...chartOrder, ...extra].map((kind) => ({
    label: kindLabel(kind),
    points: series[kind] || [],
  }))
}

function kindLabel(kind: string) {
  if (kind === "auth") return "Auth"
  if (kind === "database") return "Database"
  if (kind === "function") return "Functions"
  if (kind === "site") return "Sites"
  if (kind === "queue") return "Queue"
  return kind
}

function sum(points: Point[]) {
  return points.reduce<{ ok: number; warn: number; error: number; total: number }>(
    (total, point) => ({
      ok: total.ok + point.ok,
      warn: total.warn + point.warn,
      error: total.error + point.error,
      total: total.total + point.ok + point.warn + point.error,
    }),
    { ok: 0, warn: 0, error: 0, total: 0 },
  )
}

function hourLabel(epoch: number) {
  return new Date(epoch * 1000).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric" })
}

function hourRange(epoch: number) {
  const start = new Date(epoch * 1000)
  const end = new Date(epoch * 1000 + 60 * 60 * 1000)
  const day = start.toLocaleDateString(undefined, { month: "short", day: "numeric" })
  const from = start.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
  const to = end.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
  return `${day}, ${from}–${to}`
}
