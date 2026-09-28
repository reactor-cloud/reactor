import { request } from "./http.ts"

export function createFunctions(base: string, token: () => string, doFetch: typeof fetch) {
  return {
    async invoke(name: string, options?: { body?: unknown }): Promise<any> {
      const { body } = await request(doFetch, `${base}/fn/v1/${name}`, {
        method: "POST",
        token: token(),
        json: options?.body ?? {},
      })
      return body
    },
  }
}
