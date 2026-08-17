use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::gguf::ModelInfo;

pub const MULTIMODAL_REGISTRY_REVISION: &str = "llama.cpp-mtmd-2026-08-17";

const PROJECTOR_TYPE_KEYS: &[&str] = &["clip.vision.projector_type", "clip.audio.projector_type"];

const PROJECTOR_BOOLEAN_KEYS: &[&str] = &[
    "clip.has_vision_encoder",
    "clip.has_audio_encoder",
    "clip.has_llava_projector",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Vision,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectorRequirement {
    NotRequired,
    Optional,
    Required,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectorRequirementEvidence {
    pub requirement: ProjectorRequirement,
    pub modalities: BTreeSet<Modality>,
    pub reasons: Vec<String>,
    pub registry_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectorInfo {
    pub id: String,
    pub path: PathBuf,
    pub file_size: u64,
    pub sha256: String,
    pub name: Option<String>,
    pub general_type: Option<String>,
    pub architecture: Option<String>,
    pub projector_type: Option<String>,
    pub modalities: BTreeSet<Modality>,
    pub source_model_hint: Option<String>,
    pub inspected_at_unix_ms: u128,
}

impl ProjectorInfo {
    pub fn from_gguf(info: &ModelInfo) -> Option<Self> {
        if !is_projector_gguf(info) {
            return None;
        }

        let mut modalities = BTreeSet::new();
        if info.metadata_bool("clip.has_vision_encoder") == Some(true)
            || info.has_metadata_key("clip.vision.projector_type")
        {
            modalities.insert(Modality::Vision);
        }
        if info.metadata_bool("clip.has_audio_encoder") == Some(true)
            || info.has_metadata_key("clip.audio.projector_type")
        {
            modalities.insert(Modality::Audio);
        }

        let projector_type = PROJECTOR_TYPE_KEYS
            .iter()
            .find_map(|key| info.metadata_string(key).map(str::to_owned));
        let source_model_hint = info
            .metadata_string("general.basename")
            .or_else(|| info.metadata_string("general.name"))
            .map(str::to_owned);

        Some(Self {
            id: format!("projector-{}", &info.sha256[..32]),
            path: info.path.clone(),
            file_size: info.file_size,
            sha256: info.sha256.clone(),
            name: info.name.clone(),
            general_type: info.general_type.clone(),
            architecture: info.architecture.clone(),
            projector_type,
            modalities,
            source_model_hint,
            inspected_at_unix_ms: info.inspected_at_unix_ms,
        })
    }
}

pub fn is_projector_gguf(info: &ModelInfo) -> bool {
    info.general_type.as_deref() == Some("mmproj")
        || info.architecture.as_deref() == Some("clip")
        || PROJECTOR_TYPE_KEYS
            .iter()
            .any(|key| info.has_metadata_key(key))
        || PROJECTOR_BOOLEAN_KEYS
            .iter()
            .any(|key| info.metadata_bool(key) == Some(true))
}

pub fn projector_requirement(model: &ModelInfo) -> ProjectorRequirementEvidence {
    let architecture = model.architecture.as_deref();
    let mut modalities = BTreeSet::new();
    let mut reasons = Vec::new();

    if is_projector_gguf(model) {
        reasons
            .push("the selected GGUF is itself projector/CLIP evidence, not a text model".into());
        return ProjectorRequirementEvidence {
            requirement: ProjectorRequirement::Unknown,
            modalities,
            reasons,
            registry_revision: MULTIMODAL_REGISTRY_REVISION.into(),
        };
    }

    match architecture {
        Some("qwen2vl" | "qwen3vl" | "qwen3vlmoe" | "mistral3") => {
            modalities.insert(Modality::Vision);
            reasons.push(format!(
                "architecture {} is a known multimodal family with external projector support",
                architecture.unwrap_or_default()
            ));
            ProjectorRequirementEvidence {
                requirement: ProjectorRequirement::Required,
                modalities,
                reasons,
                registry_revision: MULTIMODAL_REGISTRY_REVISION.into(),
            }
        }
        Some("gemma3") => {
            modalities.insert(Modality::Vision);
            reasons.push(
                "Gemma 3 includes both vision-capable and text-only variants; projector need must be confirmed from model/runtime evidence"
                    .into(),
            );
            ProjectorRequirementEvidence {
                requirement: ProjectorRequirement::Optional,
                modalities,
                reasons,
                registry_revision: MULTIMODAL_REGISTRY_REVISION.into(),
            }
        }
        Some(
            "llama" | "llama4" | "qwen" | "qwen2" | "qwen2moe" | "qwen3" | "qwen3moe" | "qwen3next"
            | "qwen35" | "qwen35moe" | "phi2" | "phi3" | "phimoe" | "gemma" | "gemma2" | "deepseek"
            | "deepseek2" | "deepseek4" | "mamba" | "mamba2" | "jamba" | "gpt2" | "gptj"
            | "gptneox" | "starcoder" | "starcoder2" | "command-r" | "cohere2" | "cohere2moe"
            | "olmo" | "olmo2" | "olmoe" | "bitnet" | "t5" | "t5encoder",
        ) => {
            reasons.push(
                "no external projector requirement is known for this architecture entry".into(),
            );
            ProjectorRequirementEvidence {
                requirement: ProjectorRequirement::NotRequired,
                modalities,
                reasons,
                registry_revision: MULTIMODAL_REGISTRY_REVISION.into(),
            }
        }
        Some(value) => {
            reasons.push(format!(
                "projector requirement for architecture {value} is not in the current registry"
            ));
            ProjectorRequirementEvidence {
                requirement: ProjectorRequirement::Unknown,
                modalities,
                reasons,
                registry_revision: MULTIMODAL_REGISTRY_REVISION.into(),
            }
        }
        None => {
            reasons.push("GGUF does not declare general.architecture".into());
            ProjectorRequirementEvidence {
                requirement: ProjectorRequirement::Unknown,
                modalities,
                reasons,
                registry_revision: MULTIMODAL_REGISTRY_REVISION.into(),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectorMatchStatus {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectorMatch {
    pub status: ProjectorMatchStatus,
    pub reasons: Vec<String>,
}

pub fn evaluate_projector_pair(model: &ModelInfo, projector: &ProjectorInfo) -> ProjectorMatch {
    let requirement = projector_requirement(model);
    let mut reasons = requirement.reasons.clone();

    if projector.id.is_empty() || projector.sha256.is_empty() {
        reasons.push("projector identity evidence is incomplete".into());
        return ProjectorMatch {
            status: ProjectorMatchStatus::Incompatible,
            reasons,
        };
    }

    if matches!(requirement.requirement, ProjectorRequirement::NotRequired) {
        reasons.push("the model does not currently require an external projector".into());
        return ProjectorMatch {
            status: ProjectorMatchStatus::Unknown,
            reasons,
        };
    }

    if requirement.modalities.is_empty() {
        reasons.push("required modality is unknown, so the pairing cannot be proven".into());
        return ProjectorMatch {
            status: ProjectorMatchStatus::Unknown,
            reasons,
        };
    }

    if projector.modalities.is_empty() {
        reasons
            .push("projector GGUF does not expose a recognized vision/audio encoder marker".into());
        return ProjectorMatch {
            status: ProjectorMatchStatus::Unknown,
            reasons,
        };
    }

    let missing: Vec<_> = requirement
        .modalities
        .difference(&projector.modalities)
        .copied()
        .collect();
    if !missing.is_empty() {
        reasons.push(format!(
            "projector does not provide required modalities: {:?}",
            missing
        ));
        return ProjectorMatch {
            status: ProjectorMatchStatus::Incompatible,
            reasons,
        };
    }

    reasons.push(
        "projector metadata supplies every modality required by the model registry entry".into(),
    );
    ProjectorMatch {
        status: ProjectorMatchStatus::Compatible,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn model(architecture: Option<&str>) -> ModelInfo {
        ModelInfo {
            id: "model-test".into(),
            path: PathBuf::from(r"C:\Models\模型.gguf"),
            file_size: 1,
            sha256: "a".repeat(64),
            gguf_version: 3,
            tensor_count: 0,
            metadata_count: 0,
            name: None,
            architecture: architecture.map(str::to_owned),
            context_length: None,
            quantization_version: None,
            general_type: Some("model".into()),
            file_type: None,
            parameter_count: None,
            tensor_type_counts: BTreeMap::new(),
            metadata: BTreeMap::new(),
            inspected_at_unix_ms: 1,
        }
    }

    #[test]
    fn classifies_required_optional_and_unknown_projector_cases() {
        assert_eq!(
            projector_requirement(&model(Some("qwen3vl"))).requirement,
            ProjectorRequirement::Required
        );
        assert_eq!(
            projector_requirement(&model(Some("gemma3"))).requirement,
            ProjectorRequirement::Optional
        );
        assert_eq!(
            projector_requirement(&model(Some("future-vl"))).requirement,
            ProjectorRequirement::Unknown
        );
    }

    #[test]
    fn rejects_projector_with_wrong_explicit_modality() {
        let model = model(Some("qwen3vl"));
        let projector = ProjectorInfo {
            id: "projector-test".into(),
            path: PathBuf::from(r"C:\Models\mmproj 模型.gguf"),
            file_size: 1,
            sha256: "b".repeat(64),
            name: None,
            general_type: Some("mmproj".into()),
            architecture: Some("clip".into()),
            projector_type: Some("audio".into()),
            modalities: BTreeSet::from([Modality::Audio]),
            source_model_hint: None,
            inspected_at_unix_ms: 1,
        };
        assert_eq!(
            evaluate_projector_pair(&model, &projector).status,
            ProjectorMatchStatus::Incompatible
        );
    }
}
