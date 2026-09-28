const http = require("http")
const fs = require("fs")
const path = require("path")

const root = process.cwd()
const types = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".webp": "image/webp",
  ".gif": "image/gif",
  ".ico": "image/x-icon",
  ".woff2": "font/woff2",
  ".json": "application/json",
  ".txt": "text/plain; charset=utf-8",
  ".xml": "application/xml",
  ".map": "application/json",
}

function fileFor(urlPath) {
  let rel = decodeURIComponent((urlPath || "/").split("?")[0]).replace(/^\/+/, "")
  if (rel.includes("..")) return null
  if (rel === "" || rel.endsWith("/")) rel += "index.html"
  else if (!path.extname(rel)) rel = path.join(rel, "index.html")
  const full = path.resolve(root, rel)
  if (full !== root && !full.startsWith(root + path.sep)) return null
  return full
}

const server = http.createServer((req, res) => {
  if (req.method !== "GET" && req.method !== "HEAD") {
    res.writeHead(405)
    res.end()
    return
  }
  const file = fileFor(req.url)
  if (!file) {
    res.writeHead(403)
    res.end()
    return
  }
  fs.readFile(file, (err, body) => {
    if (err) {
      res.writeHead(404)
      res.end("not found")
      return
    }
    const type = types[path.extname(file).toLowerCase()] || "application/octet-stream"
    res.writeHead(200, { "content-type": type })
    res.end(req.method === "HEAD" ? undefined : body)
  })
})

server.listen(Number(process.env.PORT), "127.0.0.1")
