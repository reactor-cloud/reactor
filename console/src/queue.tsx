import { useEffect, useState, type FormEvent } from "react"
import { useParams } from "react-router-dom"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { api } from "@/lib/api"
import { ListOrdered, Plus } from "lucide-react"

type QueueRow = { name: string; created_at: string }
type Message = { msg_id: number; message: unknown; vt: string; read_ct: number; enqueued_at: string }
type Subscription = { function_name: string; vt_secs: number; qty: number; max_reads: number }
type Tab = "messages" | "subscribers"

export function Queue() {
  const { ref = "" } = useParams()
  const [queues, setQueues] = useState<QueueRow[]>([])
  const [selected, setSelected] = useState("")
  const [tab, setTab] = useState<Tab>("messages")
  const [filter, setFilter] = useState("")
  const [creating, setCreating] = useState(false)
  const [messages, setMessages] = useState<Message[]>([])
  const [subscriptions, setSubscriptions] = useState<Subscription[]>([])
  const [error, setError] = useState("")

  function load() {
    api<QueueRow[]>(`/console/v1/projects/${ref}/queues`)
      .then((rows) => {
        setQueues(rows)
        setSelected((current) => (rows.some((row) => row.name === current) ? current : ""))
        setError("")
      })
      .catch((err) => setError(err.message))
  }
  useEffect(load, [ref])

  useEffect(() => {
    if (!selected) {
      setMessages([])
      setSubscriptions([])
      return
    }
    api<Message[]>(`/console/v1/projects/${ref}/queues/${selected}/peek`)
      .then(setMessages)
      .catch((err) => setError(err.message))
    api<Subscription[]>(`/console/v1/projects/${ref}/queues/${selected}/subscriptions`)
      .then(setSubscriptions)
      .catch((err) => setError(err.message))
  }, [ref, selected])

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const name = String(new FormData(form).get("name") || "")
    try {
      await api(`/console/v1/projects/${ref}/queues`, { method: "POST", body: JSON.stringify({ name }) })
      form.reset()
      setCreating(false)
      setSelected(name)
      setTab("messages")
      load()
    } catch (err) {
      setError(err instanceof Error ? err.message : "create failed")
    }
  }

  async function send(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const raw = String(new FormData(form).get("message") || "{}")
    let message: unknown = raw
    try {
      message = JSON.parse(raw)
    } catch {
      message = raw
    }
    try {
      await api(`/console/v1/projects/${ref}/queues/${selected}/send`, {
        method: "POST",
        body: JSON.stringify({ message, delay_secs: 0 }),
      })
      form.reset()
      setMessages(await api<Message[]>(`/console/v1/projects/${ref}/queues/${selected}/peek`))
      setError("")
    } catch (err) {
      setError(err instanceof Error ? err.message : "send failed")
    }
  }

  const shown = queues.filter((queue) => queue.name.includes(filter.trim().toLowerCase()))

  return (
    <div className="absolute inset-0 flex min-h-0">
      <aside className="flex w-56 shrink-0 flex-col border-r">
        <div className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
          <ListOrdered className="size-4 shrink-0" />
          <h1 className="text-base font-semibold">Queue</h1>
        </div>
        <div className="grid gap-2 border-b p-3">
          <Input value={filter} placeholder="Search queues" onChange={(event) => setFilter(event.target.value)} />
          <Button variant="outline" size="sm" onClick={() => setCreating((value) => !value)}>
            <Plus className="size-4" />
            New queue
          </Button>
        </div>
        {creating && (
          <form className="grid gap-2 border-b p-3" onSubmit={create}>
            <Input name="name" placeholder="queue_name" required pattern="[a-z][a-z0-9_]{0,62}" />
            {error && <p className="text-xs text-destructive">{error}</p>}
            <Button type="submit" size="sm">Create</Button>
          </form>
        )}
        <div className="min-h-0 flex-1 overflow-auto p-2">
          {shown.map((queue) => (
            <button
              key={queue.name}
              className={`flex w-full rounded-md px-2 py-1.5 text-left text-sm ${queue.name === selected ? "bg-muted font-medium" : "hover:bg-muted/60"}`}
              onClick={() => setSelected(queue.name)}
            >
              {queue.name}
            </button>
          ))}
          {shown.length === 0 && <p className="px-2 py-1.5 text-sm text-muted-foreground">No queues yet.</p>}
        </div>
      </aside>
      <section className="flex min-w-0 flex-1 flex-col">
        {selected ? (
          <>
            <div className="flex h-10 items-end gap-1 border-b px-2">
              <TabButton current={tab === "messages"} onClick={() => setTab("messages")}>Messages</TabButton>
              <TabButton current={tab === "subscribers"} onClick={() => setTab("subscribers")}>Subscribers</TabButton>
            </div>
            {tab === "messages" ? (
              <div className="flex min-h-0 flex-1 flex-col">
                <form className="flex h-12 shrink-0 items-center gap-2 border-b px-3" onSubmit={send}>
                  <Input name="message" placeholder='{"hello":"world"}' required className="max-w-xs" />
                  {error && <p className="truncate text-sm text-destructive">{error}</p>}
                  <Button type="submit" variant="outline" size="sm" className="ml-auto">Send</Button>
                </form>
                <div className="min-h-0 flex-1 overflow-auto">
                  <table className="w-full text-sm">
                    <thead className="sticky top-0 z-10 bg-muted text-left">
                      <tr>
                        <th className="whitespace-nowrap px-3 py-2 font-medium">id</th>
                        <th className="px-3 py-2 font-medium">message</th>
                        <th className="whitespace-nowrap px-3 py-2 font-medium">reads</th>
                        <th className="whitespace-nowrap px-3 py-2 font-medium">visible</th>
                      </tr>
                    </thead>
                    <tbody>
                      {messages.map((row) => (
                        <tr key={row.msg_id} className="border-t">
                          <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">{row.msg_id}</td>
                          <td className="max-w-md truncate px-3 py-1.5 font-mono text-xs">{JSON.stringify(row.message)}</td>
                          <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">{row.read_ct}</td>
                          <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">{row.vt}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  {messages.length === 0 && <p className="p-4 text-sm text-muted-foreground">No messages.</p>}
                </div>
                <div className="flex h-10 shrink-0 items-center border-t px-3 text-xs text-muted-foreground">{messages.length} messages</div>
              </div>
            ) : (
              <div className="flex min-h-0 flex-1 flex-col">
                <div className="min-h-0 flex-1 overflow-auto">
                  <table className="w-full text-sm">
                    <thead className="sticky top-0 z-10 bg-muted text-left">
                      <tr>
                        <th className="px-3 py-2 font-medium">function</th>
                        <th className="whitespace-nowrap px-3 py-2 font-medium">visibility</th>
                        <th className="whitespace-nowrap px-3 py-2 font-medium">qty</th>
                        <th className="whitespace-nowrap px-3 py-2 font-medium">max reads</th>
                      </tr>
                    </thead>
                    <tbody>
                      {subscriptions.map((row) => (
                        <tr key={row.function_name} className="border-t">
                          <td className="px-3 py-1.5 font-mono text-xs">{row.function_name}</td>
                          <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">{row.vt_secs}s</td>
                          <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">{row.qty}</td>
                          <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs">{row.max_reads}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  {subscriptions.length === 0 && <p className="p-4 text-sm text-muted-foreground">No subscribers.</p>}
                </div>
                <div className="flex h-10 shrink-0 items-center border-t px-3 text-xs text-muted-foreground">{subscriptions.length} subscribers</div>
              </div>
            )}
          </>
        ) : (
          <p className="p-4 text-sm text-muted-foreground">Select a queue.</p>
        )}
      </section>
    </div>
  )
}

function TabButton({ current, onClick, children }: { current: boolean; onClick: () => void; children: string }) {
  return (
    <button
      type="button"
      className={`rounded-t-md border border-b-0 px-3 py-1.5 text-sm ${current ? "bg-background" : "bg-muted/50 text-muted-foreground"}`}
      onClick={onClick}
    >
      {children}
    </button>
  )
}
