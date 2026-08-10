# bartizan-lsp

A companion Rust language server that supplies code actions which neither clippy
nor rust-analyzer currently offers. It runs **alongside** rust-analyzer, not
instead of it.

## Why

Clippy tags each of its suggestions with an applicability level. Where a lint
emits a `MachineApplicable` replacement, rust-analyzer already surfaces it as a
quick fix and `cargo clippy --fix` applies it in bulk. But a number of lints ship
*no structured suggestion at all* — `collapsible_match` is one — because the
transformation isn't unconditionally safe. Those arrive in your editor as a
warning with nothing to apply.

rust-analyzer can't fill that gap either. Its assists cover neighbouring ground
(`merge_nested_if` is effectively the assist form of `clippy::collapsible_if`),
but there's no assist for every unserved lint, and upstream has had a
[feature freeze on IDE assists](https://github.com/rust-lang/rust-analyzer/blob/master/CONTRIBUTING.md)
since December 2025 pending the rowan transition, so new ones are closed on
sight.

So this sits beside it. Editors dispatch `textDocument/codeAction` to every
server registered for the language and merge the responses, which means both
sets of actions share one lightbulb with no coordination required.

## Design

Transformations are written against [`ra_ap_syntax`](https://crates.io/crates/ra_ap_syntax) —
rust-analyzer's own lossless syntax tree, published from the upstream repo. This
is deliberate: a transformation written here ports to a real rust-analyzer assist
nearly unchanged, so when the freeze lifts, upstreaming is a port rather than a
rewrite. The cost is that `ra_ap_syntax` is a `0.0.x` crate published weekly with
no stability promise, so it's pinned and upgraded on purpose.

The server holds no project model and watches no files. It parses the buffers the
editor sends it and nothing else, which is what keeps it cheap enough to run next
to a rust-analyzer already using several gigabytes.

## Status

Early. The server handshakes, tracks open buffers and advertises
`codeActionProvider`; the transformations themselves are being written.

## Editors

**VS Code** — `editors/vscode/` contains a thin client, since VS Code can only
register a language server from an extension and not from settings. Run
`npm install` there and symlink the directory into `~/.vscode-oss/extensions/`.

**Claude Code** — declare it in a local plugin marketplace with an `lspServers`
block pointing at the binary.

**Kate, Neovim, Helix** — configure `bartizan-lsp` as an additional server for
Rust; no wrapper needed.
