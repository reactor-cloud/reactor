FROM node:22-bookworm AS console
WORKDIR /ui
COPY console/package.json console/package-lock.json ./
RUN npm ci
COPY console .
RUN npm run build

FROM rust:1-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends libclang-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --release -p reactor-server -p reactor-cli -p reactor-builder \
    && mkdir -p /out \
    && cp target/release/reactor-server target/release/reactor-cli target/release/reactor-builder /out/

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl unzip procps libpq5 libatomic1 libstdc++6 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=postgrest/postgrest:v12.2.12 /bin/postgrest /usr/local/bin/postgrest
RUN curl -fsSL https://bun.sh/install | bash \
    && cp /root/.bun/bin/bun /usr/local/bin/bun
RUN curl -fsSL https://github.com/aws/aws-lambda-runtime-interface-emulator/releases/latest/download/aws-lambda-rie \
    -o /usr/local/bin/aws-lambda-rie \
    && chmod +x /usr/local/bin/aws-lambda-rie
COPY --from=build /out/reactor-server /usr/local/bin/reactor-server
COPY --from=build /out/reactor-cli /usr/local/bin/reactor-cli
COPY --from=build /out/reactor-builder /usr/local/bin/reactor-builder
COPY --from=console /usr/local/bin/node /usr/local/bin/node
RUN node -e 'process.version'
COPY --from=console /ui/dist /app/console
COPY sql /app/sql
COPY deploy/compose/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
ENV REACTOR_SQL_DIR=/app/sql
ENV REACTOR_CONSOLE_DIR=/app/console
EXPOSE 8000
ENTRYPOINT ["/entrypoint.sh"]
