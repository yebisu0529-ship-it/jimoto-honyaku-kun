//! Windows PowerShell 5.1 reads a script that has no byte order mark in the
//! system ANSI code page (932 on Japanese Windows). Any non-ASCII text in a
//! script it runs, such as the `Talk風Live翻訳.cmd` file name in `package.ps1`,
//! must therefore be saved as UTF-8 with a BOM.

use std::path::Path;

#[test]
fn powershell_scripts_with_non_ascii_text_keep_a_utf8_bom() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    for dir in [root.to_path_buf(), root.join("scripts")] {
        for entry in std::fs::read_dir(&dir).expect("read script directory") {
            let path = entry.expect("directory entry").path();
            if path.extension().is_none_or(|extension| extension != "ps1") {
                continue;
            }
            let bytes = std::fs::read(&path).expect("read script");
            checked += 1;
            if bytes.iter().any(|byte| *byte >= 0x80) {
                assert!(
                    bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
                    "{} has non-ASCII text but no UTF-8 BOM; Windows PowerShell 5.1 would misread it",
                    path.display()
                );
            }
        }
    }
    assert!(
        checked >= 6,
        "expected the repository's PowerShell scripts, found {checked}"
    );
}
