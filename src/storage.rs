//! Shared storage selection for the GUI and setup helper.
use std::path::{Path, PathBuf};

pub fn data_root() -> PathBuf {
    let normal = directories::ProjectDirs::from("jp", "LocalTools", "PlamoTranslate")
        .map(|p| p.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("userdata"));
    resolve(
        std::env::var_os("PLAMO_DATA_DIR").as_deref().map(Path::new),
        normal,
    )
}

fn resolve(selected: Option<&Path>, normal: PathBuf) -> PathBuf {
    selected
        .filter(|p| p.is_absolute())
        .map(Path::to_path_buf)
        .unwrap_or(normal)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_data_directory_is_shared_and_relative_overrides_are_ignored() {
        let normal = std::env::temp_dir().join("normal-jimoto");
        let isolated = std::env::temp_dir().join("isolated-jimoto");
        assert_eq!(resolve(Some(&isolated), normal.clone()), isolated);
        assert_eq!(resolve(Some(Path::new("relative")), normal.clone()), normal);
    }
}
