import { Globe } from "lucide-react"
import type { ReactNode } from "react"

const surfaces = [
  { href: "https://github.com/reactor-cloud/reactor", label: "GitHub", icon: <GithubMark /> },
  { href: "https://www.npmjs.com/package/@reactor-cloud/client", label: "npm", icon: <NpmMark /> },
  { href: "https://discord.gg/9ZjrHwTXy", label: "Discord", icon: <DiscordMark /> },
  { href: "https://www.reactor.cloud", label: "Website", icon: <Globe className="size-4" /> },
]

export function SurfaceLinks() {
  return (
    <nav aria-label="Reactor" className="flex items-center">
      {surfaces.map((surface) => (
        <a
          key={surface.label}
          href={surface.href}
          target="_blank"
          rel="noreferrer"
          title={surface.label}
          aria-label={surface.label}
          className="flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          {surface.icon}
        </a>
      ))}
    </nav>
  )
}

function Mark({ children }: { children: ReactNode }) {
  return (
    <svg viewBox="0 0 24 24" className="size-4" fill="currentColor" aria-hidden="true">
      {children}
    </svg>
  )
}

function GithubMark() {
  return (
    <Mark>
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </Mark>
  )
}

function NpmMark() {
  return (
    <Mark>
      <path d="M0 7.334v8h6.666v1.332H12v-1.332h12v-8H0zm6.666 6.664H5.334v-4H3.999v4H1.335V8.667h5.331v5.331zm4 0v-4H8.665v4H6.666V8.667h4v5.331zm9.334-4h-1.334v4h-1.332v-4h-1.334v4h-1.332V8.667h5.332v1.331z" />
    </Mark>
  )
}

function DiscordMark() {
  return (
    <Mark>
      <path d="M19.27 5.33C17.94 4.71 16.5 4.26 15 4a.1.1 0 0 0-.1.05c-.18.33-.39.76-.53 1.09a16.1 16.1 0 0 0-4.74 0c-.14-.34-.35-.76-.54-1.09a.1.1 0 0 0-.1-.05c-1.5.26-2.93.71-4.27 1.33a.09.09 0 0 0-.04.03C2.44 9.05 1.64 12.65 1.9 16.2c0 .03.02.05.04.06 1.8 1.32 3.53 2.12 5.24 2.65.04 0 .08-.01.1-.05.4-.55.76-1.13 1.07-1.74a.1.1 0 0 0-.05-.14 11 11 0 0 1-1.57-.75.1.1 0 0 1-.01-.16c.11-.08.21-.17.31-.25a.1.1 0 0 1 .1-.01c3.3 1.51 6.87 1.51 10.13 0a.1.1 0 0 1 .1.01c.1.09.21.17.32.26a.1.1 0 0 1 0 .16c-.5.29-1.02.54-1.57.75a.1.1 0 0 0-.05.14c.32.61.68 1.19 1.07 1.74.02.04.06.05.1.05 1.72-.53 3.45-1.33 5.25-2.65a.1.1 0 0 0 .04-.06c.31-4.07-.53-7.64-2.24-10.84a.08.08 0 0 0-.04-.03ZM8.52 14.1c-.99 0-1.8-.9-1.8-2.02 0-1.11.79-2.02 1.8-2.02s1.82.91 1.8 2.02c0 1.11-.8 2.02-1.8 2.02Zm6.97 0c-.99 0-1.8-.9-1.8-2.02 0-1.11.79-2.02 1.8-2.02s1.82.91 1.8 2.02c0 1.11-.79 2.02-1.8 2.02Z" />
    </Mark>
  )
}
