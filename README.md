# locked-in

Supply-chain linting for package installs and lockfiles.

A committed lockfile only protects a build when every install command honors it. For example, `npm install` can update a lockfile when `package.json` and `package-lock.json` disagree; [`npm ci`](https://docs.npmjs.com/cli/commands/npm-ci) fails instead and never writes either file. Unpinned package additions create a similar gap by allowing the registry to choose the version.

`locked-in` finds these gaps in Dockerfiles, CI workflows, shell scripts, Makefiles, Markdown, and `package.json` scripts. It enforces frozen-lockfile installs and version-pinned package additions for npm, pnpm, yarn, and bun. It also checks that JavaScript, Cargo, and Go manifests have corresponding lockfiles committed to Git.

The policy is deliberately conservative: install commands that may change dependency resolution are reported rather than inferred to be safe.

## Quick start

Choose a version from the [releases page](https://github.com/asymmetric-research/locked-in/releases), verify its commit, and install from that immutable revision. For example, `v0.4.0` resolves to `69b7eba80deb2d0285b51d63647c4a1139c1bab3`:

```bash
cargo install --locked --git https://github.com/asymmetric-research/locked-in \
  --rev 69b7eba80deb2d0285b51d63647c4a1139c1bab3 locked-in
```

Scan the current repository or pass a repository path:

```bash
locked-in
locked-in /path/to/repo
```

`locked-in` exits with status 0 when no violations are found, 1 when violations are found, and 2 for invalid command-line arguments. Missing tracked lockfiles and unavailable Git metadata are warnings and do not fail a run.

## GitHub Actions

Pin both the checkout action and `locked-in` to immutable commits:

```yaml
name: Lint package installs

on:
  pull_request:
  push:
    branches:
      - main

permissions:
  contents: read

jobs:
  locked-in:
    runs-on: ubuntu-latest
    env:
      # v0.4.0. Check releases before updating this value.
      LOCKED_IN_COMMIT: 69b7eba80deb2d0285b51d63647c4a1139c1bab3
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6.0.2
        with:
          persist-credentials: false
      - name: Install locked-in from pinned source
        run: cargo install --locked --git https://github.com/asymmetric-research/locked-in --rev "$LOCKED_IN_COMMIT" locked-in
      - name: Run locked-in
        run: locked-in .
```

This configuration avoids floating action and source references, disables persisted checkout credentials, and uses the repository's committed `Cargo.lock` during installation. Review new releases and their source before changing either pin. [zizmor](https://github.com/zizmorcore/zizmor) can independently check the workflow configuration.

## Rules

### npm

Accepted:

```bash
npm ci
npm i package@1.2.3
```

Reported:

```text
npm install
npm i package
```

### pnpm

Accepted:

```bash
pnpm install --frozen-lockfile
pnpm add package@1.2.3
```

Reported:

```text
pnpm install
pnpm add package
```

### yarn

Accepted:

```bash
yarn install --frozen-lockfile
yarn install --immutable
yarn add package@1.2.3
```

Reported:

```text
yarn install
yarn add package
```

### bun

Accepted:

```bash
bun install --frozen-lockfile
bun add package@1.2.3
```

- A bare `bun install` is also accepted when the repository's `bunfig.toml` sets [`[install].frozenLockfile = true`](https://bun.com/docs/runtime/bunfig#install-frozenlockfile).

Reported:

```text
bun install
bun add package
```

## Tracked lockfiles

`locked-in` reads `.git/index` to determine whether manifests and lockfiles are committed. A lockfile that exists only in the working tree does not satisfy this check.

| Manifest | Accepted lockfile |
|---|---|
| `package.json` | `package-lock.json`, `npm-shrinkwrap.json`, `pnpm-lock.yaml`, `yarn.lock`, `bun.lockb`, or `bun.lock` |
| `Cargo.toml` | `Cargo.lock` |
| `go.mod` | `go.sum` |

Cargo workspace members may use a tracked `Cargo.lock` from an ancestor workspace root. The Cargo Book [recommends committing `Cargo.lock` when in doubt](https://doc.rust-lang.org/cargo/faq.html#why-have-cargolock-in-version-control); it gives security reviews a deterministic dependency snapshot for libraries as well as binaries.

JavaScript workspace members may use a tracked lockfile from an ancestor workspace root that declares them as a member. `locked-in` recognizes the `package.json` `workspaces` array, the `{ "packages": [...] }` form, and `pnpm-workspace.yaml` `packages` entries. Workspace globs such as `packages/*` are resolved against the member path. A standalone `package.json` still requires a lockfile in its own directory.

A Go module without any `require` directives does not require `go.sum`.

If Git metadata is unavailable, `locked-in` reports a warning and skips tracked-lockfile validation. Missing tracked lockfiles are also warnings; unsafe install commands remain errors.

## Scanned files

`locked-in` scans:

- Dockerfiles: `Dockerfile*` and `*.dockerfile`
- Markdown: `*.md`
- Shell scripts: `*.sh`, `*.bash`, `*.zsh`, `*.fish`, `*.ksh`, and `*.csh`
- Makefiles: `Makefile`, `makefile`, `GNUmakefile`, and `*.mk`
- GitHub Actions workflows: `.github/workflows/*.yml` and `.github/workflows/*.yaml`
- `package.json` scripts

Scanning respects `.gitignore`. It also skips common generated and vendored directories, including `node_modules`, `target`, `dist`, `build`, `coverage`, `vendor`, `.next`, `.nuxt`, `.turbo`, and `.cache`.

Repositories are treated as untrusted input. `locked-in` rejects symbolic-link inputs and applies these limits:

| Input | Limit |
|---|---:|
| Source file | 16 MiB |
| Configuration or `package.json` file | 2 MiB |
| Git index | 64 MiB and 100,000 entries |
| Relevant files per repository | 100,000 |
| Concurrent file parsing | 2 files |
| Retained findings | 1,000 per file |

Files that cannot be read within these limits are reported as errors rather than silently skipped.

## Ignore directives

Use an inline directive when an unsafe-looking command is intentional and has been reviewed. A directive on its own line suppresses the next line:

```text
# locked-in: ignore
bun install
```

A directive at the end of a line suppresses that line:

```text
bun install  # locked-in: ignore
```

Include a rule ID to suppress only that rule:

```text
# locked-in: ignore[yarn-frozen-lockfile]
yarn install

npm i eslint  # locked-in: ignore[npm-version-pin]
```

Shell, YAML, Makefile, and Dockerfile inputs use `#` comments. Markdown uses `<!-- locked-in: ignore -->`.

### Rule IDs

| Rule ID | Description |
|---|---|
| `npm-install-bare` | Bare `npm install`; use `npm ci` |
| `npm-version-pin` | `npm install` or `npm i` without `@version` |
| `pnpm-frozen-lockfile` | `pnpm install` without `--frozen-lockfile` |
| `pnpm-version-pin` | `pnpm add` without `@version` |
| `yarn-frozen-lockfile` | `yarn install` without `--frozen-lockfile` or `--immutable` |
| `yarn-version-pin` | `yarn add` without `@version` |
| `bun-frozen-lockfile` | Bun's install command without a frozen lockfile flag or repository configuration |
| `bun-version-pin` | `bun add` without `@version` |
| `missing-tracked-lockfile` | A tracked manifest has no corresponding tracked lockfile |
| `git-metadata-unavailable` | Git metadata is unavailable for tracked-lockfile validation |
| `scan-input-unavailable` | An input could not be read safely within scanner limits |
| `scan-file-limit` | A repository exceeds the maximum number of scanned files |
| `scan-violation-limit` | Additional findings were omitted after the per-file limit |

## Example output

```text
✗ ./Dockerfile
  Error line 15: Use 'npm ci' instead of 'npm install' for lockfile-based installations
  > npm install

✗ ./.github/workflows/deploy.yml
  Error line 42: Use 'pnpm install --frozen-lockfile' to respect lockfile
  > pnpm install

═══════════════════════════════════════
✗ Found 2 violation(s) and 0 warning(s) in 2 files
```

## Dependency cooldowns

Lockfiles and version pins make dependency resolution reproducible; they do not prevent a newly published malicious version from being selected when a dependency is intentionally updated. A minimum release age adds a delay before new versions become eligible for installation. The two controls address different parts of the update process and should be used together where the package manager supports them.

For Bun, a repository-level policy can set a 24-hour minimum release age:

```toml
# bunfig.toml
[install]
minimumReleaseAge = 86400
```

[Package managers need to cool down](https://nesbitt.io/2026/03/04/package-managers-need-to-cool-down.html) surveys package-manager support and the security tradeoffs. Support changes over time, so check current npm, pnpm, yarn, and bun documentation before adopting a policy.

## Why we built it

At Asymmetric Research, package installation appears in the same places we review for higher-level security properties: build scripts, CI workflows, containers, and developer tooling. A repository can have a lockfile and still bypass it with one permissive install command, and that command is easy to miss in a large review.

We built `locked-in` to make that policy explicit and mechanically enforceable. It checks the command sites and the Git-tracked dependency state, reports uncertainty rather than treating it as safe, and is bounded so it can run against untrusted repositories in CI and security work.

## License

`locked-in` is available under the [Apache License 2.0](LICENSE).
