//! Source duration through the restricted `FFprobe` adapter.
//!
//! Supplied-transcript alignment needs the normalized duration of the staged
//! snapshot. This adapter runs the existing P04 probe, with its fixed
//! arguments, admission, deadline and output bounds, and reports only the
//! duration.

use vsift_application::{SourceDurationProbe, SourceProbeError};
use vsift_domain::MediaTime;

use crate::{
    FfmpegMedia, FilesystemSessionStore, HostIsolation, MediaError, MediaProviderConformance,
    ProcessCancellation, SourceSnapshot,
};

/// Probes a held snapshot's duration with trusted `FFmpeg`/`FFprobe` executables.
pub struct FfprobeSourceDuration {
    registry: MediaProviderConformance,
    host_isolation: HostIsolation,
    store: FilesystemSessionStore,
    cancellation: ProcessCancellation,
}

impl FfprobeSourceDuration {
    /// Creates the probe over an open store whose root holds the snapshot.
    #[must_use]
    pub const fn new(
        registry: MediaProviderConformance,
        host_isolation: HostIsolation,
        store: FilesystemSessionStore,
        cancellation: ProcessCancellation,
    ) -> Self {
        Self {
            registry,
            host_isolation,
            store,
            cancellation,
        }
    }
}

impl SourceDurationProbe<SourceSnapshot> for FfprobeSourceDuration {
    async fn source_duration(
        &self,
        source: &SourceSnapshot,
    ) -> Result<MediaTime, SourceProbeError> {
        let media = FfmpegMedia::new(self.registry.clone(), self.host_isolation, &self.store);
        media
            .probe(source, self.cancellation.clone())
            .await
            .map(|description| description.duration)
            .map_err(|error| match error {
                MediaError::CapacityUnavailable => SourceProbeError::Busy,
                MediaError::Deadline => SourceProbeError::Deadline,
                MediaError::Cancelled => SourceProbeError::Cancelled,
                MediaError::OutputLimit => SourceProbeError::ResourceLimit,
                MediaError::Process(_) | MediaError::Request(_) | MediaError::InvalidSourcePath => {
                    SourceProbeError::Io
                }
                _ => SourceProbeError::InvalidSource,
            })
    }
}
