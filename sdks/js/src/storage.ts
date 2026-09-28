import { request } from "./http.ts"

export function createStorage(base: string, token: () => string, doFetch: typeof fetch) {
  return {
    from(bucket: string) {
      return {
        async upload(path: string, body: BodyInit, options?: { contentType?: string }): Promise<void> {
          const presign = await request(doFetch, `${base}/storage/v1/object/presign`, {
            method: "POST",
            token: token(),
            json: { bucket, key: path, method: "PUT" },
          })
          const put = await doFetch(presign.body.url, {
            method: "PUT",
            headers: { "content-type": options?.contentType ?? "application/octet-stream" },
            body,
          })
          if (!put.ok) throw new Error(await put.text())
        },

        async download(path: string): Promise<Uint8Array> {
          const presign = await request(doFetch, `${base}/storage/v1/object/presign`, {
            method: "POST",
            token: token(),
            json: { bucket, key: path, method: "GET" },
          })
          const get = await doFetch(presign.body.url)
          if (!get.ok) throw new Error(await get.text())
          return new Uint8Array(await get.arrayBuffer())
        },
      }
    },
  }
}
