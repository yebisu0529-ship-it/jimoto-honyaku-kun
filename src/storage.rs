//! Shared storage selection for the GUI and setup helper.
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// The folder the app runs from: `PLAMO_APP_DIR`, which the launcher sets to
/// the unzipped folder, or the exe's folder. A build under `target/release` or
/// `target/debug` belongs to its project folder.
pub fn app_root() -> PathBuf {
    if let Some(root) = std::env::var_os("PLAMO_APP_DIR") {
        return root.into();
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        if dir
            .file_name()
            .is_some_and(|n| n == "release" || n == "debug")
            && let Some(target) = dir.parent()
            && target.file_name().is_some_and(|n| n == "target")
            && let Some(project) = target.parent()
            && project.join("Cargo.toml").exists()
        {
            return project.into();
        }
        return dir.into();
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Where settings, history, models and runtimes are kept: `PLAMO_DATA_DIR`
/// when it is an absolute path, otherwise `data` in the app's folder, so the
/// unzipped folder holds everything. A PC prepared by a version that kept them
/// under Local AppData goes on using that folder, so nothing is fetched again.
/// Decided once, so every part of the app uses the same folder.
pub fn data_root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let earlier = directories::ProjectDirs::from("jp", "LocalTools", "PlamoTranslate")
            .map(|p| p.data_local_dir().to_path_buf());
        resolve(
            std::env::var_os("PLAMO_DATA_DIR").as_deref().map(Path::new),
            &app_root().join("data"),
            earlier.as_deref(),
        )
    })
    .clone()
}

fn resolve(selected: Option<&Path>, portable: &Path, earlier: Option<&Path>) -> PathBuf {
    if let Some(selected) = selected.filter(|p| p.is_absolute()) {
        return selected.to_path_buf();
    }
    if portable.is_dir() {
        return portable.to_path_buf();
    }
    if let Some(earlier) = earlier.filter(|p| p.is_dir()) {
        return earlier.to_path_buf();
    }
    // A folder the user cannot write to, such as Program Files, keeps the data
    // under Local AppData.
    match (std::fs::create_dir_all(portable), earlier) {
        (Err(_), Some(earlier)) => earlier.to_path_buf(),
        _ => portable.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_data_directory_is_shared_and_relative_overrides_are_ignored() {
        let base = tempfile::tempdir().unwrap();
        let portable = base.path().join("app/data");
        let isolated = base.path().join("isolated");
        assert_eq!(resolve(Some(&isolated), &portable, None), isolated);
        assert_eq!(
            resolve(Some(Path::new("relative")), &portable, None),
            portable
        );
    }

    #[test]
    fn a_new_pc_keeps_everything_in_the_unzipped_folder() {
        let base = tempfile::tempdir().unwrap();
        let portable = base.path().join("app/data");
        let earlier = base.path().join("LocalAppData/PlamoTranslate/data");
        assert_eq!(resolve(None, &portable, Some(&earlier)), portable);
        assert!(portable.is_dir(), "the folder is made at once");
        assert!(!earlier.exists(), "Local AppData is not touched");
    }

    #[test]
    fn a_pc_prepared_before_keeps_its_folder_until_the_unzipped_one_has_data() {
        let base = tempfile::tempdir().unwrap();
        let portable = base.path().join("app/data");
        let earlier = base.path().join("LocalAppData/PlamoTranslate/data");
        std::fs::create_dir_all(&earlier).unwrap();
        assert_eq!(resolve(None, &portable, Some(&earlier)), earlier);
        assert!(!portable.exists(), "no empty folder is left in the app");
        std::fs::create_dir_all(&portable).unwrap();
        assert_eq!(resolve(None, &portable, Some(&earlier)), portable);
    }

    #[test]
    fn an_app_folder_that_cannot_be_written_falls_back_to_local_app_data() {
        let base = tempfile::tempdir().unwrap();
        let file = base.path().join("not-a-folder");
        std::fs::write(&file, b"").unwrap();
        let earlier = base.path().join("LocalAppData/PlamoTranslate/data");
        assert_eq!(resolve(None, &file.join("data"), Some(&earlier)), earlier);
    }
}
