//! Finding the tools of a public page's fetch (ADR 0028 section 4.7, #409).
//!
//! The pair rule lives here: resolution is by pair, not by tool. "Configured
//! first, then managed" applies to yt-dlp, and the runtime follows yt-dlp's
//! source: a managed yt-dlp runs with the managed Deno only, and a user's with
//! the user's runtime or with none, never the managed Deno. So the pinned pair
//! is only ever run together, and a runtime registered without a yt-dlp is not
//! used. Nothing is resolved until a link has answered with a web page,
//! because opening a managed version hashes it.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use vsift_domain::{ManagedComponent, PageToolRecord, PageToolVersion};
use vsift_infrastructure::{
    ManagedRuntimeHold, ManagedRuntimeRole, PageTool, PageToolSource, PageTools,
    ProcessCancellation, ProxyVariables, TrustedExecutable, UserOnlineTool,
    identify_user_online_tool, managed_executable_name,
};

use crate::engine::Engine;

/// The longest one registered tool may take to identify itself.
const IDENTIFY_TIME: Duration = Duration::from_secs(30);

/// Which pair a fetch runs, decided by what the user registered.
#[derive(Debug, Eq, PartialEq)]
enum PagePair {
    /// No yt-dlp is registered: the managed pair, whatever else is.
    Managed,
    /// A registered yt-dlp, with the registered runtime if there is one.
    Registered {
        yt_dlp: PathBuf,
        js_runtime: Option<PathBuf>,
    },
}

impl PagePair {
    fn of(yt_dlp: Option<PathBuf>, js_runtime: Option<PathBuf>) -> Self {
        yt_dlp.map_or(Self::Managed, |yt_dlp| Self::Registered {
            yt_dlp,
            js_runtime,
        })
    }
}

/// The engine, as the source of a page's tools, for one ingest.
pub(crate) struct EnginePageTools<'engine> {
    pub(crate) engine: &'engine Engine,
    pub(crate) cancellation: ProcessCancellation,
}

impl PageToolSource for EnginePageTools<'_> {
    async fn page_tools(&self) -> Option<PageTools> {
        let store = self.engine.user_configuration().ok()?;
        let pair = PagePair::of(store.read_yt_dlp().ok()?, store.read_js_runtime().ok()?);
        let (yt_dlp, js_runtime) = match pair {
            PagePair::Managed => {
                let (yt_dlp, js_runtime) = self.engine.managed_page_pair()?;
                (yt_dlp, Some(js_runtime))
            }
            PagePair::Registered { yt_dlp, js_runtime } => {
                let state = store.media_tool_verification_state().ok()?;
                self.registered_pair(&yt_dlp, js_runtime.as_deref(), state.workspace_parent())
                    .await?
            }
        };
        Some(PageTools::new(
            yt_dlp,
            js_runtime,
            self.verified_ffmpeg().await,
            self.engine.config().host_isolation.into_infrastructure(),
            ProxyVariables::from_environment(),
        ))
    }
}

impl EnginePageTools<'_> {
    /// The user's yt-dlp, identified as `setup check` identifies it, and
    /// their runtime. A yt-dlp that cannot be identified is not run. A runtime
    /// that cannot be identified is left out, so yt-dlp runs with none and
    /// `js_runtime_missing` is the honest outcome if the site needs one.
    async fn registered_pair(
        &self,
        yt_dlp: &Path,
        js_runtime: Option<&Path>,
        workspace_parent: &Path,
    ) -> Option<(PageTool, Option<PageTool>)> {
        let yt_dlp = self
            .registered_tool(UserOnlineTool::YtDlp, yt_dlp, workspace_parent)
            .await?;
        let js_runtime = match js_runtime {
            Some(path) => {
                self.registered_tool(UserOnlineTool::JsRuntime, path, workspace_parent)
                    .await
            }
            None => None,
        };
        Some((yt_dlp, js_runtime))
    }

    async fn registered_tool(
        &self,
        tool: UserOnlineTool,
        path: &Path,
        workspace_parent: &Path,
    ) -> Option<PageTool> {
        let executable = TrustedExecutable::explicit(path).ok()?;
        let facts = identify_user_online_tool(
            tool,
            &executable,
            workspace_parent,
            self.engine.config().host_isolation.into_infrastructure(),
            IDENTIFY_TIME,
            self.cancellation.clone(),
        )
        .await
        .ok()?;
        Some(PageTool {
            executable,
            record: PageToolRecord::UserManaged {
                version: PageToolVersion::parse(&facts.version_line).ok()?,
                sha256: facts.sha256,
            },
        })
    }

    /// The `FFmpeg` that joins separate streams, once verified as every media
    /// read's is; `None` when the media tools are missing or fail, which only
    /// a page that needs a join turns into a refusal.
    async fn verified_ffmpeg(&self) -> Option<TrustedExecutable> {
        let tools = self.engine.media_tools().ok()?;
        self.engine
            .ensure_media_tools_verified_with(&tools, &self.cancellation)
            .await
            .ok()?;
        Some(tools.ffmpeg)
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

#[cfg(test)]
mod tests {
    use super::{PagePair, PathBuf};

    fn path(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    /// The planner's ruling of 2026-10-31: the runtime follows yt-dlp's source.
    #[test]
    fn the_runtime_follows_ytdlps_source() {
        assert_eq!(
            PagePair::of(Some(path("/u/yt-dlp")), Some(path("/u/deno"))),
            PagePair::Registered {
                yt_dlp: path("/u/yt-dlp"),
                js_runtime: Some(path("/u/deno")),
            }
        );
        assert_eq!(
            PagePair::of(Some(path("/u/yt-dlp")), None),
            PagePair::Registered {
                yt_dlp: path("/u/yt-dlp"),
                js_runtime: None,
            },
            "a user's yt-dlp never takes the managed Deno"
        );
        assert_eq!(
            PagePair::of(None, Some(path("/u/deno"))),
            PagePair::Managed,
            "a runtime with no registered yt-dlp is not used"
        );
        assert_eq!(PagePair::of(None, None), PagePair::Managed);
    }
}
