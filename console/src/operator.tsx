import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react"
import { ArrowUp, History, ImagePlus, SquarePen, Star, X } from "lucide-react"
import { Popover } from "radix-ui"
import Markdown from "react-markdown"
import remarkGfm from "remark-gfm"
import { api } from "@/lib/api"

const MODEL_KEY = "reactor.agent.model"
const FAVORITES_KEY = "reactor.agent.favorites"

type ImageFile = { name: string; url: string }
type Message = { role: "user" | "assistant" | "tool"; text: string; name?: string; images: string[] }
type Thread = {
  id: string | null
  running: boolean
  messages: { role: string; content: string; name?: string | null }[]
}
type Conversation = { id: string; running: boolean; preview: string | null; at: number }
type CatalogModel = { id: string; name: string }

function loadFavorites() {
  try {
    const raw = JSON.parse(localStorage.getItem(FAVORITES_KEY) || "[]")
    return Array.isArray(raw) ? raw.filter((item): item is string => typeof item === "string") : []
  } catch {
    return []
  }
}

function fromThread(thread: Thread): Message[] {
  return thread.messages.map((message) => ({
    role: message.role === "user" ? "user" : message.role === "tool" ? "tool" : "assistant",
    text: message.content,
    name: message.name || undefined,
    images: [],
  }))
}

