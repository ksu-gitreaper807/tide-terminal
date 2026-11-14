//! The GUI's driver policy must never leak into programs run by the terminal.
#[cfg(target_os = "linux")]
#[test]
fn supervised_programs_receive_the_original_gpu_environment() {
    use std::process::{Command, Stdio};
    let binary = std::env::var("TIDE_APPEND_TEST_BINARY")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_tide-terminal").into());
    let dir = tempfile::tempdir().unwrap();
    for original in [None, Some("/user/custom driver.json")] {
        let target = dir.path().join("environment.json");
        let saved = serde_json::json!([
            ["VK_DRIVER_FILES", original.map(|s| s.as_bytes())],
            ["__EGL_VENDOR_LIBRARY_FILENAMES", null]
        ]);
        let status = Command::new(&binary)
            .args(["run", "--private", "--json", "--", "/usr/bin/python3", "-c",
                "import json,os,sys;json.dump({k:os.environ.get(k) for k in ['VK_DRIVER_FILES','__EGL_VENDOR_LIBRARY_FILENAMES','_TIDE_GUI_DRIVER_ENV']},open(sys.argv[1],'w'))"])
            .arg(&target)
            .env("_TIDE_GUI_DRIVER_ENV", saved.to_string())
            .env("VK_DRIVER_FILES", "/gui/filtered-driver.json")
            .env("__EGL_VENDOR_LIBRARY_FILENAMES", "/gui/mesa.json")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let received: serde_json::Value =
            serde_json::from_slice(&std::fs::read(target).unwrap()).unwrap();
        assert_eq!(received["VK_DRIVER_FILES"], serde_json::json!(original));
        assert!(received["__EGL_VENDOR_LIBRARY_FILENAMES"].is_null());
        assert!(received["_TIDE_GUI_DRIVER_ENV"].is_null());
    }
}
