//! Finding the tools of a public page's fetch (ADR 0028 section 4.7, #409).
//!
//! The pair rule lives here: yt-dlp and its runtime are resolved together, so
//! the pinned pair is only ever run together. Nothing is resolved until a link
//! has answered with a web page, because opening a managed version hashes it.

use vsift_domain::{ManagedComponent, PageToolRecord, PageToolVersion};
use vsift_infrastructure::{
    ManagedRuntimeHold, ManagedRuntimeRole, PageTool, PageToolSource, PageTools, ProxyVariables,
    TrustedExecutable, managed_executable_name,
};

use crate::engine::Engine;

/// The engine, as the source of a page's tools.
pub(crate) struct EnginePageTools<'engine>(pub(crate) &'engine Engine);

impl PageToolSource for EnginePageTools<'_> {
    async fn page_tools(&self) -> Option<PageTools> {
        let (yt_dlp, js_runtime) = self.0.managed_page_pair()?;
        Some(PageTools::new(
            yt_dlp,
            Some(js_runtime),
            self.0.verified_ffmpeg().await,
            self.0.config().host_isolation.into_infrastructure(),
            ProxyVariables::from_environment(),
        ))
    }
}

impl Engine {
    /// The selected managed yt-dlp with the managed Deno, verified and held;
    /// `None` unless both are there: a managed yt-dlp runs with the managed
    /// Deno only.
    fn managed_page_pair(&self) -> Option<(PageTool, PageTool)> {
        let mut managed = self.managed_lookup();
        let yt_dlp = managed_page_tool(
            &managed.held(ManagedComponent::YtDlp)?,
            ManagedRuntimeRole::YtDlp,
        )?;
        let js_runtime = managed_page_tool(
            &managed.held(ManagedComponent::JsRuntimeDeno)?,
            ManagedRuntimeRole::Deno,
        )?;
        Some((yt_dlp, js_runtime))
    }

    /// The `FFmpeg` that joins separate streams, once verified as every media
    /// read's is; `None` when the media tools are missing or fail, which only
    /// a page that needs a join turns into a refusal.
    async fn verified_ffmpeg(&self) -> Option<TrustedExecutable> {
        let tools = self.media_tools().ok()?;
        self.ensure_media_tools_verified(&tools).await.ok()?;
        Some(tools.ffmpeg)
    }
}

/// The executable of `role` in a held managed version, and its record.
fn managed_page_tool(hold: &ManagedRuntimeHold, role: ManagedRuntimeRole) -> Option<PageTool> {
    let catalogue_version = PageToolVersion::parse(hold.runtime().identity().version()).ok()?;
    let executable = TrustedExecutable::managed_in(hold, &managed_executable_name(role)?).ok()?;
    Some(PageTool {
        executable,
        record: PageToolRecord::Managed { catalogue_version },
    })
}
