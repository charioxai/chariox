use crate::error::DaemonError;

const BYTES_PER_MEBIBYTE: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SliceDiskQuotaLimits {
    pub(super) writable_layer_bytes: u64,
    pub(super) persistent_home_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WritableLayerQuotaEvidence {
    pub(super) backend_supports_hard_quota: bool,
    pub(super) effective_limit_bytes: Option<u64>,
    pub(super) used_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PersistentHomeQuotaEvidence {
    pub(super) backend_supports_hard_quota: bool,
    pub(super) is_persistent: bool,
    pub(super) effective_limit_bytes: Option<u64>,
    pub(super) used_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SliceDiskQuotaEvidence {
    pub(super) writable_layer: WritableLayerQuotaEvidence,
    pub(super) persistent_home: PersistentHomeQuotaEvidence,
}

impl SliceDiskQuotaLimits {
    pub(super) fn from_megabytes(
        writable_layer_mb: Option<u32>,
        persistent_home_mb: Option<u32>,
    ) -> Result<Option<Self>, DaemonError> {
        match (writable_layer_mb, persistent_home_mb) {
            (None, None) => Ok(None),
            (Some(0), _) => Err(DaemonError::InvalidConfig {
                field: "slices.linux.disk_layer_mb",
                message: "disk_layer_mb must be a positive integer",
            }),
            (_, Some(0)) => Err(DaemonError::InvalidConfig {
                field: "slices.linux.disk_home_mb",
                message: "disk_home_mb must be a positive integer",
            }),
            (Some(layer_mb), Some(home_mb)) => {
                Ok(Some(Self {
                    writable_layer_bytes: u64::from(layer_mb) * BYTES_PER_MEBIBYTE,
                    persistent_home_bytes: u64::from(home_mb) * BYTES_PER_MEBIBYTE,
                }))
            }
            (Some(_), None) => Err(DaemonError::InvalidConfig {
                field: "slices.linux.disk_home_mb",
                message: "disk_layer_mb and disk_home_mb must be configured together",
            }),
            (None, Some(_)) => Err(DaemonError::InvalidConfig {
                field: "slices.linux.disk_layer_mb",
                message: "disk_layer_mb and disk_home_mb must be configured together",
            }),
        }
    }
}

/// Admit a bounded slice only after the backend proves hard enforcement for
/// both storage classes and reports the effective limits and existing usage.
/// Usage measurement is an admission check; it is never treated as quota
/// enforcement by itself.
pub(super) fn require_verified_quotas(
    limits: Option<&SliceDiskQuotaLimits>,
    evidence: Option<&SliceDiskQuotaEvidence>,
) -> Result<(), DaemonError> {
    let Some(limits) = limits else {
        return Ok(());
    };
    let Some(evidence) = evidence else {
        return Err(quota_error(
            "hard writable-layer and persistent-home quotas cannot be verified; the Docker broker and provisioner must report backend support, persistent-volume type, current usage, and both effective hard limits before a bounded slice can start",
        ));
    };

    if !evidence.writable_layer.backend_supports_hard_quota {
        return Err(quota_error(
            "Docker engine or backing filesystem does not support a hard writable-layer quota",
        ));
    }
    if !evidence.persistent_home.backend_supports_hard_quota {
        return Err(quota_error(
            "backing filesystem does not support a hard persistent-home quota",
        ));
    }
    if !evidence.persistent_home.is_persistent {
        return Err(quota_error(
            "persistent-home quota resolved to non-persistent storage; tmpfs is not an acceptable substitute",
        ));
    }

    require_exact_limit(
        evidence.writable_layer.effective_limit_bytes,
        limits.writable_layer_bytes,
        "writable-layer",
    )?;
    require_exact_limit(
        evidence.persistent_home.effective_limit_bytes,
        limits.persistent_home_bytes,
        "persistent-home",
    )?;
    require_existing_usage_within_limit(
        evidence.writable_layer.used_bytes,
        limits.writable_layer_bytes,
        "writable-layer",
    )?;
    require_existing_usage_within_limit(
        evidence.persistent_home.used_bytes,
        limits.persistent_home_bytes,
        "persistent-home",
    )?;
    Ok(())
}

fn require_exact_limit(
    effective_limit_bytes: Option<u64>,
    requested_limit_bytes: u64,
    storage_name: &str,
) -> Result<(), DaemonError> {
    match effective_limit_bytes {
        Some(effective) if effective == requested_limit_bytes => Ok(()),
        Some(_) => Err(quota_error(format!(
            "effective {storage_name} hard quota does not match the configured limit"
        ))),
        None => Err(quota_error(format!(
            "effective {storage_name} hard quota could not be read back"
        ))),
    }
}

fn require_existing_usage_within_limit(
    used_bytes: Option<u64>,
    requested_limit_bytes: u64,
    storage_name: &str,
) -> Result<(), DaemonError> {
    match used_bytes {
        Some(used) if used <= requested_limit_bytes => Ok(()),
        Some(_) => Err(quota_error(format!(
            "existing {storage_name} data exceeds the configured hard quota"
        ))),
        None => Err(quota_error(format!(
            "existing {storage_name} data usage could not be measured"
        ))),
    }
}

fn quota_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "slice.disk.quota",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> SliceDiskQuotaLimits {
        SliceDiskQuotaLimits {
            writable_layer_bytes: 512 * BYTES_PER_MEBIBYTE,
            persistent_home_bytes: 2_048 * BYTES_PER_MEBIBYTE,
        }
    }

    fn invalid_config_field(
        writable_layer_mb: Option<u32>,
        persistent_home_mb: Option<u32>,
    ) -> &'static str {
        match SliceDiskQuotaLimits::from_megabytes(writable_layer_mb, persistent_home_mb)
            .expect_err("invalid quota configuration must be rejected")
        {
            DaemonError::InvalidConfig { field, .. } => field,
            error => panic!("expected invalid configuration error, got {error}"),
        }
    }

    fn verified_evidence() -> SliceDiskQuotaEvidence {
        let limits = limits();
        SliceDiskQuotaEvidence {
            writable_layer: WritableLayerQuotaEvidence {
                backend_supports_hard_quota: true,
                effective_limit_bytes: Some(limits.writable_layer_bytes),
                used_bytes: Some(64 * BYTES_PER_MEBIBYTE),
            },
            persistent_home: PersistentHomeQuotaEvidence {
                backend_supports_hard_quota: true,
                is_persistent: true,
                effective_limit_bytes: Some(limits.persistent_home_bytes),
                used_bytes: Some(256 * BYTES_PER_MEBIBYTE),
            },
        }
    }

    #[test]
    fn absent_pair_preserves_unconfigured_slice_behavior() {
        let limits = SliceDiskQuotaLimits::from_megabytes(None, None)
            .expect("unset limits should preserve legacy behavior");
        assert!(limits.is_none());
        require_verified_quotas(limits.as_ref(), None)
            .expect("unconfigured local and self-hosted slices should remain usable");
    }

    #[test]
    fn bounded_policy_requires_both_positive_caps() {
        assert!(SliceDiskQuotaLimits::from_megabytes(Some(512), None).is_err());
        assert!(SliceDiskQuotaLimits::from_megabytes(None, Some(2_048)).is_err());
        assert!(SliceDiskQuotaLimits::from_megabytes(Some(0), Some(2_048)).is_err());
        assert!(SliceDiskQuotaLimits::from_megabytes(Some(512), Some(0)).is_err());

        assert_eq!(
            invalid_config_field(Some(0), Some(2_048)),
            "slices.linux.disk_layer_mb"
        );
        assert_eq!(
            invalid_config_field(Some(512), Some(0)),
            "slices.linux.disk_home_mb"
        );
        assert_eq!(
            invalid_config_field(Some(512), None),
            "slices.linux.disk_home_mb"
        );
        assert_eq!(
            invalid_config_field(None, Some(2_048)),
            "slices.linux.disk_layer_mb"
        );

        let limits = SliceDiskQuotaLimits::from_megabytes(Some(512), Some(2_048))
            .expect("paired caps should be accepted");
        assert_eq!(
            limits,
            Some(SliceDiskQuotaLimits {
                writable_layer_bytes: 512 * BYTES_PER_MEBIBYTE,
                persistent_home_bytes: 2_048 * BYTES_PER_MEBIBYTE,
            })
        );

        let maximum = SliceDiskQuotaLimits::from_megabytes(Some(u32::MAX), Some(u32::MAX))
            .expect("u32 megabyte caps fit in the widened byte conversion");
        assert_eq!(
            maximum,
            Some(SliceDiskQuotaLimits {
                writable_layer_bytes: u64::from(u32::MAX) * BYTES_PER_MEBIBYTE,
                persistent_home_bytes: u64::from(u32::MAX) * BYTES_PER_MEBIBYTE,
            })
        );
    }

    #[test]
    fn verified_layer_and_persistent_home_caps_admit_existing_data() {
        let limits = limits();
        require_verified_quotas(Some(&limits), Some(&verified_evidence()))
            .expect("both hard caps and in-limit existing usage should admit");
    }

    #[test]
    fn bounded_mode_fails_closed_without_backend_and_effective_limit_proof() {
        let limits = limits();
        let error = require_verified_quotas(Some(&limits), None)
            .expect_err("sampling or missing backend proof must not admit a bounded slice");
        assert!(error.to_string().contains("cannot be verified"));
    }

    #[test]
    fn unsupported_layer_or_home_quota_backend_is_rejected() {
        let limits = limits();
        let mut evidence = verified_evidence();
        evidence.writable_layer.backend_supports_hard_quota = false;
        let error = require_verified_quotas(Some(&limits), Some(&evidence))
            .expect_err("unsupported writable-layer quota must fail closed");
        assert!(error.to_string().contains("writable-layer quota"));

        let mut evidence = verified_evidence();
        evidence.persistent_home.backend_supports_hard_quota = false;
        let error = require_verified_quotas(Some(&limits), Some(&evidence))
            .expect_err("unsupported home-volume quota must fail closed");
        assert!(error.to_string().contains("persistent-home quota"));
    }

    #[test]
    fn non_persistent_home_storage_is_rejected() {
        let limits = limits();
        let mut evidence = verified_evidence();
        evidence.persistent_home.is_persistent = false;
        let error = require_verified_quotas(Some(&limits), Some(&evidence))
            .expect_err("tmpfs cannot satisfy the persistent-home quota");
        assert!(error.to_string().contains("tmpfs"));
    }

    #[test]
    fn unreadable_or_changed_quota_readback_is_rejected() {
        let limits = limits();
        let mut evidence = verified_evidence();
        evidence.persistent_home.effective_limit_bytes = None;
        let error = require_verified_quotas(Some(&limits), Some(&evidence))
            .expect_err("missing home quota readback must fail closed");
        assert!(error.to_string().contains("could not be read back"));

        let mut evidence = verified_evidence();
        evidence.writable_layer.effective_limit_bytes = Some(limits.writable_layer_bytes - 1);
        let error = require_verified_quotas(Some(&limits), Some(&evidence))
            .expect_err("a mismatched layer limit must fail closed");
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn oversized_or_unmeasurable_existing_data_is_rejected_in_each_storage_class() {
        let limits = limits();
        let mut evidence = verified_evidence();
        evidence.writable_layer.used_bytes = Some(limits.writable_layer_bytes + 1);
        let error = require_verified_quotas(Some(&limits), Some(&evidence))
            .expect_err("oversized writable-layer data must not be truncated or admitted");
        assert!(error.to_string().contains("existing writable-layer data exceeds"));

        let mut evidence = verified_evidence();
        evidence.persistent_home.used_bytes = Some(limits.persistent_home_bytes + 1);
        let error = require_verified_quotas(Some(&limits), Some(&evidence))
            .expect_err("oversized home data must not be truncated or admitted");
        assert!(error.to_string().contains("existing persistent-home data exceeds"));

        let mut evidence = verified_evidence();
        evidence.persistent_home.used_bytes = None;
        assert!(require_verified_quotas(Some(&limits), Some(&evidence)).is_err());

        let mut evidence = verified_evidence();
        evidence.writable_layer.used_bytes = None;
        assert!(require_verified_quotas(Some(&limits), Some(&evidence)).is_err());
    }
}
