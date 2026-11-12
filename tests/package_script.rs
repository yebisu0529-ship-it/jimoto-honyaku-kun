//! `scripts/package.ps1` packages release binaries that the tests do not
//! build, so these tests run it on a copy of its file list with stand-in files.

use std::{fs, path::Path, process::Command};

#[test]
fn the_release_files_are_named_after_the_repository_and_the_app_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = root.join("scripts").join("package.ps1");
    let listed = fs::read_to_string(&script).expect("read package.ps1");
    let app = tempfile::tempdir().expect("temporary app folder");
    for part in listed.split("Source = '").skip(1) {
        let source = &part[..part.find('\'').expect("quoted source")];
        let path = app.path().join(source);
        fs::create_dir_all(path.parent().expect("parent folder")).expect("create folder");
        fs::write(&path, source).expect("write stand-in file");
    }
    fs::copy(root.join("start.ps1"), app.path().join("start.ps1")).expect("copy launcher");
    let out = app.path().join("dist");
    let run = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg("-AppRoot")
        .arg(app.path())
        .arg("-OutDir")
        .arg(&out)
        .output()
        .expect("start Windows PowerShell");
    assert!(
        run.status.success(),
        "package.ps1 failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let version = env!("CARGO_PKG_VERSION");
    let name = format!("talk-style-live-translation-{version}-win-x64");
    let mut made: Vec<String> = fs::read_dir(&out)
        .expect("read output folder")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    made.sort();
    assert_eq!(
        made,
        [
            format!("{name}.manifest.json"),
            format!("{name}.zip"),
            format!("{name}.zip.sha256"),
        ]
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join(format!("{name}.manifest.json"))).unwrap())
            .expect("manifest is JSON");
    assert_eq!(manifest["app"], "talk-style-live-translation");
    assert_eq!(manifest["version"], version);
}
