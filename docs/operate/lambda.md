---
title: AWS
description: Deploy Reactor on Lambda, Aurora, and a Fargate site service.
---

`deploy/lambda/template.yaml` is the AWS layout. One Lambda serves the HTTP API. A Fargate service behind a load balancer serves the console and project sites, and it publishes user functions as their own Lambdas. Postgres is Aurora Serverless v2. Blobs are the stack's S3 bucket.

The API Lambda runs with `REACTOR_RUNTIME__MODE=lambda` and `functions.publisher=fake`. It does not run site processes or publish user functions. Its base domain is `apps.invalid`, so it never claims a project host. The Fargate task runs with `REACTOR_RUNTIME__MODE=listen`, `functions.runtime=lambda`, and `functions.publisher=aws`. Project hosts are `{ref}.{base_domain}`. The console is the base host itself, for example `aws1.reactor.cloud`, with sites at `abcd.aws1.reactor.cloud`.

Both processes are the same arm64 image. PostgREST listens on `127.0.0.1:3000` inside the container. The console files are at `/app/console`.

## Names

These names are unique in the account. The defaults are the first cluster. A second stack in the same account must override all four. Changing them on a stack that is already serving replaces the Lambda, the log group, or the task family.

| Parameter | Default | What it names |
| --- | --- | --- |
| `FunctionName` | `reactor-v2` | API Lambda |
| `LayerName` | `reactor-bun` | Bun layer for user functions |
| `LogGroupName` | `/reactor/site` | Fargate logs |
| `TaskFamily` | `reactor-site` | ECS task definition |

IAM roles, the VPC, Aurora, the bucket, and the load balancer take generated names. Two stacks can share the JWT secret and the OpenRouter secret. They should not share a base domain.

## Before the first deploy

The template expects two secrets to exist already. It creates the database password, the authenticator password, and the operator token itself.

`JwtSecretName` (default `reactor-v2/jwt`) is JSON with `private_pem` and `public_jwk`. `private_pem` is an Ed25519 PKCS#8 PEM. `public_jwk` is a JSON string `{"kty":"OKP","crv":"Ed25519","x":"…","alg":"EdDSA"}`. Compose's `keys` service writes `private.pem` and `public.jwk` in that shape.

`AgentSecretName` (default `reactor-v2/openrouter`) is JSON with `api_key`. The agent calls OpenRouter with it. The stack still requires the secret when the agent is unused.

Build the image from `v2/` as one linux/arm64 manifest. A multi-arch index is rejected by Lambda.

```sh
aws ecr create-repository --repository-name reactor-v2 --region eu-central-1
aws ecr get-login-password --region eu-central-1 \
  | docker login --username AWS --password-stdin "$ACCOUNT.dkr.ecr.eu-central-1.amazonaws.com"
docker buildx build --platform linux/arm64 --provenance=false --sbom=false \
  -t "$ACCOUNT.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2:prove" --push .
```

`deploy/lambda/build-bun-layer.sh` downloads the linux aarch64 Bun binary into `deploy/lambda/bun-layer/`. That directory is gitignored. User functions run on `provided.al2023`, arm64, with Bun at `/opt/bin/bun`.

## Deploy

