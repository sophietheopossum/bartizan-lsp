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

use std::collections::HashMap;
use std::sync::Mutex;

use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

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
        let Some(_text) = self.docs.get(&params.text_document.uri) else {
            return Ok(None);
        };
        // Transformations land here next.
        Ok(None)
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
