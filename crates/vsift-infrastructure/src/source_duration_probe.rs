//! Source duration and recording-time tags through the restricted `FFprobe`
//! adapter.
//!
//! Supplied-transcript alignment needs the normalized duration of the staged
//! snapshot, and time anchoring (ADR 0029) the container's two
//! recording-time tags. This adapter runs the existing P04 probe once, with
//! its fixed arguments, admission, deadline and output bounds, and reports
//! only those.

use vsift_application::{ProbedSource, SourceProbe, SourceProbeError};

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

impl SourceProbe<SourceSnapshot> for FfprobeSourceDuration {
    async fn probe_source(
        &self,
        source: &SourceSnapshot,
    ) -> Result<ProbedSource, SourceProbeError> {
        let media = FfmpegMedia::new(self.registry.clone(), self.host_isolation, &self.store);
        media
            .probe(source, self.cancellation.clone())
            .await
            .map(|description| ProbedSource {
                duration: description.duration,
                clock_tags: description.clock_tags,
            })
            .map_err(|error| match error {
                MediaError::CapacityUnavailable => SourceProbeError::Busy,
                MediaError::Deadline => SourceProbeError::Deadline,
                MediaError::Cancelled => SourceProbeError::Cancelled,
                MediaError::OutputLimit => SourceProbeError::ResourceLimit,
                MediaError::Process(_) | MediaError::Request(_) => SourceProbeError::Io,
                _ => SourceProbeError::InvalidSource,
            })
    }
}
