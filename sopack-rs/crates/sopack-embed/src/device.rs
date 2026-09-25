//! Execution device selection (SOPACK-1.0-PLAN.md §3.6): `Cpu` is 1.0's
//! default; `CoreMl`/`Cuda` are opt-in via the `coreml`/`cuda` cargo
//! features and are self-verifying — every non-CPU run embeds the
//! calibration fixture before touching real data (`engine::build_with_device_check`),
//! and `Auto` falls back to CPU on a low score while an explicit device
//! errors out.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Device {
    #[default]
    Cpu,
    CoreMl,
    Cuda,
    /// Try the best execution provider compiled in for this platform, self
    /// verify it against the calibration fixture, and fall back to `Cpu`
    /// with a warning if it fails. `Device::new` (this module) treats a
    /// literal `Auto` identically to `Cpu` — resolving *which* concrete
    /// device "auto" means is `engine::build_with_device_check`'s job, not
    /// a single session's.
    Auto,
}

impl Device {
    pub fn as_str(self) -> &'static str {
        match self {
            Device::Cpu => "cpu",
            Device::CoreMl => "coreml",
            Device::Cuda => "cuda",
            Device::Auto => "auto",
        }
    }
}

impl std::fmt::Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Device {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "cpu" => Ok(Device::Cpu),
            "coreml" => Ok(Device::CoreMl),
            "cuda" => Ok(Device::Cuda),
            "auto" => Ok(Device::Auto),
            other => Err(format!(
                "unknown device {other:?} (expected cpu, coreml, cuda or auto)"
            )),
        }
    }
}

/// The concrete, non-`Auto` devices worth trying, best first, given which
/// cargo features are compiled in and which platform this is — what
/// `Device::Auto` expands to. Pure and platform-checked via `cfg!` (not
/// `#[cfg]`) so it's callable — and testable — on every platform; the
/// feature/platform gating that actually matters for whether the EP can be
/// *used* lives in `execution_providers` below.
pub fn auto_candidates() -> Vec<Device> {
    let mut v = Vec::new();
    if cfg!(all(feature = "coreml", target_os = "macos")) {
        v.push(Device::CoreMl);
    }
    if cfg!(feature = "cuda") {
        v.push(Device::Cuda);
    }
    v.push(Device::Cpu);
    v
}

/// Execution providers to register on a `Session` for `device`, given which
/// cargo features are compiled in. `Cpu` needs none — ONNX Runtime always
/// has a CPU EP. Errors for `CoreMl`/`Cuda` when the matching feature
/// wasn't compiled in, or for `CoreMl` off macOS (inert there even with the
/// feature on, per SOPACK-1.0-PLAN.md M2's "coreml ... inert elsewhere").
pub fn execution_providers(
    device: Device,
) -> crate::error::Result<Vec<ort::ep::ExecutionProviderDispatch>> {
    match device {
        Device::Cpu | Device::Auto => Ok(vec![]),
        Device::CoreMl => coreml_providers(),
        Device::Cuda => cuda_providers(),
    }
}

#[cfg(all(feature = "coreml", target_os = "macos"))]
fn coreml_providers() -> crate::error::Result<Vec<ort::ep::ExecutionProviderDispatch>> {
    use ort::ep::coreml::ComputeUnits;
    // `CPUAndGPU`, never an ANE-only unit set: the brief is explicit that
    // CoreML must prefer fp32 compute (CPU+GPU), leaving the
    // "is ANE fp16 acceptable" question entirely to the calibration gate,
    // per machine, rather than opting into it here.
    Ok(vec![ort::ep::CoreML::default()
        .with_compute_units(ComputeUnits::CPUAndGPU)
        .build()])
}

#[cfg(not(all(feature = "coreml", target_os = "macos")))]
fn coreml_providers() -> crate::error::Result<Vec<ort::ep::ExecutionProviderDispatch>> {
    #[cfg(not(feature = "coreml"))]
    {
        Err(crate::error::EmbedError::DeviceNotCompiled {
            device: "coreml".into(),
            feature: "coreml",
        })
    }
    #[cfg(feature = "coreml")]
    {
        Err(crate::error::EmbedError::DeviceNotAvailable {
            device: "coreml (macOS only)".into(),
        })
    }
}

#[cfg(feature = "cuda")]
fn cuda_providers() -> crate::error::Result<Vec<ort::ep::ExecutionProviderDispatch>> {
    Ok(vec![ort::ep::CUDA::default().build()])
}

#[cfg(not(feature = "cuda"))]
fn cuda_providers() -> crate::error::Result<Vec<ort::ep::ExecutionProviderDispatch>> {
    Err(crate::error::EmbedError::DeviceNotCompiled {
        device: "cuda".into(),
        feature: "cuda",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_devices_case_insensitively() {
        assert_eq!("cpu".parse::<Device>().unwrap(), Device::Cpu);
        assert_eq!("CoreML".parse::<Device>().unwrap(), Device::CoreMl);
        assert_eq!("CUDA".parse::<Device>().unwrap(), Device::Cuda);
        assert_eq!("auto".parse::<Device>().unwrap(), Device::Auto);
    }

    #[test]
    fn rejects_unknown_device() {
        let err = "tpu".parse::<Device>().unwrap_err();
        assert!(err.contains("tpu"));
    }

    #[test]
    fn default_device_is_cpu() {
        assert_eq!(Device::default(), Device::Cpu);
    }

    #[test]
    fn cpu_and_auto_need_no_execution_providers() {
        assert!(execution_providers(Device::Cpu).unwrap().is_empty());
        assert!(execution_providers(Device::Auto).unwrap().is_empty());
    }

    #[test]
    fn auto_candidates_always_ends_in_cpu() {
        let candidates = auto_candidates();
        assert_eq!(candidates.last(), Some(&Device::Cpu));
    }

    #[cfg(not(feature = "cuda"))]
    #[test]
    fn cuda_without_the_feature_is_a_clear_error_not_a_silent_cpu_fallback() {
        let err = execution_providers(Device::Cuda).unwrap_err();
        assert!(matches!(
            err,
            crate::error::EmbedError::DeviceNotCompiled { .. }
        ));
    }

    #[cfg(not(feature = "coreml"))]
    #[test]
    fn coreml_without_the_feature_is_a_clear_error() {
        let err = execution_providers(Device::CoreMl).unwrap_err();
        assert!(matches!(
            err,
            crate::error::EmbedError::DeviceNotCompiled { .. }
        ));
    }
}