export function Operator({
  open,
  onOpenChange,
  projectRef,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  projectRef?: string
}) {
  const [threadId, setThreadId] = useState<string | null>(null)
  const [messages, setMessages] = useState<Message[]>([])
  const [running, setRunning] = useState(false)
  const [error, setError] = useState("")
  const [draft, setDraft] = useState("")
  const [images, setImages] = useState<ImageFile[]>([])
  const [pane, setPane] = useState<"thread" | "history">("thread")
  const [conversations, setConversations] = useState<Conversation[]>([])
  const [historyReady, setHistoryReady] = useState(false)
  const [model, setModel] = useState(() => localStorage.getItem(MODEL_KEY) || "")
  const [favorites, setFavorites] = useState<string[]>(loadFavorites)
  const [catalog, setCatalog] = useState<CatalogModel[]>([])
  const fileRef = useRef<HTMLInputElement>(null)
  const threadRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    setThreadId(null)
    setMessages([])
    setRunning(false)
    setError("")
    setDraft("")
    setImages([])
    setPane("thread")
    let stop = false
    api<{ default: string; models: CatalogModel[] }>("/console/v1/agent/models")
      .then((body) => {
        if (stop) return
        setCatalog(body.models)
        setModel((current) => {
          if (current) return current
          localStorage.setItem(MODEL_KEY, body.default)
          return body.default
        })
      })
      .catch(() => {})
    return () => {
      stop = true
    }
  }, [open])

  useEffect(() => {
    if (!open || !threadId || !running) return
    let stop = false
    const timer = setInterval(() => {
      api<Thread>(`/console/v1/agent/threads/${threadId}`)
        .then((thread) => {
          if (stop) return
          setMessages(fromThread(thread))
          setRunning(thread.running)
        })
        .catch((err: Error) => {
          if (!stop) setError(err.message)
        })
    }, 1000)
    return () => {
      stop = true
      clearInterval(timer)
    }
  }, [open, threadId, running])

  const last = messages.at(-1)
  const thinking = running && !(last?.role === "assistant" && last.text.trim())

  useEffect(() => {
    const thread = threadRef.current
    if (thread) thread.scrollTop = thread.scrollHeight
  }, [messages, open, thinking])

  function addImages(list: FileList | null) {
    if (!list) return
    for (const file of list) {
      if (!file.type.startsWith("image/")) continue
      const reader = new FileReader()
      reader.onload = () => {
        setImages((current) => [...current, { name: file.name, url: String(reader.result) }])
      }
      reader.readAsDataURL(file)
    }
  }

  async function send() {
    const text = draft.trim()
    if (!text || running) return
    setDraft("")
    setImages([])
    setError("")
    setMessages((current) => [...current, { role: "user", text, images: images.map((image) => image.url) }])
    setRunning(true)
    try {
      let id = threadId
      if (!id) {
        const created = await api<{ id: string }>("/console/v1/agent/threads", {
          method: "POST",
          body: JSON.stringify({ project_ref: projectRef || null }),
        })
        id = created.id
        setThreadId(id)
      }
      setPane("thread")
      await api(`/console/v1/agent/threads/${id}/messages`, {
        method: "POST",
        body: JSON.stringify({ text, model: model || undefined }),
      })
    } catch (err) {
      setRunning(false)
      setError(err instanceof Error ? err.message : "The message was not sent")
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault()
      send()
    }
  }

  function startNew() {
    setThreadId(null)
    setMessages([])
    setRunning(false)
    setError("")
    setDraft("")
    setImages([])
    setPane("thread")
  }

  async function showHistory() {
    setPane("history")
    setHistoryReady(false)
    setError("")
    try {
      setConversations(await api<Conversation[]>("/console/v1/agent/threads"))
    } catch (err) {
      setConversations([])
      setError(err instanceof Error ? err.message : "History is unavailable")
    } finally {
      setHistoryReady(true)
    }
  }

  async function openConversation(id: string) {
    setError("")
    try {
      const thread = await api<Thread>(`/console/v1/agent/threads/${id}`)
      setThreadId(thread.id)
      setMessages(fromThread(thread))
      setRunning(thread.running)
      setPane("thread")
    } catch (err) {
      setError(err instanceof Error ? err.message : "That conversation is unavailable")
    }
  }

  function chooseModel(id: string) {
    setModel(id)
    localStorage.setItem(MODEL_KEY, id)
  }

  function toggleFavorite(id: string) {
    setFavorites((current) => {
      const next = current.includes(id) ? current.filter((item) => item !== id) : [...current, id]
      localStorage.setItem(FAVORITES_KEY, JSON.stringify(next))
      return next
    })
  }

  if (!open) return null

  const canSend = draft.trim().length > 0 && !running

  return (
    <aside className="flex h-full min-h-0 w-[380px] shrink-0 flex-col overflow-hidden border-l bg-background">
      <div className="flex h-14 shrink-0 items-center gap-2 border-b px-3">
        <OperatorMark className="size-8 rounded-lg" />
        <span className="min-w-0 truncate text-sm font-medium">Reactor Operator</span>
        <div className="ml-auto flex items-center">
          <button
            type="button"
            aria-label="New conversation"
            className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted"
            onClick={startNew}
          >
            <SquarePen className="size-4" />
          </button>
          <button
            type="button"
            aria-label="Conversation history"
            aria-pressed={pane === "history"}
            className={`flex size-7 items-center justify-center rounded-md hover:bg-muted ${pane === "history" ? "bg-muted text-foreground" : "text-muted-foreground"}`}
            onClick={showHistory}
          >
            <History className="size-4" />
          </button>
          <button
            type="button"
            aria-label="Close Reactor Operator"
            className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted"
            onClick={() => onOpenChange(false)}
          >
            <X className="size-4" />
          </button>
        </div>
      </div>
      <div ref={threadRef} className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4">
        {pane === "history" ? (
          <HistoryList conversations={conversations} ready={historyReady} onOpen={openConversation} />
        ) : (
          messages.map((message, index) => <MessageView key={index} message={message} />)
        )}
        {pane === "thread" && thinking && <Thinking />}
        {error && <p className="text-sm text-destructive">{error}</p>}
      </div>
      <form
        className="shrink-0 border-t p-3"
        onSubmit={(event) => {
          event.preventDefault()
          send()
        }}
      >
        <div className="rounded-xl border bg-background shadow-sm">
          {images.length > 0 && (
            <div className="flex flex-wrap gap-2 px-2 pt-2">
              {images.map((image, index) => (
                <div key={`${image.name}-${index}`} className="relative">
                  <img src={image.url} alt={image.name} className="size-14 rounded-md object-cover" />
                  <button
                    type="button"
                    aria-label={`Remove ${image.name}`}
                    className="absolute -top-1.5 -right-1.5 flex size-4 items-center justify-center rounded-full bg-foreground text-background"
                    onClick={() => setImages((current) => current.filter((_, place) => place !== index))}
                  >
                    <X className="size-2.5" />
                  </button>
                </div>
              ))}
            </div>
          )}
          <textarea
            value={draft}
            rows={2}
            placeholder="Ask Reactor Operator…"
            className="max-h-40 w-full resize-none bg-transparent px-3 pt-3 text-sm outline-none placeholder:text-muted-foreground"
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={onKeyDown}
          />
          <div className="flex items-center gap-1 px-2 pb-2">
            <button
              type="button"
              aria-label="Upload image"
              className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted"
              onClick={() => fileRef.current?.click()}
            >
              <ImagePlus className="size-4" />
            </button>
            <ModelPicker
              model={model}
              catalog={catalog}
              favorites={favorites}
              onChoose={chooseModel}
              onToggleFavorite={toggleFavorite}
            />
            <input
              ref={fileRef}
              type="file"
              accept="image/*"
              multiple
              className="hidden"
              onChange={(event) => {
                addImages(event.target.files)
                event.target.value = ""
              }}
            />
            <button
              type="submit"
              aria-label="Send"
              disabled={!canSend}
              className="ml-auto flex size-7 items-center justify-center rounded-full bg-foreground text-background disabled:opacity-30"
            >
              <ArrowUp className="size-4" />
            </button>
          </div>
        </div>
      </form>
    </aside>
  )
}

