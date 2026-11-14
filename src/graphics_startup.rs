//! Keep hybrid GPU discovery local to the GUI, without changing child programs.
use crate::config::GpuMode;
use portable_pty::CommandBuilder;

const SAVED: &str = "_TIDE_GUI_DRIVER_ENV";
const KEYS: [&str; 2] = ["VK_DRIVER_FILES", "__EGL_VENDOR_LIBRARY_FILENAMES"];
type Saved = Vec<(String, Option<Vec<u8>>)>;

#[cfg(target_os = "linux")]
fn driver_plan(
    mode: GpuMode,
    vendors: &[String],
    manifests: &[std::path::PathBuf],
    mesa_egl: Option<std::path::PathBuf>,
    explicit: impl Fn(&str) -> bool,
) -> Option<Vec<(&'static str, std::ffi::OsString)>> {
    if mode != GpuMode::Integrated
        || !vendors.iter().any(|v| v == "0x10de")
        || [
            "WGPU_BACKEND",
            "VK_DRIVER_FILES",
            "VK_ICD_FILENAMES",
            "VK_ADD_DRIVER_FILES",
            "VK_LOADER_DRIVERS_SELECT",
            "VK_LOADER_DRIVERS_DISABLE",
            "VK_LOADER_DRIVERS_ENABLE",
            "__NV_PRIME_RENDER_OFFLOAD",
            "__VK_LAYER_NV_optimus",
        ]
        .into_iter()
        .any(&explicit)
    {
        return None;
    }
    let paths: Vec<_> = manifests
        .iter()
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            name.ends_with(".json")
                && ((vendors.iter().any(|v| v == "0x8086") && name.starts_with("intel_"))
                    || (vendors.iter().any(|v| v == "0x1002") && name.starts_with("radeon_")))
        })
        .collect();
    if paths.is_empty() || (mesa_egl.is_none() && !explicit(KEYS[1])) {
        return None;
    }
    let mut plan = vec![(KEYS[0], std::env::join_paths(paths).ok()?)];
    if !explicit(KEYS[1])
        && let Some(path) = mesa_egl
    {
        plan.push((KEYS[1], path.into_os_string()));
    }
    Some(plan)
}