From `deploy/lambda`, with the [SAM CLI](https://docs.aws.amazon.com/serverless-application-model/latest/developerguide/install-sam-cli.html):

```sh
sam deploy \
  --template-file template.yaml \
  --stack-name reactor-v2 \
  --region eu-central-1 \
  --capabilities CAPABILITY_IAM CAPABILITY_AUTO_EXPAND \
  --image-repository "$ACCOUNT.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2" \
  --resolve-s3 \
  --no-confirm-changeset \
  --parameter-overrides \
    ImageUri="$ACCOUNT.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2:prove" \
    BaseDomain=apps.example.com \
    PublicUrl=https://pending.invalid \
    AgentSecretName=reactor-v2/openrouter \
    JwtSecretName=reactor-v2/jwt \
    EngineVersion=16.8
```

Leave `CertificateArn` unset until the certificate below is issued. An empty certificate serves HTTP on port 80 only.

Outputs are `ApiUrl`, `AlbDns`, and `BunLayerArn`. `GET /health` on both URLs is an empty 200 once Postgres answers. `GET /console` on the load balancer is the console. `GET /console/v1/setup` is `{"needs_setup":true}` until the first operator is created.

The first Fargate tasks can exit with `Name or service not known` while Aurora's hostname is still propagating. The service retries. A later task logs `listening on 0.0.0.0:8000`. Aurora's master user cannot `ALTER ROLE authenticator SET pgrst.db_schemas`. The server logs that warning and writes the schema list into the PostgREST config instead. That is expected on Aurora.

Pushing a new image onto a tag that a live task definition still references does not roll that service until something starts new tasks. Deploy a live stack with an image digest when the tag must stay put.

## Domain

Request an ACM certificate in the same region as the load balancer. The names are the console host and the site wildcard, for example `aws1.reactor.cloud` and `*.aws1.reactor.cloud`. Validation is DNS. One CNAME usually covers both names.

In Cloudflare, create that validation CNAME with the proxy off. After the certificate is `ISSUED`, create two more grey-cloud CNAMEs to `AlbDns`:

| Name | Type | Content |
| --- | --- | --- |
| `aws1.reactor.cloud` | CNAME | the load balancer DNS name |
| `*.aws1.reactor.cloud` | CNAME | the load balancer DNS name |

Proxy stays off so TLS terminates on the load balancer and the `Host` header is the project host. Then update the same stack:

```sh
sam deploy \
  --template-file template.yaml \
  --stack-name reactor-v2 \
  --region eu-central-1 \
  --capabilities CAPABILITY_IAM CAPABILITY_AUTO_EXPAND \
  --image-repository "$ACCOUNT.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2" \
  --resolve-s3 \
  --no-confirm-changeset \
  --parameter-overrides \
    ImageUri="$ACCOUNT.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2:prove" \
    BaseDomain=aws1.reactor.cloud \
    PublicUrl=https://aws1.reactor.cloud \
    CertificateArn=arn:aws:acm:eu-central-1:ACCOUNT:certificate/CERT_ID \
    AgentSecretName=reactor-v2/openrouter \
    JwtSecretName=reactor-v2/jwt \
    EngineVersion=16.8
```

Pass the same `FunctionName`, `LayerName`, `LogGroupName`, and `TaskFamily` the stack already uses. Omitting them is safe only while they still match the defaults.

With a certificate, the template adds an HTTPS listener and redirects port 80 to HTTPS. `PublicUrl` is `https://aws1.reactor.cloud`, so a project site link is `https://{ref}.aws1.reactor.cloud`. The console is `https://aws1.reactor.cloud/console`. The first operator is a platform admin, so login asks for a passkey and an authenticator before it issues a session.

`flarectl` talks to Cloudflare with `CF_API_TOKEN`. The token needs DNS edit on the zone. Records for this layout are DNS-only (`--proxy` left off).

## What a second stack proved

A stack named `reactor-v2-proof`, with `FunctionName=reactor-v2-proof`, `LayerName=reactor-bun-proof`, `LogGroupName=/reactor/site-proof`, and `TaskFamily=reactor-site-proof`, reached `CREATE_COMPLETE` beside a live stack and did not change it. The proof load balancer and API both returned 200 for `/health`. `/console` returned 200, and setup reported `needs_setup: true`. A host under `apps.example.com` was routed as a site. That stack was then deleted.

Idle cost in `eu-central-1` is about $145 per month: Aurora at its 0.5 ACU floor, one NAT gateway, the load balancer, one 0.5 vCPU Fargate task, and three public IPv4 addresses. Aurora's cap in the template is 1 ACU, which adds about $50 if it stays there. User-function Lambdas are extra and appear only after a project deploy.

Delete a stack you created with `aws cloudformation delete-stack --stack-name <that-stack>`. Deleting the stack that is serving sites removes its database and bucket.
