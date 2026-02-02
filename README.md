# locked-in

Lints JavaScript package manager commands (npm, pnpm, yarn, bun) to enforce version pinning and lockfile usage.

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
```

Exit code 0 on success, 1 if violations found.

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
