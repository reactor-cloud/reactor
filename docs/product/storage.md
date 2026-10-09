---
title: Storage
description: Presigned uploads and downloads. The server does not proxy file bytes.
---

Storage is files for one project. Objects live in a filesystem or in S3. The key Reactor authorizes is `{ref}/{bucket}/{key}`. A token for project A cannot presign a key under project B.

The server checks the project, then returns a URL. The client uploads or downloads on that URL. That keeps Lambda's body limit and the bandwidth bill off the API. The JavaScript client’s `storage.from(bucket).upload` does this presign and PUT for you.

## Presign

`POST /storage/v1/object/presign`

```json
{ "bucket": "files", "key": "notes/hello.txt", "method": "PUT" }
```

`method` defaults to `PUT`. Use `GET` for a download. The response:

```json
{ "url": "https://...", "key": "<ref>/files/notes/hello.txt" }
```

Without a bucket row, the URL is valid for 300 seconds. Send `expires_in` to ask for a shorter or longer URL. A PUT is capped at 300 seconds. A GET is capped at 3600. `PUT` the bytes to `url` with a `Content-Type`. `GET` the URL to download. A bad signature is 403:

```json
{ "error": "bad signature" }
```

A key that escapes the project is also 403, `{ "error": "invalid object key" }`. An expired URL is rejected the same way.

Keys that contain `..` or escape the project prefix are refused. `_functions` and `_sites` are refused. Those prefixes hold function zips and site files.

## Buckets

A bucket is a row in `storage_buckets` in the project schema. `storage_objects` records each object and its owner. Both tables force row-level security and start with no allow policy. Until a bucket row exists, presign keeps the legacy behavior above.

`POST /storage/v1/bucket` with a service key creates one:

```json
{ "name": "photos", "public": true }
```

A private bucket presigns only when a policy lets the caller `SELECT` or `INSERT` that object row. The server sets `request.jwt.claims` and the caller's role for that check. An owner policy looks like this:

```sql
CREATE POLICY storage_owner ON storage_objects
  FOR ALL TO authenticated
  USING (owner::text = current_setting('request.jwt.claims', true)::json->>'sub')
  WITH CHECK (owner::text = current_setting('request.jwt.claims', true)::json->>'sub');
```

A public bucket serves `GET /storage/v1/object/public/{bucket}/{key}` with no token and no expiry. The response sets `Cache-Control: public, max-age=86400`. The process caches the public flag for a few seconds and does not hold a database connection while it reads the object. Writes to a public bucket still presign, and still need an `INSERT` policy.

`GET /storage/v1/bucket/{name}` returns `public_url_base` when the bucket is public. The client joins the object name onto that base. Set `storage.cdn_public_base` when a CDN should be that host. Leave it empty and the host is the project host, `{ref}.{base_domain}`. The blob bucket itself stays private on Fly Tigris, AWS S3, and local rustfs. Point Cloudflare or CloudFront at `/storage/v1/object/public/*` on the project host and honor `Cache-Control`. A cache miss reaches Reactor. The next request is served from the edge.

## Backends

| `storage.backend` | Behavior |
| --- | --- |
| `fs` | Files under `storage.fs_root`. The signed URL points back at `/storage/v1/signed` on the server. |
| `s3` | A bucket, region, and endpoint. Local Compose uses an S3-compatible store. Set `storage.public_endpoint` when clients cannot reach the internal endpoint. |

S3 settings (`bucket`, `region`, `endpoint`, `access_key`, `secret_key`, `public_endpoint`) live under `[storage]` when the backend is `s3`. On AWS, the task role can be used when static keys are not set.

## Clients

```js
await reactor.storage.from("files").upload("notes/hello.txt", body, {
  contentType: "text/plain",
})
const bytes = await reactor.storage.from("files").download("notes/hello.txt")
await reactor.storage.createBucket("photos", { public: true })
const url = await reactor.storage.from("photos").getPublicUrl("notes/hello.txt")
```

The Swift client has the same `upload` and `download` on a bucket.

## Console

The console lists blob keys under `{ref}/`, skipping `_functions` and `_sites`. Upload, open, download, and delete from the storage page. That page uses the operator's authority, not a user's row policy. Create a bucket there, mark it public, and copy the stable URL from the file details. A private or legacy file still opens through a signed URL.

`reactor storage` lists objects. `reactor storage buckets` lists buckets. `reactor storage buckets create photos --public` creates one. `reactor storage url photos notes/hello.txt` prints the URL.
