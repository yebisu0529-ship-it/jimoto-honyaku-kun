//! Run before packaging a release. Asks every file the app pins for its first byte.
use plamo_translate::download_check;

fn main() -> anyhow::Result<()> {
    println!("アプリが固定しているファイルを、先頭の1バイトだけ取得して確かめます。");
    let (mut checked, mut failed) = (0, 0);
    download_check::run(|check| {
        checked += 1;
        match &check.result {
            Ok(host) => println!("OK  {}  {}  ({host})", check.group, check.name),
            Err(error) => {
                failed += 1;
                println!("NG  {}  {}  {error:#}", check.group, check.name);
            }
        }
    })?;
    anyhow::ensure!(
        failed == 0,
        "{checked}件中{failed}件を取得できません。固定先を直してから配布してください。"
    );
    println!("{checked}件すべてを取得できます。");
    Ok(())
}
