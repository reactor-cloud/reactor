---
title: Storage
description: Presigned uploads and downloads. The server does not proxy file bytes.
---

Objects live in a filesystem or in S3. The key Reactor authorizes is `{ref}/{bucket}/{key}`. A token for project A cannot presign a key under project B.

The server checks the project, then returns a URL. The client uploads or downloads on that URL. That keeps Lambda's body limit and the bandwidth bill off the API.

## Presign

`POST /storage/v1/object/presign`

```json
{ "bucket": "files", "key": "notes/hello.txt", "method": "PUT" }
```

`method` defaults to `PUT`. Use `GET` for a download. The response:

```json
{ "url": "https://...", "key": "<ref>/files/notes/hello.txt" }
```

The URL is valid for 300 seconds. `PUT` the bytes to `url` with a `Content-Type`. `GET` the URL to download. A bad signature or an expired URL is rejected.

Keys that contain `..` or escape the project prefix are refused.

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
```

The Swift client has the same `upload` and `download` on a bucket.

## Console

The console lists blob keys under `{ref}/`, skipping `_functions` and `_sites`. Those prefixes hold function zips and site files. Upload, open, download, and delete from the storage page. That page uses the operator's authority, not a user's row policy.