export function OperatorMark({ className }: { className?: string }) {
  return (
    <span className={`inline-flex shrink-0 items-center justify-center overflow-hidden bg-[#0B1126] ${className ?? ""}`}>
      <img src={`${import.meta.env.BASE_URL}operator.png`} alt="" className="size-[64%] object-contain" />
    </span>
  )
}

function HistoryList({
  conversations,
  ready,
  onOpen,
}: {
  conversations: Conversation[]
  ready: boolean
  onOpen: (id: string) => void
}) {
  if (!ready) return null
  if (conversations.length === 0) {
    return <p className="text-sm text-muted-foreground">No conversations yet.</p>
  }
  return (
    <div className="flex flex-col">
      {conversations.map((conversation) => (
        <button
          key={conversation.id}
          type="button"
          className="border-b py-3 text-left last:border-b-0 hover:bg-muted/60"
          onClick={() => onOpen(conversation.id)}
        >
          <p className="line-clamp-2 text-sm">{conversation.preview || "New conversation"}</p>
          <p className="mt-1 text-xs text-muted-foreground">
            {formatWhen(conversation.at)}
            {conversation.running ? " · Running" : ""}
          </p>
        </button>
      ))}
    </div>
  )
}

function ModelPicker({
  model,
  catalog,
  favorites,
  onChoose,
  onToggleFavorite,
}: {
  model: string
  catalog: CatalogModel[]
  favorites: string[]
  onChoose: (id: string) => void
  onToggleFavorite: (id: string) => void
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState("")
  const rows = useMemo(() => {
    const needle = query.trim().toLowerCase()
    const known = new Map(catalog.map((item) => [item.id, item]))
    for (const id of favorites) {
      if (!known.has(id)) known.set(id, { id, name: id })
    }
    const matches = [...known.values()].filter((item) => {
      if (!needle) return true
      return item.name.toLowerCase().includes(needle) || item.id.toLowerCase().includes(needle)
    })
    matches.sort((left, right) => {
      const leftFavorite = favorites.includes(left.id) ? 0 : 1
      const rightFavorite = favorites.includes(right.id) ? 0 : 1
      if (leftFavorite !== rightFavorite) return leftFavorite - rightFavorite
      return left.name.localeCompare(right.name)
    })
    return matches
  }, [catalog, favorites, query])

  return (
    <Popover.Root
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) setQuery("")
      }}
    >
      <Popover.Trigger asChild>
        <button
          type="button"
          aria-label="Model"
          className="flex h-7 max-w-40 items-center truncate rounded-md px-2 text-xs text-muted-foreground hover:bg-muted"
        >
          {modelLabel(model)}
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="start"
          sideOffset={8}
          className="z-50 flex max-h-80 w-72 flex-col overflow-hidden rounded-lg bg-popover text-popover-foreground shadow-md ring-1 ring-foreground/10"
        >
          <input
            value={query}
            placeholder="Search OpenRouter"
            className="border-b bg-transparent px-3 py-2 text-sm outline-none placeholder:text-muted-foreground"
            onChange={(event) => setQuery(event.target.value)}
          />
          <div className="min-h-0 flex-1 overflow-auto p-1">
            {rows.length === 0 && <p className="px-2 py-2 text-sm text-muted-foreground">No models</p>}
            {rows.map((item) => {
              const favorite = favorites.includes(item.id)
              return (
                <div key={item.id} className={`flex items-center rounded-md ${item.id === model ? "bg-muted" : ""}`}>
                  <button
                    type="button"
                    className="min-w-0 flex-1 truncate px-2 py-1.5 text-left text-sm"
                    onClick={() => {
                      onChoose(item.id)
                      setOpen(false)
                    }}
                  >
                    {item.name}
                  </button>
                  <button
                    type="button"
                    aria-label={favorite ? `Unfavorite ${item.name}` : `Favorite ${item.name}`}
                    aria-pressed={favorite}
                    className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted"
                    onClick={() => onToggleFavorite(item.id)}
                  >
                    <Star className={`size-3.5 ${favorite ? "fill-foreground text-foreground" : ""}`} />
                  </button>
                </div>
              )
            })}
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  )
}

