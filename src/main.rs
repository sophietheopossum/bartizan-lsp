//! Bartizan — a companion language server that supplies Rust code actions which
//! neither clippy nor rust-analyzer currently offers.
//!
//! A bartizan is a turret corbelled out from a wall: a small structure attached
//! to a larger one, covering ground the main wall cannot reach. rust-analyzer is
//! the wall.
//!
//! Why this exists rather than a rust-analyzer patch: upstream has a feature
//! freeze on IDE assists (CONTRIBUTING.md, in force since Dec 2025) pending the
//! rowan transition, so new assists are closed on sight. This sits alongside
//! rust-analyzer instead — editors dispatch `textDocument/codeAction` to every
//! server registered for the language, so both sets of actions appear together.

mod fixes;
mod line_index;
use std::collections::HashMap;
use std::sync::Mutex;

use ra_ap_syntax::{
	Edition,
	SourceFile,
	TextRange
};
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

use line_index::LineIndex;
#[derive(Default)]
struct Documents(Mutex<HashMap<Uri, String>>);

impl Documents {
    fn set(&self, uri: Uri, text: String) {
        self.0.lock().unwrap().insert(uri, text);
    }

    fn get(&self, uri: &Uri) -> Option<String> {
        self.0.lock().unwrap().get(uri).cloned()
    }

    fn remove(&self, uri: &Uri) {
        self.0.lock().unwrap().remove(uri);
    }
}

struct Backend {
    #[allow(dead_code)]
    client: Client,
    docs: Documents,
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            // Derived, never spelled out — a hardcoded name silently survived a
            // rename once already and the handshake test was what caught it.
            server_info: Some(ServerInfo {
                name: env!("CARGO_PKG_NAME").into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            capabilities: ServerCapabilities {
                // Full sync: these files are small enough that incremental sync
                // buys nothing, and a whole-buffer copy keeps the parse trivial.
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.docs
            .set(params.text_document.uri, params.text_document.text);
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.pop() {
            self.docs.set(params.text_document.uri, change.text);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.docs.remove(&params.text_document.uri);
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let Some(text) = self.docs.get(&params.text_document.uri) else {
            return Ok(None);
        };
        let index = LineIndex::new(&text);
        let Some(offset) = index.offset(params.range.start) else {
            return Ok(None);
        };

        // Parsing is unconditional and cheap relative to anything semantic —
        // there is no project model here to keep warm, which is the whole reason
        // this can sit next to a rust-analyzer already holding several GB.
        let parse = SourceFile::parse(&text, Edition::CURRENT);

        let tree = parse.tree();

        // Fixes the editor asked for by name. `context.diagnostics` carries what
        // the client already knows about this range — including rust-analyzer's
        // clippy output — so a lint that needed the type table to *detect* can be
        // acted on here without this server ever building one.
        let from_diagnostics = params.context.diagnostics.iter().filter_map(|diagnostic| {
            let NumberOrString::String(code) = diagnostic.code.as_ref()? else {
                return None;
            };
            let start = index.offset(diagnostic.range.start)?;
            let end = index.offset(diagnostic.range.end)?;
            let fix = fixes::for_diagnostic(&tree, code, TextRange::new(start, end))?;
            Some((fix, Some(diagnostic.clone())))
        });

        let actions: Vec<_> = fixes::all(&tree, offset)
            .into_iter()
            .map(|fix| (fix, None))
            .chain(from_diagnostics)
            .map(|(fix, diagnostic)| {
                let edits = fix
                    .edits
                    .iter()
                    .map(|(range, new_text)| TextEdit {
                        range: index.range(*range),
                        new_text: new_text.clone(),
                    })
                    .collect();
                let mut changes = HashMap::new();
                changes.insert(params.text_document.uri.clone(), edits);

                CodeActionOrCommand::CodeAction(CodeAction {
                    title: fix.title,
                    // Answering a diagnostic makes it a quickfix, and naming that
                    // diagnostic attaches it to the squiggle's own lightbulb.
                    // Everything else keeps the kind rust-analyzer gives its
                    // assists, so those land in the Refactor menu beside them
                    // rather than in a category of their own.
                    kind: Some(if diagnostic.is_some() {
                        CodeActionKind::QUICKFIX
                    } else {
                        CodeActionKind::REFACTOR_REWRITE
                    }),
                    diagnostics: diagnostic.map(|d| vec![d]),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            })
            .collect();

        Ok((!actions.is_empty()).then_some(actions))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        docs: Documents::default(),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
