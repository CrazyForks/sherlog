# Rust runtime and distribution

`shlog` is a published standalone Rust CLI with bundled SQLite/FTS5. Production code lives in the root crate under `src/`; TypeScript under `eval/` launches the binary for evaluation. There is no second TypeScript CLI and Node.js is not a runtime dependency.

The current design is maintained in [ARCHITECTURE.md](ARCHITECTURE.md), source interpretation and incremental rules in [SOURCE_CONTRACTS.md](SOURCE_CONTRACTS.md), and remaining acceptance work in [ROADMAP.md](ROADMAP.md). This page does not duplicate those contracts.

## Distribution

Current native targets are Apple Silicon macOS (`aarch64-apple-darwin`) and Linux x64 GNU (`x86_64-unknown-linux-gnu`). Each release includes archives, SPDX SBOMs, checksums, installer and Homebrew formula. Historical releases may contain additional targets.

The installer verifies checksums and installs without sudo. Existing executable replacement requires `SHERLOG_FORCE=1`; Homebrew users update through the tap. See [USAGE.md](USAGE.md) for upgrade commands.

A source commit, GitHub Release, tap update, website deployment, installed CLI and installed skill are separate delivery surfaces. Follow [the release checklist](../AGENTS.md#发布--更新闭环) and read each surface back. Never infer the installed version from a checkout build or keep machine-specific installed-version claims in architecture documentation.
