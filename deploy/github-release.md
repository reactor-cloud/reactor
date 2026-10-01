# GitHub release

How to tag a Reactor version and publish the public repos. Rolling that image onto aws1, sw1, or fly1 is [clusters.md](clusters.md). A GitHub tag does not update those clusters. The release workflow pushes a GHCR image only.

The lab repo at `v2/` has no remote. `v2/scripts/sync-release.sh` is the only writer for the public checkouts under `/Users/cdelconde/Dev/Github/Reactor/`.

## Version

The git tag is `v1.26.09-beta.N`. Cargo, npm, and Maven use `1.26.9-beta.N` because semver rejects the leading zero in `09`. OpenAPI and the Swift pin use the tag text without the leading `v` when a client is part of that release.

Bump the server version in:

- `Cargo.toml` and every workspace package version in `Cargo.lock`
- `CHANGELOG.md` (a new section; leave the previous section as it was published)
- `LICENSE` (`Licensed Work`), `SECURITY.md`, `README.md`
- `openapi/openapi.yaml`
- `websites/docs/src/content/docs/index.md` and the CLI install line in `operate/cli.md`
- `.github/ISSUE_TEMPLATE/bug.yml`
- `deploy/publish/homebrew/Formula/reactor.rb` (`url` and `version`) and `deploy/publish/homebrew/README.md`

Leave the JavaScript, Swift, and Kotlin package versions on the last tag that changed those clients. Say that in the changelog and in the README. Tag a client repo only when its code changed. A license-line bump copied by the sync script does not need a new client tag.

Do not commit `docs/reactor-v2.design.md`, `docs/reactor-v2-console.design.md`, or `.cursor/`. Do not commit `.env`, `*.pem`, `deploy/aws/.state/`, `deploy/fly/.state/`, `reactor.toml`, or `.reactor/`.

Commit on lab `main`, then `git tag v1.26.09-beta.N`.

## Public repos

```sh
bash v2/scripts/sync-release.sh
```

That rsyncs `v2/` into `reactor-cloud/reactor` except websites, sdks, `deploy/publish`, and the sync script. It copies the docs markdown into `reactor/docs`. It fills `reactor-js`, `reactor-swift`, `reactor-kotlin`, and `homebrew-reactor`. It does not push.

Commit the public `reactor` checkout and tag it `v1.26.09-beta.N`. Commit client repos when the sync changed them. Commit the Homebrew tap only after the checksum below is known, or commit it twice: once for the version, once for the checksum.

## Push

The active SSH key is `cdelconde-saige` and is denied by `reactor-cloud`. Push over HTTPS as `cdelconde2`, then switch the GitHub CLI back.

```sh
gh auth switch --user cdelconde2
git -c credential.helper= -c credential.helper='!gh auth git-credential' \
  push https://github.com/reactor-cloud/reactor.git HEAD:main
git -c credential.helper= -c credential.helper='!gh auth git-credential' \
  push https://github.com/reactor-cloud/reactor.git v1.26.09-beta.N
gh auth switch --user cdelconde-saige
```

Repeat the `HEAD:main` push for each client repo that has a new commit, and for `homebrew-reactor`. Push a client tag only when that repo was tagged.

`git push https://... HEAD:main` does not update the local `origin/main` ref. The next `git status` can say `ahead` even though GitHub has the commits.

## Homebrew checksum

The formula's `sha256` is the GitHub archive of the tag, not a hash of the lab tree. `deploy/publish` is excluded from the server repo, so editing the formula after the tag does not change that archive.

After the tag is on `reactor-cloud/reactor`:

```sh
curl -fsSL -o /tmp/reactor.tar.gz \
  https://github.com/reactor-cloud/reactor/archive/refs/tags/v1.26.09-beta.N.tar.gz
shasum -a 256 /tmp/reactor.tar.gz
```

Put that digest in `Formula/reactor.rb`, commit it in the lab repo and in `homebrew-reactor`, and push the tap as `cdelconde2`. Remove any `bottle` block left over from an older tag. A bottle is a separate release. `brew install` for this tag builds from source and needs Rust.

## What the tag does by itself

`.github/workflows/release.yml` runs on tags `v*` and only when `github.repository` is `reactor-cloud/reactor`. It builds the Dockerfile and pushes `ghcr.io/reactor-cloud/reactor:1.26.09-beta.N`. It does not move a `latest` tag. The lab copy of the workflow does not run, because the lab repo has no remote.

`reactor-js/.github/workflows/release.yml` publishes npm with `--tag beta` when `NPM_TOKEN` is set, and skips when it is not. Publish from a machine only when that workflow cannot. The npm scope is `@reactor-cloud/client`. A prerelease uses the `beta` dist-tag. Do not remove `latest` unless asked.

The Kotlin workflow publishes to Maven Central only when `MAVEN_CENTRAL_TOKEN` and `MAVEN_GPG_KEY` are set. It skips otherwise. The workflow does not pass `MAVEN_CENTRAL_USERNAME`. Do not tag Kotlin just to retry that publish.

Swift has no registry. The git tag on `reactor-cloud/reactor-swift` is the package version. Pin it with `exact:` because `1.26.09` is not valid semver.

## Docs site

`https://docs.reactor.cloud` is a site project on aws1, not a file in the image. A new image does not update it. After the markdown changed, build the Starlight site from `v2/websites/docs` with Bun (`bun ./node_modules/astro/bin/astro.mjs build`). Homebrew `node` is broken in this environment, and `bun run build` invokes it. Copy `dist/` to the linked project's `site/` and run `reactor deploy` from that project. The Homebrew CLI is `/opt/homebrew/bin/reactor`. An old shell can still hash `~/.cargo/bin/reactor`; `hash -r` if `--url` is rejected.

Confirm `https://docs.reactor.cloud/` contains the new version string.
