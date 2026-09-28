const raw = await Bun.stdin.text()
let req = {}
try { req = raw ? JSON.parse(raw) : {} } catch {}
const caller = JSON.parse(process.env.REACTOR_CALLER || "{}")
process.stdout.write(JSON.stringify({ ok: true, caller, req, banner: process.env.SITE_BANNER || null }))
