export class ReactorError extends Error {
  status: number

  constructor(message: string, status: number) {
    super(message)
    this.name = "ReactorError"
    this.status = status
  }
}

export async function request(
  doFetch: typeof fetch,
  url: string,
  options: { method?: string; token?: string; json?: unknown; body?: BodyInit; contentType?: string },
): Promise<{ status: number; body: any }> {
  const headers = new Headers()
  if (options.token) headers.set("authorization", `Bearer ${options.token}`)
  let body: BodyInit | undefined = options.body
  if (options.json !== undefined) {
    headers.set("content-type", "application/json")
    body = JSON.stringify(options.json)
  } else if (options.contentType) {
    headers.set("content-type", options.contentType)
  }
  const response = await doFetch(url, { method: options.method ?? "GET", headers, body })
  const text = await response.text()
  const parsed = text ? JSON.parse(text) : null
  if (!response.ok) {
    const message = parsed?.error || parsed?.message || text || response.statusText
    throw new ReactorError(message, response.status)
  }
  return { status: response.status, body: parsed }
}
