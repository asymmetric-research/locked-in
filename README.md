# locked-in

`locked-in` enforces version pinning, lockfile usage, and tracked-lockfile hygiene across JavaScript (npm, pnpm, yarn, bun), Cargo, and Go projects. It is **opinionated toward supply-chain security**: when in doubt, it errs on the conservative side.

It also warns when a tracked manifest exists without a corresponding lockfile tracked in the git index, so a lockfile present only in the working tree does not count.

For defense in depth, combine lockfiles and version pinning with package-manager dependency cooldowns (minimum release age), which reduce risk from newly published malicious packages.

## Rules

**npm:**
- ✅ `npm ci`, `npm i package@version`
- ❌ `npm install`, `npm i package`

**pnpm:**
- ✅ `pnpm install --frozen-lockfile`, `pnpm add package@version`
- ❌ `pnpm install`, `pnpm add package`

**yarn:**
- ✅ `yarn install --frozen-lockfile`, `yarn install --immutable`, `yarn add package@version`
- ❌ `yarn install`, `yarn add package`

**bun:**
- ✅ `bun install --frozen-lockfile`, `bun add package@version`
- ✅ bare `bun install` only when repo-local `bunfig.toml` sets `[install].frozenLockfile = true` (https://bun.com/docs/runtime/bunfig#install-frozenlockfile)
- ❌ `bun install`, `bun add package`

## Ignore Directives

Suppress violations with inline comments. Two placement styles are supported:

**Previous-line** — comment on its own line suppresses the next line:
```
# locked-in: ignore
bun install
```

**End-of-line** — comment at the end of the line suppresses that same line:
```
bun install  # locked-in: ignore
```

To suppress a specific rule, include the rule ID in brackets:
```
# locked-in: ignore[yarn-frozen-lockfile]
npm i eslint  # locked-in: ignore[npm-version-pin]
```

The comment syntax is extension-aware: `#` for shell, YAML, Makefile, and Dockerfile; `<!-- locked-in: ignore -->` for Markdown.

**Available rule IDs:**

| Rule ID | Description |
|---|---|
| `npm-install-bare` | bare `npm install` (should use `npm ci`) |
| `npm-version-pin` | `npm i/pkg` without `@version` |
| `pnpm-frozen-lockfile` | `pnpm install` without `--frozen-lockfile` |
| `pnpm-version-pin` | `pnpm add` without `@version` |
| `yarn-frozen-lockfile` | `yarn install` without `--frozen-lockfile`/`--immutable` |
| `yarn-version-pin` | `yarn add` without `@version` |
| `bun-frozen-lockfile` | `bun install` without `--frozen-lockfile` or config |
| `bun-version-pin` | `bun add` without `@version` |
| `missing-tracked-lockfile` | warning for tracked manifest without a tracked lockfile |
| `git-metadata-unavailable` | warning when git metadata is unavailable for tracked lockfile validation |

## Tracked Lockfiles

`locked-in` reads `.git/index` to verify tracked manifests have tracked lockfiles. A lockfile that exists only in the working tree does not satisfy this rule; it must be checked into git. Missing tracked lockfiles are warnings and do not fail the run.

Supported manifest pairs:

- `package.json` → `package-lock.json`, `npm-shrinkwrap.json`, `pnpm-lock.yaml`, `yarn.lock`, `bun.lockb`, or `bun.lock`
- `Cargo.toml` → `Cargo.lock`
- `go.mod` → `go.sum`

Cargo workspace members may use a tracked `Cargo.lock` from an ancestor workspace root (members never have individual lockfiles — that is Cargo workspace semantics). The workspace root's `Cargo.lock` should be committed: the Cargo Book [recommends](https://doc.rust-lang.org/cargo/faq.html#why-have-cargolock-in-version-control) checking it in ("when in doubt, check `Cargo.lock` into the version control system"), and from a supply-chain perspective it provides the same deterministic, auditable dependency snapshot that every other lockfile does, regardless of whether the crate is a library or binary.
Go modules without `require` directives do not require `go.sum`.

If git metadata is unavailable, tracked lockfile validation is skipped with a warning and does not fail the run.

## Scanned Files

- Dockerfiles (`Dockerfile*`, `*.dockerfile`)
- Markdown (`*.md`)
- Shell scripts (`*.sh`, `*.bash`, `*.zsh`, `*.fish`, `*.ksh`, `*.csh`)
- Makefiles (`Makefile`, `makefile`, `GNUmakefile`, `*.mk`)
- GitHub Actions workflows (`.github/workflows/*.yml`, `.github/workflows/*.yaml`)
- `package.json` (the `scripts` field — npm/pnpm/yarn/bun commands run from here)

Scanning respects `.gitignore` and skips common generated/vendor directories such as `node_modules`, `target`, `dist`, `build`, `coverage`, `vendor`, `.next`, `.nuxt`, `.turbo`, and `.cache`.

## Usage

### GitHub Action

The following is an example GitHub Action config that can be used to configure `locked-in`. 
It's recommended to keep this up-to-date with the latest releases on this repo. (And to verify it independently via [zizmor](https://github.com/zizmorcore/zizmor).)

```yaml
name: Lint Package Installs

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
      # Note: check recent releases and update these values.
      LOCKED_IN_COMMIT: 74d8fd31d519ea9f4f95b01191dc6171df90f045 # v0.2.0
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6.0.2
        with:
          persist-credentials: false
      - name: Install locked-in from pinned source
        run: cargo install --locked --git https://github.com/asymmetric-research/locked-in --rev "$LOCKED_IN_COMMIT" locked-in
      - name: Run locked-in
        run: locked-in .
```

This pattern is recommended over floating refs such as `uses: asymmetric-research/locked-in@main` or `cargo install --git ...` without `--rev`: 
it pins both `actions/checkout` and `locked-in` to immutable commits, disables persisted checkout credentials, and keeps 
Cargo dependency resolution locked to the repository's `Cargo.lock`.

If you prefer inline values instead of an environment variable, pin `--rev` directly:

```bash
cargo install --locked --git https://github.com/asymmetric-research/locked-in --rev 74d8fd31d519ea9f4f95b01191dc6171df90f045 locked-in
```

### CLI

```bash
# Install from a pinned commit. This example refers to locked-in v0.2.0
cargo install --locked --git https://github.com/asymmetric-research/locked-in --rev 74d8fd31d519ea9f4f95b01191dc6171df90f045 locked-in

# Run
locked-in

# Or scan a specific repository
locked-in /path/to/repo
```

Exit code 0 on success, 1 if violations found.

## Dependency Cooldowns

Lockfiles and version pinning are the primary controls this tool enforces. As defense in depth, consider enabling dependency cooldowns (minimum release age) in your package manager so very new package versions are not installable immediately.

A good overview of dependency cooldown support can be found in the post [Package managers need to cool down](https://nesbitt.io/2026/03/04/package-managers-need-to-cool-down.html).

- Use lockfiles + version pins to ensure reproducibility.
- Use a minimum release age/cooldown to reduce exposure to fresh supply-chain attacks.
- Keep this as an organizational policy in repo-level config where possible.

Example policy (npm):

```ini
# .npmrc
minimumReleaseAge=1440
```

Manager notes:
- npm: supports `minimumReleaseAge` in config.
- pnpm/yarn/bun: no direct equivalent documented here; keep using strict lockfile installs and exact version pins.

## Example

```
✗ ./Dockerfile
  Line 15: Use 'npm ci' instead of 'npm install' for lockfile-based installations
  > npm install

✗ ./.github/workflows/deploy.yml
  Line 42: Use 'pnpm install --frozen-lockfile' to respect lockfile
  > pnpm install

═══════════════════════════════════════
✗ Found 2 violation(s) in 2 files
```
