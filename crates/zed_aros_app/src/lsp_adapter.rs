//! A minimal Rust language-server adapter for AROS.
//!
//! It registers under the name `rust-analyzer` and reports a placeholder
//! binary as "user installed", which short-circuits all download/version
//! machinery in the installer. The binary is never executed: on AROS the
//! server is reached over TCP to a host-side bridge (see
//! `lsp::LanguageServer::new_tcp` and the AROS arm in `lsp_store`), so only the
//! adapter's identity and the fact that a binary "exists" matter here.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use gpui::AsyncApp;
use language::{
    LanguageServerName, LspAdapter, LspAdapterDelegate, LspInstaller, Toolchain,
};
use lsp::LanguageServerBinary;

pub struct ArosRustLspAdapter;

impl LspInstaller for ArosRustLspAdapter {
    type BinaryVersion = ();

    async fn check_if_user_installed(
        &self,
        _: &Arc<dyn LspAdapterDelegate>,
        _: Option<Toolchain>,
        _: &AsyncApp,
    ) -> Option<LanguageServerBinary> {
        // Placeholder only: the real server runs on the host bridge. Returning
        // Some(..) here makes binary resolution succeed without any download.
        Some(LanguageServerBinary {
            path: "rust-analyzer".into(),
            arguments: Vec::new(),
            env: None,
        })
    }

    async fn fetch_latest_server_version(
        &self,
        _: &Arc<dyn LspAdapterDelegate>,
        _: bool,
        _: &mut AsyncApp,
    ) -> Result<Self::BinaryVersion> {
        anyhow::bail!("no local server install on AROS; reached over the host LSP bridge")
    }

    fn fetch_server_binary(
        &self,
        _: (),
        _: PathBuf,
        _: &Arc<dyn LspAdapterDelegate>,
    ) -> impl Send + Future<Output = Result<LanguageServerBinary>> + use<> {
        async { anyhow::bail!("no local server install on AROS; reached over the host LSP bridge") }
    }

    async fn cached_server_binary(
        &self,
        _: PathBuf,
        _: &dyn LspAdapterDelegate,
    ) -> Option<LanguageServerBinary> {
        None
    }
}

#[async_trait(?Send)]
impl LspAdapter for ArosRustLspAdapter {
    fn name(&self) -> LanguageServerName {
        LanguageServerName("rust-analyzer".into())
    }
}
