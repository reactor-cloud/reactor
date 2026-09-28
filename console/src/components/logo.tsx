export function Logo({ className = "size-8", expanded = false }: { className?: string; expanded?: boolean }) {
  return (
    <span className="flex min-w-0 items-center gap-2">
      <img src={`${import.meta.env.BASE_URL}reactor-logo.svg`} alt="" className={`${className} shrink-0 rounded-md`} />
      {expanded && <span className="truncate font-medium tracking-tight">Reactor</span>}
    </span>
  )
}
