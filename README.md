# locked-in

Lints JavaScript package manager commands (npm, pnpm, yarn, bun) to enforce version pinning and lockfile usage.

For stronger supply-chain hygiene, combine lockfiles and version pinning with package-manager dependency cooldowns (minimum release age), which reduce risk from newly published malicious packages.

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

## Scanned Files

- Dockerfiles (`Dockerfile*`, `*.dockerfile`)
- Markdown (`*.md`)
- Shell scripts (`*.sh`)
- GitHub Actions workflows (`.github/workflows/*.yml`, `.github/workflows/*.yaml`)

## Usage

### GitHub Action

```yaml
name: Lint Package Installs

on: [push, pull_request]

jobs:
  locked-in:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: asymmetric-research/locked-in@main
```

To pin to a specific commit:

```yaml
      - uses: asymmetric-research/locked-in@main
        with:
          commit: abc123f  # specific commit SHA
```

### CLI

```bash
# Install
cargo install --git https://github.com/asymmetric-research/locked-in

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
