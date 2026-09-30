export function Logo({ className = "size-8", expanded = false }: { className?: string; expanded?: boolean }) {
  return (
    <span className="flex min-w-0 items-center gap-2">
      <span className={`inline-flex shrink-0 overflow-hidden rounded-[12px] bg-[#070B15] ${className}`}>
        <img src={`${import.meta.env.BASE_URL}reactor-logo.svg`} alt="" className="size-full" />
      </span>
      {expanded && <span className="truncate font-medium tracking-tight">Reactor</span>}
    </span>
  )
}
