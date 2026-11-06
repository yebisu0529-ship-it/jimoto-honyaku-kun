//! Pre-release check that every file the app pins can still be downloaded.
use crate::{
    indra_setup,
    installer::{self, Artifact, OfficialProbe},
    model_catalog, nemotron_setup, paddle_setup, punctuation_setup, speech_setup,
};
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashSet;

pub struct Check {
    pub group: &'static str,
    pub name: String,
    /// The host that served the first byte, or why the installer could not download the file.
    pub result: Result<String>,
}

/// Checks each pinned URL once and reports it as soon as it is checked.
pub fn run(mut report: impl FnMut(&Check)) -> Result<()> {
    let probe = OfficialProbe::new()?;
    let mut seen = HashSet::new();
    for (group, artifact) in pinned()? {
        if seen.insert(artifact.url.clone()) {
            report(&Check {
                group,
                result: probe.first_byte(&artifact),
                name: artifact.name,
            });
        }
    }
    Ok(())
}

fn pinned() -> Result<Vec<(&'static str, Artifact)>> {
    let catalog = model_catalog::bundled()?;
    let mut backends: Vec<&str> = catalog
        .models
        .iter()
        .flat_map(|model| model.backends.iter().map(|backend| backend.as_str()))
        .collect();
    backends.sort_unstable();
    backends.dedup();
    let mut pins: Vec<_> = backends
        .into_iter()
        .flat_map(installer::runtime_assets)
        .map(|artifact| ("翻訳の実行環境", artifact))
        .collect();
    pins.extend(catalog.models.into_iter().map(|model| {
        let artifact = Artifact {
            url: installer::hugging_face_url(&model.repo, &model.revision, &model.filename),
            name: model.filename,
            bytes: model.bytes,
            sha256: model.sha256,
        };
        ("翻訳モデル", artifact)
    }));
    for (group, artifacts) in [
        ("音声認識", audio()?),
        ("音声認識（Nemotron）", nemotron_setup::downloads()?),
        ("読み上げ", speech_setup::downloads()?),
        ("英語の文章区切り補助", punctuation_setup::downloads()?),
        ("文字認識（Indra）", indra_setup::downloads()?),
        ("文字認識（PaddleOCR）", paddle_setup::downloads()?),
    ] {
        pins.extend(artifacts.into_iter().map(|artifact| (group, artifact)));
    }
    Ok(pins)
}

// setup-audio.ps1 downloads these itself, so no Rust setup reads this file.
fn audio() -> Result<Vec<Artifact>> {
    #[derive(Deserialize)]
    struct Metadata {
        uv: Tool,
        qwen: Model,
        whisper: Model,
    }
    #[derive(Deserialize)]
    struct Tool {
        filename: String,
        url: String,
        bytes: u64,
        sha256: String,
    }
    #[derive(Deserialize)]
    struct Model {
        repo: String,
        revision: String,
        model_file: String,
        model_bytes: u64,
        model_sha256: String,
    }
    let data: Metadata = serde_json::from_str(include_str!("../scripts/audio-runtime.json"))?;
    let uv = Artifact {
        name: data.uv.filename,
        url: data.uv.url,
        bytes: data.uv.bytes,
        sha256: data.uv.sha256,
    };
    let models = [data.qwen, data.whisper].into_iter().map(|model| Artifact {
        url: installer::hugging_face_url(&model.repo, &model.revision, &model.model_file),
        name: model.model_file,
        bytes: model.model_bytes,
        sha256: model.model_sha256,
    });
    Ok(std::iter::once(uv).chain(models).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_catalog::Backend;
    use serde_json::Value;
    use std::{fs, path::Path};

    // A manifest pins a file by URL and size, or by Hugging Face repo, commit and file.
    fn pinned_urls(value: &Value, urls: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                let text = |key: &str| map.get(key).and_then(Value::as_str);
                if let Some(url) = text("url").filter(|_| map.contains_key("bytes")) {
                    urls.push(url.to_owned());
                }
                if let (Some(repo), Some(revision), Some(file)) = (
                    text("repo"),
                    text("revision"),
                    text("filename").or(text("model_file")),
                ) {
                    urls.push(installer::hugging_face_url(repo, revision, file));
                }
                map.values().for_each(|value| pinned_urls(value, urls));
            }
            Value::Array(values) => values.iter().for_each(|value| pinned_urls(value, urls)),
            _ => {}
        }
    }

    #[test]
    fn every_pinned_download_is_checked() {
        let checked: HashSet<String> = pinned()
            .unwrap()
            .into_iter()
            .map(|(_, artifact)| artifact.url)
            .collect();
        let mut manifests = 0;
        let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");
        for entry in fs::read_dir(scripts).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let mut urls = Vec::new();
            let manifest = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            pinned_urls(&manifest, &mut urls);
            manifests += usize::from(!urls.is_empty());
            for url in urls {
                assert!(
                    checked.contains(&url),
                    "{} pins {url}, but the download check skips it",
                    path.display()
                );
            }
        }
        assert!(manifests >= 7, "only {manifests} manifests pin downloads");
        for backend in [Backend::Cpu, Backend::Vulkan, Backend::Cuda] {
            for artifact in installer::runtime_assets(backend.as_str()) {
                assert!(checked.contains(&artifact.url), "{}", artifact.name);
            }
        }
    }
}