function modelLabel(id: string) {
  if (!id) return "Model"
  return id.split("/").pop() || id
}

function formatWhen(ms: number) {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  })
}

function MessageView({ message }: { message: Message }) {
  if (message.role === "tool") {
    return <p className="text-xs text-muted-foreground">{message.name || "tool"}</p>
  }
  if (message.role === "user") {
    return (
      <div className="ml-8 rounded-xl bg-muted px-3 py-2 text-sm">
        {message.images.length > 0 && (
          <div className="mb-2 flex flex-wrap gap-2">
            {message.images.map((url, index) => (
              <img key={index} src={url} alt="" className="size-16 rounded-md object-cover" />
            ))}
          </div>
        )}
        {message.text && <p className="whitespace-pre-wrap">{message.text}</p>}
      </div>
    )
  }
  if (!message.text.trim()) return null
  return (
    <div className="mr-4 flex gap-2 text-sm leading-relaxed">
      <OperatorMark className="mt-0.5 size-5 rounded" />
      <AssistantText text={message.text} />
    </div>
  )
}

function Thinking() {
  return (
    <div className="mr-4 flex items-center gap-2" aria-label="Thinking">
      <OperatorMark className="size-5 rounded" />
      <span className="flex gap-1">
        <span className="size-1.5 animate-bounce rounded-full bg-muted-foreground [animation-delay:0ms]" />
        <span className="size-1.5 animate-bounce rounded-full bg-muted-foreground [animation-delay:150ms]" />
        <span className="size-1.5 animate-bounce rounded-full bg-muted-foreground [animation-delay:300ms]" />
      </span>
    </div>
  )
}

function AssistantText({ text }: { text: string }) {
  return (
    <div className="min-w-0 flex-1 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-4 [&_p+p]:mt-2 [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-4">
      <Markdown
        skipHtml
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children }) => (
            <a href={href} target="_blank" rel="noreferrer" className="underline underline-offset-2">
              {children}
            </a>
          ),
          strong: ({ children }) => <strong className="font-semibold">{children}</strong>,
          em: ({ children }) => <em className="italic">{children}</em>,
          table: ({ children }) => (
            <div className="my-2 overflow-x-auto">
              <table className="w-full border-collapse text-xs">{children}</table>
            </div>
          ),
          th: ({ children }) => <th className="border px-2 py-1 text-left font-medium">{children}</th>,
          td: ({ children }) => <td className="border px-2 py-1 align-top">{children}</td>,
          img: () => null,
        }}
      >
        {text}
      </Markdown>
    </div>
  )
}
