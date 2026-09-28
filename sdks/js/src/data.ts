import { PostgrestClient } from "@supabase/postgrest-js"

export function createData(base: string, token: string, doFetch: typeof fetch) {
  return new PostgrestClient(`${base}/data/v1`, {
    headers: { Authorization: `Bearer ${token}` },
    fetch: doFetch,
  })
}
