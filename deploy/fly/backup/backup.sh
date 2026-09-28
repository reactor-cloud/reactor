#!/bin/sh
set -eu
: "${DATABASE_URL:?DATABASE_URL is required}"
: "${BUCKET_NAME:?BUCKET_NAME is required}"
: "${AWS_ACCESS_KEY_ID:?AWS_ACCESS_KEY_ID is required}"
: "${AWS_SECRET_ACCESS_KEY:?AWS_SECRET_ACCESS_KEY is required}"
endpoint="${AWS_ENDPOINT_URL_S3:-https://t3.storage.dev}"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
key="backups/reactor-${stamp}.sql.gz"
export AWS_REQUEST_CHECKSUM_CALCULATION=when_required
export AWS_RESPONSE_CHECKSUM_VALIDATION=when_required
export AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-auto}"
pg_dump --no-owner --no-acl "$DATABASE_URL" | gzip | aws s3 cp - "s3://${BUCKET_NAME}/${key}" \
  --endpoint-url "$endpoint" \
  --region "$AWS_DEFAULT_REGION"
echo "wrote ${key}"
