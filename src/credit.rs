//! 訳文を外へ出すときのクレジット。
//! PLaMo Community License 第5条1項は、生成物の公表時にモデルの生成物であることの明示を条件にしている。

pub const CREDIT: &str = "translated by PLaMo\nBuilt with PLaMo";

/// 画面に小さく出す1行版。
pub const LABEL: &str = "translated by PLaMo · Built with PLaMo";

/// コピー・保存など、公開されうる経路の訳文にクレジットを付ける。
pub fn published(text: &str) -> String {
    let text = text.trim_end();
    if text.is_empty() || text.contains(CREDIT) {
        return text.to_owned();
    }
    format!("{text}\n\n---\n{CREDIT}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_credit_once() {
        let once = published("こんにちは");
        assert_eq!(once, format!("こんにちは\n\n---\n{CREDIT}"));
        assert_eq!(published(&once), once);
        assert_eq!(published("  \n"), "");
    }
}
