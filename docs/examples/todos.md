---
title: Todos
description: The example app. A static site, a locked-down table, and one function.
---

The todos example is the smallest app that uses auth, data, and a function on one project host. It lives at `examples/todos`. After you link that directory and run `reactor deploy`, the site, the table, and `ping` are the same project.

## Layout

```text
examples/todos/
  reactor.toml          # url and ref, after link
  .reactor/service_key  # written by the CLI, not committed
  site/                 # index.html, css, and the page script
  functions/ping/       # index.ts
  sql lives in sql/project/002_todos.sql
```

Project migrations are the server's `sql/project/` directory, not a folder inside the example. `002_todos.sql` creates `todos` with forced row-level security. A row is visible to `authenticated` only when `user_id` matches the token's `sub`. `anon` is not granted. `service` is granted and bypasses the policy.

## Deploy

Link the directory, or create the project with `--link` from this directory, then:

```sh
reactor deploy
```

Open `http://{ref}.apps.localhost:18000/`. The page embeds the anon key and calls `createClient(window.location.origin, anonKey)`.

Signup and login post to `/auth/v1` on that origin. The session is stored in `localStorage`. The list is:

```js
reactor.from("todos").select("*").eq("user_id", session.user.id).order("created_at", { ascending: false })
```

Insert sends `title` and `user_id`. Update and delete filter on `id`. The policy still checks `user_id`, so a user cannot write someone else's row by changing the filter.

The page also posts to `/fn/v1/ping`. That function reads stdin and `REACTOR_CALLER` and writes JSON to stdout:

```json
{
  "ok": true,
  "caller": { "sub": "…", "ref": "…", "role": "authenticated" },
  "req": {},
  "banner": null
}
```

`banner` is set only when `SITE_BANNER` is one of the function's own variables. A site variable is not copied into the function. A todo row the page inserts comes back from PostgREST as `{ "id", "title", "user_id", "created_at" }` when the insert asks for the representation. Another user's token gets a PostgREST error, not that row.

## What to look at afterward

Auth calls, the data call, the function call, and the page load each append a console log. The function page shows the invoke. The site page shows the document request. Two users on the same project do not see each other's todos. Two projects do not see each other's rows, objects, or functions.
