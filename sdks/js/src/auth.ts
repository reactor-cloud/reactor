import { request } from "./http.ts"

export type User = { id: string; email: string }

export type Session = {
  access_token: string
  refresh_token: string
  user: User
}

export function createAuth(base: string, anonKey: string, doFetch: typeof fetch, session: { current: Session | null }) {
  async function save(next: Session): Promise<Session> {
    session.current = next
    return next
  }

  return {
    getSession(): Session | null {
      return session.current
    },

    async signUp(credentials: { email: string; password: string }): Promise<Session> {
      const { body } = await request(doFetch, `${base}/auth/v1/signup`, {
        method: "POST",
        token: anonKey,
        json: { email: credentials.email, password: credentials.password },
      })
      return save(body as Session)
    },

    async signInWithPassword(credentials: { email: string; password: string }): Promise<Session> {
      const { body } = await request(doFetch, `${base}/auth/v1/token`, {
        method: "POST",
        token: anonKey,
        json: { email: credentials.email, password: credentials.password },
      })
      return save(body as Session)
    },

    async signInWithMagicLink(email: string): Promise<void> {
      await request(doFetch, `${base}/auth/v1/magic-link`, {
        method: "POST",
        token: anonKey,
        json: { email },
      })
    },

    async verifyMagicLink(token: string): Promise<Session> {
      const { body } = await request(doFetch, `${base}/auth/v1/verify`, {
        method: "POST",
        token: anonKey,
        json: { token },
      })
      return save(body as Session)
    },

    async recover(email: string): Promise<void> {
      await request(doFetch, `${base}/auth/v1/recover`, {
        method: "POST",
        token: anonKey,
        json: { email },
      })
    },

    async completeRecovery(token: string, password: string): Promise<Session> {
      const { body } = await request(doFetch, `${base}/auth/v1/recover/complete`, {
        method: "POST",
        token: anonKey,
        json: { token, password },
      })
      return save(body as Session)
    },

    async invite(email: string, serviceKey: string): Promise<void> {
      await request(doFetch, `${base}/auth/v1/invite`, {
        method: "POST",
        token: serviceKey,
        json: { email },
      })
    },

    async acceptInvite(token: string, password: string): Promise<Session> {
      const { body } = await request(doFetch, `${base}/auth/v1/invite/accept`, {
        method: "POST",
        token: anonKey,
        json: { token, password },
      })
      return save(body as Session)
    },

    async getUser(): Promise<User> {
      const token = session.current?.access_token
      if (!token) throw new Error("not signed in")
      const { body } = await request(doFetch, `${base}/auth/v1/user`, { token })
      return body as User
    },

    async refreshSession(): Promise<Session> {
      const refresh = session.current?.refresh_token
      if (!refresh) throw new Error("not signed in")
      const { body } = await request(doFetch, `${base}/auth/v1/token`, {
        method: "POST",
        token: anonKey,
        json: { refresh_token: refresh },
      })
      return save(body as Session)
    },

    async signOut(): Promise<void> {
      const refresh = session.current?.refresh_token
      if (!refresh) return
      await request(doFetch, `${base}/auth/v1/logout`, {
        method: "POST",
        json: { refresh_token: refresh },
      })
      session.current = null
    },

    signInWithOAuth(): never {
      throw new Error("unsupported")
    },
  }
}