/// Re-exec before opening a window or initializing any graphics driver. Using a
/// new process avoids changing process-global environment around driver threads.
pub fn prepare(mode: GpuMode) -> anyhow::Result<()> {
    if mode != GpuMode::Integrated {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        use std::{
            os::unix::{ffi::OsStrExt, process::CommandExt},
            path::Path,
        };
        if std::env::var_os(SAVED).is_some() {
            return Ok(());
        }
        let vendors: Vec<_> = std::fs::read_dir("/sys/class/drm")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| {
                entry.file_name().to_str().is_some_and(|name| {
                    name.strip_prefix("card")
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                })
            })
            .filter_map(|entry| std::fs::read_to_string(entry.path().join("device/vendor")).ok())
            .map(|v| v.trim().to_lowercase())
            .collect();
        if !vendors.iter().any(|vendor| vendor == "0x10de") {
            return Ok(());
        }
        let mut manifests: Vec<_> = [
            "/etc/vulkan/icd.d",
            "/usr/local/share/vulkan/icd.d",
            "/usr/share/vulkan/icd.d",
        ]
        .into_iter()
        .flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .collect();
        manifests.sort();
        let mesa = [
            "/etc/glvnd/egl_vendor.d/50_mesa.json",
            "/usr/local/share/glvnd/egl_vendor.d/50_mesa.json",
            "/usr/share/glvnd/egl_vendor.d/50_mesa.json",
        ]
        .into_iter()
        .map(Path::new)
        .find(|p| p.is_file())
        .map(Path::to_owned);
        if let Some(plan) = driver_plan(mode, &vendors, &manifests, mesa, |key| {
            std::env::var_os(key).is_some()
        }) {
            let saved: Saved = plan
                .iter()
                .map(|(key, _)| {
                    (
                        (*key).into(),
                        std::env::var_os(key).map(|v| v.as_bytes().to_vec()),
                    )
                })
                .collect();
            let error = std::process::Command::new(std::env::current_exe()?)
                .args(std::env::args_os().skip(1))
                .envs(plan)
                .env(SAVED, serde_json::to_string(&saved)?)
                .exec();
            return Err(error.into());
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = mode;
    Ok(())
}

/// The terminal's loader policy must not constrain games, CUDA or nested Tide
/// instances launched by its shells. Explicit per-command overrides win later.
pub fn restore_child(cmd: &mut CommandBuilder) -> anyhow::Result<()> {
    restore(cmd, std::env::var(SAVED).ok().as_deref())
}

fn restore(cmd: &mut CommandBuilder, saved: Option<&str>) -> anyhow::Result<()> {
    cmd.env_remove(SAVED);
    if let Some(saved) = saved {
        let saved: Saved = serde_json::from_str(saved)?;
        for (key, value) in saved {
            anyhow::ensure!(
                KEYS.contains(&key.as_str()),
                "Invalid Tide GUI driver environment"
            );
            if let Some(value) = value {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    cmd.env(key, std::ffi::OsString::from_vec(value));
                }
                #[cfg(not(unix))]
                cmd.env(key, String::from_utf8(value)?);
            } else {
                cmd.env_remove(key);
            }
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn hybrid_policy_keeps_matching_mesa_drivers_and_gl_fallback() {
        let files: Vec<_> = [
            "intel_icd.json",
            "intel_hasvk_icd.json",
            "radeon_icd.json",
            "nvidia_icd.json",
        ]
        .map(PathBuf::from)
        .into();
        for (vendor, expected) in [
            ("0x8086", vec![files[0].clone(), files[1].clone()]),
            ("0x1002", vec![files[2].clone()]),
        ] {
            let plan = driver_plan(
                GpuMode::Integrated,
                &[vendor.into(), "0x10de".into()],
                &files,
                Some("50_mesa.json".into()),
                |_| false,
            )
            .unwrap();
            assert_eq!(
                std::env::split_paths(&plan[0].1).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(plan[1], (KEYS[1], "50_mesa.json".into()));
            assert!(!plan.iter().any(|(key, _)| *key == "WGPU_BACKEND"));
        }
    }

    #[test]
    fn explicit_choices_and_unknown_hardware_keep_existing_discovery() {
        let vendors = vec!["0x8086".into(), "0x10de".into()];
        let files = vec![PathBuf::from("intel_icd.json")];
        for mode in [GpuMode::Auto, GpuMode::Discrete, GpuMode::Software] {
            assert!(driver_plan(mode, &vendors, &files, None, |_| false).is_none());
        }
        for key in [
            "WGPU_BACKEND",
            "VK_DRIVER_FILES",
            "VK_ICD_FILENAMES",
            "VK_ADD_DRIVER_FILES",
            "VK_LOADER_DRIVERS_SELECT",
            "VK_LOADER_DRIVERS_DISABLE",
            "VK_LOADER_DRIVERS_ENABLE",
            "__NV_PRIME_RENDER_OFFLOAD",
            "__VK_LAYER_NV_optimus",
        ] {
            assert!(
                driver_plan(GpuMode::Integrated, &vendors, &files, None, |k| k == key).is_none(),
                "{key}"
            );
        }
        assert!(
            driver_plan(
                GpuMode::Integrated,
                &["0x8086".into()],
                &files,
                None,
                |_| false
            )
            .is_none()
        );
        assert!(
            driver_plan(
                GpuMode::Integrated,
                &["0x1234".into(), "0x10de".into()],
                &files,
                None,
                |_| false
            )
            .is_none()
        );
        assert!(driver_plan(GpuMode::Integrated, &vendors, &[], None, |_| false).is_none());
        let plan = driver_plan(
            GpuMode::Integrated,
            &vendors,
            &files,
            Some("50_mesa.json".into()),
            |k| k == KEYS[1],
        )
        .unwrap();
        assert_eq!(plan.len(), 1);
    }

    #[test]
    fn child_environment_restores_original_bytes_and_removes_the_bootstrap_marker() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let mut cmd = CommandBuilder::new("true");
        cmd.env(KEYS[0], "gui-only");
        cmd.env(KEYS[1], "gui-only");
        cmd.env(SAVED, "private-marker");
        let original = b"original-\xff";
        let saved: Saved = vec![
            (KEYS[0].into(), Some(original.to_vec())),
            (KEYS[1].into(), None),
        ];
        restore(&mut cmd, Some(&serde_json::to_string(&saved).unwrap())).unwrap();
        assert_eq!(cmd.get_env(KEYS[0]), Some(OsStr::from_bytes(original)));
        assert!(cmd.get_env(KEYS[1]).is_none());
        assert!(cmd.get_env(SAVED).is_none());
        assert!(restore(&mut cmd, Some("[[\"PATH\",null]]")).is_err());
    }
}
