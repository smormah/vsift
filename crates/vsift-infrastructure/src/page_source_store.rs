//! A session store that routes a link answering with a web page to yt-dlp
//! (ADR 0028 section 4.7, #409).
//!
//! The application's open-session use case asks its port to stage a link. The
//! direct fetch runs first, with all of section 3's checks; only a link that
//! answered `web_page` goes further, and only if a yt-dlp is resolved then.
//! Resolving opens managed versions, which hashes them, so a direct link
//! never pays for it: [`PageToolSource`] is asked at that moment. Every other
//! port operation is the store's own.

use std::{future::Future, path::Path};

use vsift_application::{
    AuthorizedSessionGenerationPublication, AuthorizedSessionStorageInitialization,
    ForegroundSessionPort, OpenSessionError, SessionStorageError, SessionStore, StageCancellation,
    StorageCapabilities,
};
use vsift_domain::{
    DownloadLimits, OperationId, SessionId, SessionLifetimePolicy, SourceFetchReason, SourceLink,
    StorageGeneration, TranscriptRevision,
};

use crate::{
    FilesystemSessionStore, PageTools, SessionRegistration, SourceSnapshot,
    link_acquisition::LinkRoute,
    source_snapshot::{SourceError, open_session_error},
};

/// Finds the tools of a page's fetch when one is needed.
pub trait PageToolSource: Send + Sync {
    /// The resolved yt-dlp and what goes with it, or `None` when no yt-dlp
    /// can be used, so the link stays refused as a web page.
    fn page_tools(&self) -> impl Future<Output = Option<PageTools>> + Send;
}

/// A [`FilesystemSessionStore`] whose links may be pages.
pub struct PageSourceStore<T> {
    store: FilesystemSessionStore,
    tools: T,
    route: LinkRoute,
}

impl<T> PageSourceStore<T> {
    /// The store, and where a page's tools come from.
    #[must_use]
    pub const fn new(store: FilesystemSessionStore, tools: T) -> Self {
        Self {
            store,
            tools,
            route: LinkRoute::production(),
        }
    }

    /// The same store fetching through a local HTTPS test server
    /// (`install-test-hooks`, development builds only).
    #[cfg(any(test, feature = "install-test-hooks"))]
    #[must_use]
    pub fn through_test_route(mut self, route: crate::LinkTestRoute) -> Self {
        self.route = LinkRoute::test(route);
        self
    }
}

impl<T: Send + Sync> SessionStore for PageSourceStore<T> {
    fn capabilities(&self) -> StorageCapabilities {
        self.store.capabilities()
    }

    fn initialize(
        &self,
        request: AuthorizedSessionStorageInitialization,
    ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
        self.store.initialize(request)
    }

    fn publish(
        &self,
        request: AuthorizedSessionGenerationPublication,
    ) -> impl Future<Output = Result<StorageGeneration, SessionStorageError>> + Send {
        self.store.publish(request)
    }
}

impl<T: PageToolSource> ForegroundSessionPort for PageSourceStore<T> {
    type Registration = SessionRegistration;
    type Snapshot = SourceSnapshot;

    fn register(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        now_unix_seconds: u64,
    ) -> Result<Self::Registration, OpenSessionError> {
        ForegroundSessionPort::register(&self.store, session_id, operation_id, now_unix_seconds)
    }

    fn lifetime_policy(&self) -> SessionLifetimePolicy {
        self.store.lifetime_policy()
    }

    fn stage_source(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        source: &Path,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self::Snapshot, OpenSessionError> {
        ForegroundSessionPort::stage_source(
            &self.store,
            session_id,
            operation_id,
            source,
            cancellation,
        )
    }

    async fn stage_link(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        link: &SourceLink,
        limits: DownloadLimits,
        cancellation: &dyn StageCancellation,
    ) -> Result<Self::Snapshot, OpenSessionError> {
        let direct = SourceSnapshot::stage_link_through(
            &self.store,
            session_id,
            operation_id,
            (link, limits),
            &self.route,
            cancellation,
        )
        .await;
        let staged = match direct {
            Err(SourceError::Fetch(SourceFetchReason::WebPage)) => {
                match self.tools.page_tools().await {
                    Some(tools) => {
                        SourceSnapshot::stage_page_through(
                            &self.store,
                            session_id,
                            operation_id,
                            (link, limits),
                            (&tools, &self.route),
                            cancellation,
                        )
                        .await
                    }
                    None => Err(SourceError::Fetch(SourceFetchReason::WebPage)),
                }
            }
            other => other,
        };
        staged.map_err(open_session_error)
    }

    fn activate(
        &self,
        snapshot: &Self::Snapshot,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        now_unix_seconds: u64,
        time_anchors: &vsift_domain::TimeAnchors,
    ) -> Result<StorageGeneration, OpenSessionError> {
        ForegroundSessionPort::activate(
            &self.store,
            snapshot,
            operation_id,
            expected_generation,
            now_unix_seconds,
            time_anchors,
        )
    }

    fn activate_with_transcript(
        &self,
        snapshot: &Self::Snapshot,
        operation_id: &OperationId,
        expected_generation: StorageGeneration,
        now_unix_seconds: u64,
        time_anchors: &vsift_domain::TimeAnchors,
        transcript: &TranscriptRevision,
    ) -> Result<StorageGeneration, OpenSessionError> {
        ForegroundSessionPort::activate_with_transcript(
            &self.store,
            snapshot,
            operation_id,
            expected_generation,
            now_unix_seconds,
            time_anchors,
            transcript,
        )
    }
}
