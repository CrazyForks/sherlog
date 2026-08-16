# Contributing

## 开发环境

Production CLI 是 standalone Rust workspace：

- Rust stable，最低 `rust-version = 1.85`；
- Cargo；
- 支持本地构建的 macOS/Linux 环境。

Node.js `>= 22` 只用于 TypeScript differential oracle 与 `eval/` harness，不是 production runtime。

安装开发依赖：

```bash
cargo fetch --locked
npm ci
```

## 常用命令

运行当前 Rust CLI：

```bash
cargo run --locked --bin shlog -- status --json
```

Production gates：

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --release --locked --bin shlog
```

Development oracle/eval gates：

```bash
npm run check
npm run eval:acceptance -- --require-candidate --cli-argv-json '["./target/release/shlog"]'
npm run eval:contract -- --require-candidate --candidate-argv-json '["./target/release/shlog"]'
```

`npm run check` 只验证 TypeScript oracle/eval workspace，不能替代 Rust gates。手工评测与批次比较仍可用：

```bash
npm run eval:manual
npm run eval:compare -- data/shlog-eval/<before-batch> data/shlog-eval/<after-batch>
```

## 贡献边界

- `sync` 是唯一 content/index/coverage writer；`cold add/remove` 只写 retention state。
- `find`、`read-*`、`list`、`stats`、`cold list` 只读 SQLite。`status` 不返回/检索正文、不写 index；inventory cache miss 会流式读取 raw 并只用 privacy-allowlisted projection 派生 proof，exact cache hit 不重 parse。
- TypeScript `src/**` 是 differential oracle，不是 production entrypoint；行为修改以 Rust contract 为主，并决定 oracle/eval 是否需要同步。
- v8 SQLite 是唯一持久化真相；不要重新引入 metadata sidecar、daemon/watcher 或第二状态存储。
- 保持 source privacy allowlist、query-only read、SQL scope-filter pushdown、message evidence provenance 与 `incremental == full replay` 不变量。
- 不要把目标态建议写成当前已实现事实；涉及 CLI/JSON/storage 的变更同步更新 `skill-packages/sherlog` 与当前态文档。
- 不要提交 `data/`、`target/` 或 `node_modules/`。

## 提交前

至少运行与改动相关的 Rust gates 与 `npm run check`。查询、排序或 JSON contract 变更还应使用显式 native candidate 跑 acceptance/contract；sync/migration/cold 变更应补对应 Rust state/failure tests。

Native tag/assets 发布前只能说明 source-ready。不要用本地 build、全局旧 `shlog` 或 TypeScript oracle 冒充已发布 native CLI。
