//! Preserve the source and publish file translations only as complete files.
use anyhow::{Context, Result, ensure};
use std::{
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn check_destination(input: &Path, output: &Path) -> Result<()> {
    let exists = output.try_exists().context("出力先を確認できません。")?;
    ensure!(
        !exists
            || !same_file::is_same_file(input, output)
                .context("元のファイルと出力先を確認できません。")?,
        "元のファイルを保つため、別のファイル名を選んでください。"
    );
    Ok(())
}

pub(super) fn write_translation(
    input: &Path,
    output: &Path,
    content: &str,
    cancelled: &AtomicBool,
) -> Result<()> {
    ensure!(!cancelled.load(Ordering::Relaxed), "中止しました。");
    check_destination(input, output)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .context("出力先に一時ファイルを作成できません。")?;
    temporary
        .write_all(content.as_bytes())
        .context("訳文を保存できません。")?;
    temporary
        .as_file()
        .sync_all()
        .context("訳文をディスクに書き込めません。")?;
    ensure!(!cancelled.load(Ordering::Relaxed), "中止しました。");
    // Translation may take minutes; the destination can change while it runs.
    check_destination(input, output)?;
    temporary
        .persist(output)
        .map_err(|error| error.error)
        .context("訳文の保存を確定できません。出力先は変更していません。")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn regression_alias_and_hard_link_cannot_overwrite_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.txt");
        fs::write(&input, "original source").unwrap();
        let hard_link = dir.path().join("alias.txt");
        fs::hard_link(&input, &hard_link).unwrap();
        for output in [input.clone(), dir.path().join("./source.txt"), hard_link] {
            assert!(check_destination(&input, &output).is_err());
            assert!(
                write_translation(&input, &output, "translated", &AtomicBool::new(false)).is_err()
            );
            assert_eq!(fs::read_to_string(&input).unwrap(), "original source");
        }
        #[cfg(windows)]
        assert!(check_destination(&input, &dir.path().join("SOURCE.TXT")).is_err());
    }

    #[test]
    fn regression_destination_is_checked_again_when_publishing() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.txt");
        let output = dir.path().join("result.txt");
        fs::write(&input, "original source").unwrap();
        check_destination(&input, &output).unwrap();
        fs::hard_link(&input, &output).unwrap();
        assert!(write_translation(&input, &output, "translated", &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read_to_string(&input).unwrap(), "original source");
    }

    #[test]
    fn successful_publication_replaces_output_and_preserves_source() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.txt");
        let output = dir.path().join("result.txt");
        fs::write(&input, "original source").unwrap();
        for translation in ["first translation", "次の訳文"] {
            write_translation(&input, &output, translation, &AtomicBool::new(false)).unwrap();
            assert_eq!(fs::read_to_string(&input).unwrap(), "original source");
            assert_eq!(fs::read_to_string(&output).unwrap(), translation);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        }
    }

    #[test]
    fn cancellation_and_failed_commit_leave_existing_data_and_no_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.txt");
        let output = dir.path().join("result.txt");
        fs::write(&input, "original source").unwrap();
        fs::write(&output, "previous translation").unwrap();
        assert!(write_translation(&input, &output, "new", &AtomicBool::new(true)).is_err());
        assert_eq!(fs::read_to_string(&output).unwrap(), "previous translation");
        let blocked = dir.path().join("blocked.txt");
        fs::create_dir(&blocked).unwrap();
        fs::write(blocked.join("keep.txt"), "keep").unwrap();
        assert!(write_translation(&input, &blocked, "new", &AtomicBool::new(false)).is_err());
        assert_eq!(
            fs::read_to_string(blocked.join("keep.txt")).unwrap(),
            "keep"
        );
        assert_eq!(fs::read_to_string(&input).unwrap(), "original source");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
    }
}
