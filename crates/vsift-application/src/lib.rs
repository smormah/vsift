//! Application use cases and infrastructure ports for `VSift`.

#![forbid(unsafe_code)]

use std::future::Future;

use vsift_domain::{DependencyStatus, RuntimeCapability, RuntimeDependency, RuntimeReadiness};

mod storage;

pub use storage::{
    AuthorizedSessionStorageInitialization, InitializeSessionStorage,
    InitializeSessionStorageRequest, InitializedSessionStorage, SessionStorageError, SessionStore,
    StorageCapabilities,
};

/// Port used to inspect one specialist runtime dependency.
pub trait DependencyProbe: Send + Sync {
    /// Inspects a dependency without modifying the local machine.
    fn probe(&self, dependency: RuntimeDependency)
    -> impl Future<Output = DependencyStatus> + Send;
}

/// Complete result of the read-only runtime diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeDiagnosis {
    /// Aggregate ability of `VSift` to process a video locally.
    pub readiness: RuntimeReadiness,
    /// Individual dependency results in stable display order.
    pub dependencies: Vec<DependencyStatus>,
}

/// Read-only use case that diagnoses `VSift`'s local runtime.
pub struct DiagnoseRuntime<P> {
    probe: P,
}

impl<P> DiagnoseRuntime<P>
where
    P: DependencyProbe,
{
    /// Creates the use case with its dependency probe.
    pub const fn new(probe: P) -> Self {
        Self { probe }
    }

    /// Inspects all known dependencies and calculates aggregate readiness.
    pub async fn execute(&self) -> RuntimeDiagnosis {
        let mut dependencies = Vec::with_capacity(RuntimeDependency::ALL.len());

        for dependency in RuntimeDependency::ALL {
            dependencies.push(self.probe.probe(dependency).await);
        }

        let media_ready = dependencies
            .iter()
            .filter(|status| status.dependency.capability() == RuntimeCapability::MediaProcessing)
            .all(|status| status.state.is_available());

        let transcription_ready = dependencies.iter().any(|status| {
            status.dependency.capability() == RuntimeCapability::Transcription
                && status.state.is_available()
        });

        let readiness = match (media_ready, transcription_ready) {
            (false, _) => RuntimeReadiness::Blocked,
            (true, false) => RuntimeReadiness::Degraded,
            (true, true) => RuntimeReadiness::Ready,
        };

        RuntimeDiagnosis {
            readiness,
            dependencies,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::ready;

    use super::{DependencyProbe, DiagnoseRuntime};
    use vsift_domain::{DependencyState, DependencyStatus, RuntimeDependency, RuntimeReadiness};

    struct FakeProbe {
        unavailable: Vec<RuntimeDependency>,
    }

    impl DependencyProbe for FakeProbe {
        fn probe(
            &self,
            dependency: RuntimeDependency,
        ) -> impl Future<Output = DependencyStatus> + Send {
            let state = if self.unavailable.contains(&dependency) {
                DependencyState::Missing
            } else {
                DependencyState::Available {
                    version: String::from("test version"),
                }
            };

            ready(DependencyStatus { dependency, state })
        }
    }

    #[tokio::test]
    async fn reports_blocked_when_media_processing_is_missing() {
        let use_case = DiagnoseRuntime::new(FakeProbe {
            unavailable: vec![RuntimeDependency::Ffmpeg],
        });

        let diagnosis = use_case.execute().await;

        assert_eq!(diagnosis.readiness, RuntimeReadiness::Blocked);
    }

    #[tokio::test]
    async fn reports_degraded_when_only_transcription_is_missing() {
        let use_case = DiagnoseRuntime::new(FakeProbe {
            unavailable: vec![RuntimeDependency::Whisper],
        });

        let diagnosis = use_case.execute().await;

        assert_eq!(diagnosis.readiness, RuntimeReadiness::Degraded);
    }

    #[tokio::test]
    async fn reports_ready_when_every_dependency_is_available() {
        let use_case = DiagnoseRuntime::new(FakeProbe {
            unavailable: Vec::new(),
        });

        let diagnosis = use_case.execute().await;

        assert_eq!(diagnosis.readiness, RuntimeReadiness::Ready);
    }
}
