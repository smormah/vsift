//! Immutable operation configuration and precedence.

use std::{error::Error, fmt, time::Duration};

use crate::command::ExecutionProfile;

const DEFAULT_PROBE_TIMEOUT_SECONDS: u64 = 5;
const MAX_PROBE_TIMEOUT_SECONDS: u64 = 60;

/// Values contributed by one explicit configuration layer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ConfigLayer {
    pub profile: Option<ExecutionProfile>,
    pub probe_timeout_seconds: Option<u64>,
}

/// Host-enforced bounds that no caller-controlled layer can override.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostPolicy {
    allowed_profiles: Vec<ExecutionProfile>,
    maximum_probe_timeout_seconds: u64,
}

impl HostPolicy {
    /// Returns the R0 local-host policy.
    #[must_use]
    pub(crate) fn local_r0() -> Self {
        Self {
            allowed_profiles: vec![ExecutionProfile::Desktop, ExecutionProfile::Worker],
            maximum_probe_timeout_seconds: MAX_PROBE_TIMEOUT_SECONDS,
        }
    }
}

/// Fully resolved immutable values for one setup check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EffectiveConfig {
    pub profile: ExecutionProfile,
    pub probe_timeout: Duration,
}

impl EffectiveConfig {
    /// Resolves `explicit -> selected -> user -> defaults`, then applies host policy.
    pub(crate) fn resolve(
        explicit: ConfigLayer,
        selected: ConfigLayer,
        user: ConfigLayer,
        host_policy: &HostPolicy,
    ) -> Result<Self, ConfigError> {
        let defaults = ConfigLayer {
            profile: Some(ExecutionProfile::Desktop),
            probe_timeout_seconds: Some(DEFAULT_PROBE_TIMEOUT_SECONDS),
        };
        let profile = explicit
            .profile
            .or(selected.profile)
            .or(user.profile)
            .or(defaults.profile)
            .ok_or(ConfigError::MissingDefault)?;
        let timeout_seconds = explicit
            .probe_timeout_seconds
            .or(selected.probe_timeout_seconds)
            .or(user.probe_timeout_seconds)
            .or(defaults.probe_timeout_seconds)
            .ok_or(ConfigError::MissingDefault)?;

        if !host_policy.allowed_profiles.contains(&profile) {
            return Err(ConfigError::ProfileDenied);
        }
        if timeout_seconds == 0 || timeout_seconds > host_policy.maximum_probe_timeout_seconds {
            return Err(ConfigError::ProbeTimeoutOutsidePolicy);
        }

        Ok(Self {
            profile,
            probe_timeout: Duration::from_secs(timeout_seconds),
        })
    }
}

/// Why immutable configuration could not be resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigError {
    /// A required built-in default was absent.
    MissingDefault,
    /// The selected profile is not allowed by host policy.
    ProfileDenied,
    /// The dependency-probe deadline violates the host cap.
    ProbeTimeoutOutsidePolicy,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MissingDefault => "required configuration default is missing",
            Self::ProfileDenied => "execution profile is denied by host policy",
            Self::ProbeTimeoutOutsidePolicy => "probe timeout is outside host policy",
        };
        formatter.write_str(message)
    }
}

impl Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::{ConfigError, ConfigLayer, EffectiveConfig, HostPolicy};
    use crate::command::ExecutionProfile;

    #[test]
    fn explicit_selected_user_and_default_precedence_is_deterministic() {
        let effective = EffectiveConfig::resolve(
            ConfigLayer {
                profile: None,
                probe_timeout_seconds: Some(4),
            },
            ConfigLayer {
                profile: Some(ExecutionProfile::Worker),
                probe_timeout_seconds: Some(8),
            },
            ConfigLayer {
                profile: Some(ExecutionProfile::Desktop),
                probe_timeout_seconds: Some(16),
            },
            &HostPolicy::local_r0(),
        );

        assert_eq!(
            effective,
            Ok(EffectiveConfig {
                profile: ExecutionProfile::Worker,
                probe_timeout: std::time::Duration::from_secs(4),
            })
        );
    }

    #[test]
    fn defaults_are_applied_without_loading_project_configuration() {
        let effective = EffectiveConfig::resolve(
            ConfigLayer::default(),
            ConfigLayer::default(),
            ConfigLayer::default(),
            &HostPolicy::local_r0(),
        );

        assert_eq!(
            effective,
            Ok(EffectiveConfig {
                profile: ExecutionProfile::Desktop,
                probe_timeout: std::time::Duration::from_secs(5),
            })
        );
    }

    #[test]
    fn host_policy_caps_every_configuration_layer() {
        let result = EffectiveConfig::resolve(
            ConfigLayer {
                profile: None,
                probe_timeout_seconds: Some(61),
            },
            ConfigLayer::default(),
            ConfigLayer::default(),
            &HostPolicy::local_r0(),
        );

        assert_eq!(result, Err(ConfigError::ProbeTimeoutOutsidePolicy));
    }
}
