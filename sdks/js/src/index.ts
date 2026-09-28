import { createAuth, type Session } from "./auth.ts"
import { createData } from "./data.ts"
import { createFunctions } from "./functions.ts"
import { createStorage } from "./storage.ts"

export type { Session, User } from "./auth.ts"
export { ReactorError } from "./http.ts"

export type ReactorOptions = {
  fetch?: typeof fetch
  session?: Session | null
}

export function createClient(url: string, anonKey: string, options: ReactorOptions = {}) {
  const base = url.replace(/\/$/, "")
  const doFetch = options.fetch ?? globalThis.fetch
  const session = { current: options.session ?? null }
  const token = () => session.current?.access_token ?? anonKey

  return {
    auth: createAuth(base, anonKey, doFetch, session),
    from(table: string) {
      return createData(base, token(), doFetch).from(table)
    },
    rpc(fn: string, args: Record<string, unknown> = {}) {
      return createData(base, token(), doFetch).rpc(fn, args)
    },
    storage: createStorage(base, token, doFetch),
    functions: createFunctions(base, token, doFetch),
  }
}
