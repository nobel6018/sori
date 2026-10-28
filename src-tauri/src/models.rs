use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::engine::ModelPaths;

/// scripts/install-models.{sh,ps1}가 받는 파일 이름. 스크립트와 맞춰야 한다
pub const SEGMENTATION_FILE: &str = "pyannote-segmentation-3-0.onnx";
pub const EMBEDDING_FILE: &str = "3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx";

pub struct WhisperModel {
    pub id: &'static str,
    pub file: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub size_bytes: u64,
    /// install-models 스크립트의 --whisper 값
    pub script_arg: &'static str,
}

pub const WHISPER_MODELS: [WhisperModel; 2] = [
    WhisperModel {
        id: "large-v3-turbo",
        file: "ggml-large-v3-turbo.bin",
        label: "large-v3-turbo",
        description: "빠르고 정확도도 충분합니다. 대부분 이걸로 충분합니다",
        size_bytes: 1_624_555_275,
        script_arg: "turbo",
    },
    WhisperModel {
        id: "large-v3",
        file: "ggml-large-v3.bin",
        label: "large-v3",
        description: "조금 더 정확하지만 받아 적는 데 약 2배 걸립니다",
        size_bytes: 3_095_033_483,
        script_arg: "large-v3",
    },
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhisperOption {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub size_bytes: u64,
    pub script_arg: &'static str,
    /// 설치돼 있으면 실제 경로
    pub path: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub models_dir: PathBuf,
    pub whisper: Vec<WhisperOption>,
    pub segmentation: Option<PathBuf>,
    pub embedding: Option<PathBuf>,
}

/// 모델을 찾는 위치. 스크립트가 받는 앱 데이터 폴더를 먼저 보고,
/// sori 이전부터 쓰던 공용 위치(~/whisper-models, ~/sherpa-onnx/models)도 본다
fn candidates(models_dir: &Path, home: &Path, kind: &str, file: &str) -> Vec<PathBuf> {
    let mut paths = vec![models_dir.join(file)];
    match kind {
        "whisper" => paths.push(home.join("whisper-models").join(file)),
        "segmentation" => paths.push(
            home.join("sherpa-onnx/models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx"),
        ),
        // 같은 파일인데 예전에 이름을 줄여 저장한 경우가 있다
        "embedding" => {
            paths.push(home.join("sherpa-onnx/models").join(file));
            paths.push(home.join("sherpa-onnx/models/3dspeaker_eres2net_base_zh-cn_16k.onnx"));
        }
        _ => {}
    }
    paths
}

fn find(models_dir: &Path, home: &Path, kind: &str, file: &str) -> Option<PathBuf> {
    candidates(models_dir, home, kind, file).into_iter().find(|p| p.is_file())
}

pub fn catalog(models_dir: &Path, home: &Path) -> Catalog {
    Catalog {
        models_dir: models_dir.to_path_buf(),
        whisper: WHISPER_MODELS
            .iter()
            .map(|m| WhisperOption {
                id: m.id,
                label: m.label,
                description: m.description,
                size_bytes: m.size_bytes,
                script_arg: m.script_arg,
                path: find(models_dir, home, "whisper", m.file),
            })
            .collect(),
        segmentation: find(models_dir, home, "segmentation", SEGMENTATION_FILE),
        embedding: find(models_dir, home, "embedding", EMBEDDING_FILE),
    }
}

/// 설정이 없을 때 쓰는 경로. 설치된 파일이 있으면 그걸, 없으면 스크립트가 받을 자리를 가리킨다
pub fn default_paths(models_dir: &Path, home: &Path) -> ModelPaths {
    let catalog = catalog(models_dir, home);
    let turbo = &WHISPER_MODELS[0];
    ModelPaths {
        whisper: catalog
            .whisper
            .iter()
            .find_map(|w| w.path.clone())
            .unwrap_or_else(|| models_dir.join(turbo.file)),
        segmentation: catalog.segmentation.unwrap_or_else(|| models_dir.join(SEGMENTATION_FILE)),
        embedding: catalog.embedding.unwrap_or_else(|| models_dir.join(EMBEDDING_FILE)),
    }
}

/// 처리 전에 모델 파일이 있는지 본다. 없으면 설치 방법까지 담은 오류 문구를 돌려준다
pub fn check(paths: &ModelPaths) -> Result<(), String> {
    let missing: Vec<String> = [
        ("Whisper 모델", &paths.whisper),
        ("화자 구간 모델", &paths.segmentation),
        ("화자 임베딩 모델", &paths.embedding),
    ]
    .iter()
    .filter(|(_, p)| !p.is_file())
    .map(|(name, p)| format!("{name}: {}", p.display()))
    .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let script = if cfg!(windows) {
        "scripts\\install-models.ps1"
    } else {
        "scripts/install-models.sh"
    };
    Err(format!(
        "모델 파일이 없습니다.\n{}\n\n{script}로 설치한 뒤 설정에서 경로를 확인해 주세요",
        missing.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sori-models-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 앱_모델_폴더를_예전_위치보다_먼저_찾는다() {
        let root = temp("order");
        let (models, home) = (root.join("models"), root.join("home"));
        std::fs::create_dir_all(home.join("whisper-models")).unwrap();
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(home.join("whisper-models/ggml-large-v3-turbo.bin"), b"x").unwrap();
        std::fs::write(models.join("ggml-large-v3-turbo.bin"), b"x").unwrap();

        let c = catalog(&models, &home);

        assert_eq!(c.whisper[0].path.as_deref(), Some(models.join("ggml-large-v3-turbo.bin").as_path()));
        assert_eq!(c.whisper[1].path, None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 아무것도_없으면_스크립트가_받을_자리를_기본값으로_쓴다() {
        let root = temp("empty");
        let models = root.join("models");

        let p = default_paths(&models, &root.join("home"));

        assert_eq!(p.whisper, models.join("ggml-large-v3-turbo.bin"));
        assert!(check(&p).unwrap_err().contains("install-models"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
