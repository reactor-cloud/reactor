---
title: Lambda
description: The AWS template. One API Lambda, Aurora, S3, and a site service.
---

`v2/deploy/lambda/template.yaml` is the AWS layout. It is one API Lambda, a private Aurora Postgres, an S3 bucket, and a Fargate service for site processes.

The template's description is the contract: Reactor on one Lambda, private Aurora Serverless v2, S3, and a Fargate site service.

## The API Lambda

The function is the Reactor image, `PackageType: Image`, arm64, behind an HTTP API that forwards `ANY /` and `ANY /{proxy+}`.

It runs with `REACTOR_RUNTIME__MODE=lambda`. The database URL uses the Aurora endpoint and `sslmode=require`. The JWT private key and the operator token come from Secrets Manager. Storage is the stack's bucket in the same region. PostgREST is expected on `127.0.0.1:3000` inside that environment. The console files are at `/app/console`.

`PublicUrl`, `BaseDomain`, and `CertificateArn` are parameters. The committed defaults are placeholders. Set them before you treat the stack as a real host.

## User functions

The binary can publish a user function as its own Lambda when `functions.runtime` is `lambda` and `functions.publisher` is `aws`. Invoke then calls that function. The API process does not run the user's code.

The template ships with `functions.runtime=bun` and `functions.publisher=fake`. In that mode, invoke still runs Bun inside the API Lambda, the same way Compose does. A Bun layer and an IAM role for user functions are in the template so a real publisher has something to attach. Switching the publisher is a configuration change, not a new client API.

Presigned S3 URLs are the upload path either way.

## Sites

Static files are objects in the bucket. A deployment with a command is not run inside the API Lambda. The template runs a Fargate service behind a load balancer for that work. The API Lambda still authorizes and records the deployment.

## What stays the same

Project resolution, tokens, schemas, grants, the PostgREST pre-request, and the blob key layout. A client that works against Compose works against this stack without a new SDK.

`REACTOR_HANDLER` can still limit a process to `auth`, `storage`, `sites`, `fn`, or `platform`. The template does not split the API that way. It sends every path to one function.
