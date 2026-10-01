# Clusters

How to create a Reactor cluster on AWS or Fly, and how to roll a new image onto one that is already serving. The GitHub tag and the public repos are a separate step: [github-release.md](github-release.md).

The lab tree `v2/` is the source. Build the image from that directory. `deploy/aws/.state/` and `deploy/fly/.state/` hold cluster secrets. Do not commit them, and do not print them.

`GET /health` is an empty 200 when Postgres answers `SELECT 1`. It does not check the blob store.

## Live clusters

| Name | Where | Public URL | How it is named |
| --- | --- | --- | --- |
| aws1 | CloudFormation stack `reactor-v2` in `eu-central-1` | `https://aws1.reactor.cloud` | AWS profile `relai-admin`, account `968517329789` |
| sw1 | CloudFormation stack `reactor-sw1` in `eu-central-1` | `https://sw1.reactor.cloud` | Same account and ECR repository as aws1 |
| fly1 | Fly app `reactor-v2` in org `reactor-890`, region `fra` | `https://fly1.reactor.cloud` | `deploy/fly/fly.toml` |

Both AWS stacks pull `968517329789.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2`. They do not share a base domain. The default AWS CLI profile is not valid for this account. Pass `--profile relai-admin`.

The image is linux/arm64. Lambda rejects an OCI index that includes an attestation manifest. A pushed tag whose manifest is about 856 bytes and contains a `manifests` array is that index. Replace the tag with the arm64 child before CloudFormation uses it.

## Image

From `v2/`:

```sh
docker build --platform linux/arm64 -t reactor-v2:TAG -f Dockerfile .
```

`TAG` matches the git tag without the leading `v`, for example `1.26.09-beta.4`. Docker is `/usr/local/bin/docker` when it is missing from `PATH`.

Log in and push:

```sh
aws ecr get-login-password --profile relai-admin --region eu-central-1 \
  | docker login --username AWS --password-stdin 968517329789.dkr.ecr.eu-central-1.amazonaws.com
docker tag reactor-v2:TAG 968517329789.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2:TAG
docker push 968517329789.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2:TAG
```

Create the repository first if this account does not have it yet: `aws ecr create-repository --repository-name reactor-v2 --region eu-central-1`.

Then confirm the tag is a single image. Fetch the manifest. If it has `manifests`, delete the tag and put the linux/arm64 child back:

```sh
aws ecr batch-get-image --profile relai-admin --region eu-central-1 \
  --repository-name reactor-v2 --image-ids imageTag=TAG \
  --accepted-media-types application/vnd.oci.image.index.v1+json \
  --query 'images[0].imageManifest' --output text
```

The arm64 entry has `"platform": {"os": "linux", "architecture": "arm64"}`. The other entry is the attestation (`unknown` / `unknown`). Delete `imageTag=TAG`, `batch-get-image` that arm64 digest as `application/vnd.oci.image.manifest.v1+json`, and `aws ecr put-image --image-tag TAG --image-manifest file://...`. Read the tag again. It must have `layers` and no `manifests` key.

`docker buildx build --provenance=false --sbom=false` is the other way to avoid the index. Still check the manifest. A tag that still points at an index will fail the Lambda update.

## Update an AWS cluster

Do this for each stack. aws1 is `reactor-v2`. sw1 is `reactor-sw1`. Read the parameter names from the stack. Do not assume the two stacks have the same set.

```sh
aws cloudformation describe-stacks --profile relai-admin --region eu-central-1 \
  --stack-name STACK --query 'Stacks[0].Parameters[].ParameterKey' --output text
```

Update only `ImageUri`. Every other parameter is `UsePreviousValue=true`.

```sh
aws cloudformation update-stack --profile relai-admin --region eu-central-1 \
  --stack-name STACK \
  --use-previous-template \
  --capabilities CAPABILITY_IAM CAPABILITY_NAMED_IAM CAPABILITY_AUTO_EXPAND \
  --parameters \
    ParameterKey=ImageUri,ParameterValue=968517329789.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2:TAG \
    ParameterKey=OTHER,UsePreviousValue=true
```

`CAPABILITY_NAMED_IAM` is required. Without it the call returns `InsufficientCapabilitiesException`.

Wait until the status is `UPDATE_COMPLETE`:

```sh
aws cloudformation wait stack-update-complete --profile relai-admin --region eu-central-1 --stack-name STACK
```

Then check:

- `GET https://HOST/health` is 200.
- The Lambda's image URI is the new tag. The function name is the stack's `FunctionName` parameter, not always `reactor-v2`.
- The ECS service's task definition image is the new tag. Find the cluster and service with `aws ecs list-clusters` and `list-services` if the name is not already known. aws1's site cluster starts with `reactor-v2-SiteCluster-`.

Pushing a new image onto a tag that a running task definition already uses does not start new tasks. Changing `ImageUri` on the stack does.

A new image does not replace site files. Those are objects in the bucket. Redeploy a site only when its files changed.

## New AWS cluster

One account can hold more than one stack. The second stack must set its own `FunctionName`, `LayerName`, `LogGroupName`, and `TaskFamily`. The defaults are aws1's names. Reusing them replaces the live Lambda, log group, or task family. IAM, the VPC, Aurora, the bucket, and the load balancer get generated names. Two stacks may share `JwtSecretName` and `AgentSecretName`. They must not share `BaseDomain`.

