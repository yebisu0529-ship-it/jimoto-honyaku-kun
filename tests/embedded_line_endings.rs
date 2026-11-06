//! Files embedded with `include_bytes!` are hashed into installation records,
//! so every checkout has to produce the same bytes for them.
use std::{fs, path::Path};

#[test]
fn embedded_scripts_keep_lf_line_endings_in_every_checkout() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let attributes = fs::read_to_string(root.join(".gitattributes")).unwrap();
    let pinned: Vec<&str> = attributes
        .lines()
        .filter(|line| line.split_whitespace().any(|item| item == "eol=lf"))
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    let mut embedded = Vec::new();
    for entry in fs::read_dir(root.join("src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        for part in source.split("include_bytes!(\"../").skip(1) {
            let file = &part[..part.find('"').unwrap()];
            if file.starts_with("scripts/") {
                embedded.push(file.to_owned());
            }
        }
    }
    embedded.sort();
    embedded.dedup();
    assert!(embedded.len() >= 7, "embedded scripts: {embedded:?}");
    for file in &embedded {
        assert!(
            pinned.contains(&file.as_str()),
            "{file} is embedded but .gitattributes does not pin it to LF"
        );
        let bytes = fs::read(root.join(file)).unwrap();
        assert!(
            !bytes.windows(2).any(|pair| pair == b"\r\n"),
            "{file} has CRLF line endings in this checkout"
        );
    }
}