The template is `deploy/lambda/template.yaml`. It expects the JWT secret and the OpenRouter secret to exist already. It creates the database password, the authenticator password, and the operator token. `JwtSecretName` is JSON with `private_pem` (Ed25519 PKCS#8) and `public_jwk`. `AgentSecretName` is JSON with `api_key`. The stack requires the agent secret even when the agent is unused.

`deploy/lambda/build-bun-layer.sh` fills `deploy/lambda/bun-layer/` with the linux aarch64 Bun binary. That directory is gitignored. User functions on AWS run from that layer.

From `deploy/lambda`, with the SAM CLI:

```sh
sam deploy \
  --template-file template.yaml \
  --stack-name NEW_STACK \
  --region eu-central-1 \
  --profile relai-admin \
  --capabilities CAPABILITY_IAM CAPABILITY_NAMED_IAM CAPABILITY_AUTO_EXPAND \
  --image-repository 968517329789.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2 \
  --resolve-s3 \
  --no-confirm-changeset \
  --parameter-overrides \
    ImageUri=968517329789.dkr.ecr.eu-central-1.amazonaws.com/reactor-v2:TAG \
    FunctionName=NEW_STACK \
    LayerName=NEW_STACK-bun \
    LogGroupName=/reactor/NEW_STACK \
    TaskFamily=NEW_STACK-site \
    BaseDomain=NEW.reactor.cloud \
    PublicUrl=https://pending.invalid \
    AgentSecretName=reactor-v2/openrouter \
    JwtSecretName=reactor-v2/jwt \
    EngineVersion=16.8
```

Leave `CertificateArn` empty until ACM has issued a certificate. An empty certificate serves HTTP on port 80 only.

Outputs are `ApiUrl`, `AlbDns`, and `BunLayerArn`. `GET /health` on the API URL and on the load balancer is 200 once Postgres answers. `GET /console/v1/setup` is `{"needs_setup":true}` until the first operator exists.

The first Fargate tasks can exit with `Name or service not known` while Aurora's hostname is still propagating. The service retries. Aurora's master user cannot `ALTER ROLE authenticator SET pgrst.db_schemas`. The server logs that warning and writes the schema list into the PostgREST config. That path still sends `NOTIFY pgrst`.

Request an ACM certificate in `eu-central-1` for the console host and the site wildcard, for example `sw1.reactor.cloud` and `*.sw1.reactor.cloud`. Validation is a DNS CNAME. In Cloudflare, create that CNAME with the proxy off (`flarectl`, token `CF_API_TOKEN` from the shell environment). Do not print the token. After the certificate is `ISSUED`, attach it by updating the stack with `CertificateArn`, `BaseDomain`, and `PublicUrl=https://HOST`. Pass every other parameter the stack already has, including the four unique names. Add grey-cloud CNAMEs for the host and `*.host` to `AlbDns`. Proxy stays off so TLS terminates on the load balancer and the `Host` header selects the project.

`deploy/aws/prove.sh` checks a stack after the image is up. It reads `deploy/aws/.state/cluster.env` and does not deploy.

Idle cost in `eu-central-1` is about $145 per month before user-function Lambdas. Delete a stack you created with `aws cloudformation delete-stack`. Deleting a stack that is serving sites deletes its database and bucket.

## Update fly1

From `v2/`, with `fly` already signed in:

```sh
fly deploy . --config deploy/fly/fly.toml --ha=false
```

This builds `Dockerfile` on Fly and rolls the two machines of app `reactor-v2`. Do not run `deploy/fly/deploy.sh` for a code roll. That script bootstraps Postgres, Tigris, secrets, and the backup machine. Running it against fly1 is a no-op where those objects exist, and it exits if `REACTOR_AGENT__API_KEY` is unset or if the Postgres secret exists while `deploy/fly/.state/cluster.env` is missing.

Fly prints `expected 1 AAAA records for reactor-v2.fly.dev` and still exits 0. The deploy succeeded when both machines reach a good state. Confirm `GET https://fly1.reactor.cloud/health` is 200. The hostname that matters is `fly1.reactor.cloud`, which CNAMEs to the Fly app. `reactor-v2.fly.dev` is the app hostname Fly checks.

Do not run `fly secrets deploy` to pick up staged secrets. Secrets already on the app stay in place across `fly deploy`.

## New Fly cluster

`deploy/fly/deploy.sh` and `deploy/fly/fly.toml` are fly1. The script's app names are `reactor-v2`, `reactor-v2-db`, and `reactor-v2-backup` in org `reactor-890`, region `fra`. Running it does not create a second cluster.

For a new cluster, copy `deploy/fly/fly.toml` and `deploy/fly/deploy.sh` and change:

- `app` in the toml, and `APP`, `DB`, and `BACKUP` in the script
- `REACTOR_RUNTIME__BASE_DOMAIN` and `REACTOR_HTTP__PUBLIC_URL`
- the Tigris storage name (`fly storage create --name`)
- the state directory, so the new cluster does not read fly1's `cluster.env`

Export `REACTOR_AGENT__API_KEY`, then run the new script from `v2/`. It creates the Postgres app and volume, waits for the TCP check, creates Tigris on first run, and stages app secrets only when `REACTOR_DATABASE__URL` is not already set. It writes passwords and the operator token into the new state directory. It does not deploy the server image. The last line tells you to run `fly deploy`.

Point DNS at the new app only after `GET /health` on the Fly hostname is 200. Add the public hostname as a certificate on that Fly app before switching the CNAME. Leave fly1's DNS and apps alone.

A second Fly cluster still uses one listen-mode process. Functions stay on Bun in that process (`REACTOR_FUNCTIONS__PUBLISHER=fake`). AWS is the layout that publishes each user function as its own Lambda.
