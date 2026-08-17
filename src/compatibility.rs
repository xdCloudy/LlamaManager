use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    gguf::ModelInfo,
    llama::{LlamaInstallation, now_ms},
    multimodal::{
        ProjectorInfo, ProjectorMatchStatus, ProjectorRequirement, evaluate_projector_pair,
        is_projector_gguf, projector_requirement,
    },
};

pub const ARCHITECTURE_REGISTRY_REVISION: &str = "llama.cpp-arch-2026-08-17";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    Compatible,
    Limited,
    Incompatible,
    Unknown,
}

impl CompatibilityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compatible => "compatible",
            Self::Limited => "limited",
            Self::Incompatible => "incompatible",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityReason {
    pub code: String,
    pub message: String,
    pub evidence: Vec<String>,
}

impl CompatibilityReason {
    fn new(code: &str, message: impl Into<String>, evidence: Vec<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            evidence,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityResult {
    pub model_id: String,
    pub installation_id: String,
    pub model_sha256: String,
    pub installation_fingerprint: String,
    pub registry_revision: String,
    pub status: CompatibilityStatus,
    pub reasons: Vec<CompatibilityReason>,
    pub computed_at_unix_ms: u128,
}

impl CompatibilityResult {
    pub fn is_stale(&self, model: &ModelInfo, installation: &LlamaInstallation) -> bool {
        self.model_sha256 != model.sha256
            || self.installation_fingerprint != installation_fingerprint(installation)
            || self.registry_revision != ARCHITECTURE_REGISTRY_REVISION
    }
}

pub fn installation_fingerprint(installation: &LlamaInstallation) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"llamamanager:compatibility-installation:v1\n");
    hasher.update(installation.id.as_bytes());
    hasher.update(b"\n");

    for tool in [
        installation.server.as_ref(),
        installation.bench.as_ref(),
        installation.fit_params.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        hasher.update(tool.sha256.as_bytes());
        hasher.update(b"\n");
    }

    for capability in &installation.capabilities {
        hasher.update(capability.as_bytes());
        hasher.update(b"\n");
    }

    hex::encode(hasher.finalize())
}

pub fn evaluate_compatibility(
    model: &ModelInfo,
    installation: &LlamaInstallation,
    projector: Option<&ProjectorInfo>,
) -> CompatibilityResult {
    let fingerprint = installation_fingerprint(installation);
    let mut reasons = Vec::new();
    let mut status = CompatibilityStatus::Compatible;

    if is_projector_gguf(model) {
        status = CompatibilityStatus::Incompatible;
        reasons.push(CompatibilityReason::new(
            "model_is_projector",
            "the selected GGUF is projector/CLIP data rather than a loadable text model",
            vec![format!("model={}", model.path.display())],
        ));
        return result(model, installation, fingerprint, status, reasons);
    }

    let Some(server) = installation.server.as_ref() else {
        status = CompatibilityStatus::Incompatible;
        reasons.push(CompatibilityReason::new(
            "server_missing",
            "the selected llama.cpp installation does not contain llama-server",
            vec![format!("installation={}", installation.root_path.display())],
        ));
        return result(model, installation, fingerprint, status, reasons);
    };

    if !help_has_option(&server.help_output, "--model")
        && !help_has_option(&server.help_output, "-m")
    {
        status = CompatibilityStatus::Incompatible;
        reasons.push(CompatibilityReason::new(
            "model_option_missing",
            "llama-server capability evidence does not expose a model-file option",
            vec![format!("server_sha256={}", server.sha256)],
        ));
        return result(model, installation, fingerprint, status, reasons);
    }

    match model.architecture.as_deref() {
        None => {
            status = CompatibilityStatus::Unknown;
            reasons.push(CompatibilityReason::new(
                "architecture_missing",
                "GGUF does not declare general.architecture, so support cannot be inferred safely",
                vec![format!("model_sha256={}", model.sha256)],
            ));
        }
        Some(architecture) if !known_upstream_architecture(architecture) => {
            status = CompatibilityStatus::Unknown;
            reasons.push(CompatibilityReason::new(
                "architecture_unknown",
                format!(
                    "architecture {architecture} is not in the application's current llama.cpp architecture registry"
                ),
                vec![
                    format!("general.architecture={architecture}"),
                    format!("registry={ARCHITECTURE_REGISTRY_REVISION}"),
                ],
            ));
        }
        Some(architecture) => {
            reasons.push(CompatibilityReason::new(
                "architecture_known",
                format!("architecture {architecture} is recognized by the current registry"),
                vec![
                    format!("general.architecture={architecture}"),
                    format!("registry={ARCHITECTURE_REGISTRY_REVISION}"),
                ],
            ));
        }
    }

    if matches!(status, CompatibilityStatus::Unknown | CompatibilityStatus::Incompatible) {
        return result(model, installation, fingerprint, status, reasons);
    }

    if model.file_type.is_none() || model.quantization_version.is_none() {
        status = CompatibilityStatus::Limited;
        reasons.push(CompatibilityReason::new(
            "quantization_metadata_partial",
            "quantization/file-type metadata is incomplete; loading may still work but the static decision is limited",
            vec![
                format!("general.file_type={:?}", model.file_type),
                format!(
                    "general.quantization_version={:?}",
                    model.quantization_version
                ),
            ],
        ));
    }

    if model.tensor_count > 0 && model.tensor_type_counts.is_empty() {
        status = CompatibilityStatus::Limited;
        reasons.push(CompatibilityReason::new(
            "tensor_summary_stale",
            "tensor summary is missing for a model with declared tensors; re-inspection is required for full evidence",
            vec![format!("tensor_count={}", model.tensor_count)],
        ));
    }

    let requirement = projector_requirement(model);
    match requirement.requirement {
        ProjectorRequirement::Required => {
            let Some(projector) = projector else {
                status = CompatibilityStatus::Limited;
                reasons.push(CompatibilityReason::new(
                    "projector_required_missing",
                    "the model requires an external multimodal projector but none is associated",
                    requirement.reasons,
                ));
                return result(model, installation, fingerprint, status, reasons);
            };

            if !help_has_option(&server.help_output, "--mmproj")
                && !help_has_option(&server.help_output, "-mm")
            {
                status = CompatibilityStatus::Incompatible;
                reasons.push(CompatibilityReason::new(
                    "mmproj_capability_missing",
                    "the model requires a projector, but selected llama-server does not expose --mmproj capability",
                    vec![format!("server_sha256={}", server.sha256)],
                ));
                return result(model, installation, fingerprint, status, reasons);
            }

            let pairing = evaluate_projector_pair(model, projector);
            match pairing.status {
                ProjectorMatchStatus::Compatible => reasons.push(CompatibilityReason::new(
                    "projector_pair_compatible",
                    "associated projector supplies the required modality evidence",
                    pairing.reasons,
                )),
                ProjectorMatchStatus::Incompatible => {
                    status = CompatibilityStatus::Incompatible;
                    reasons.push(CompatibilityReason::new(
                        "projector_pair_incompatible",
                        "associated projector is incompatible with the model's required modalities",
                        pairing.reasons,
                    ));
                }
                ProjectorMatchStatus::Unknown => {
                    status = CompatibilityStatus::Limited;
                    reasons.push(CompatibilityReason::new(
                        "projector_pair_unknown",
                        "projector pairing cannot be proven from available metadata",
                        pairing.reasons,
                    ));
                }
            }
        }
        ProjectorRequirement::Optional => {
            status = CompatibilityStatus::Limited;
            reasons.push(CompatibilityReason::new(
                "projector_optional",
                "this architecture family has variants with different projector requirements",
                requirement.reasons,
            ));
            if let Some(projector) = projector {
                let pairing = evaluate_projector_pair(model, projector);
                if pairing.status == ProjectorMatchStatus::Incompatible {
                    status = CompatibilityStatus::Incompatible;
                }
                reasons.push(CompatibilityReason::new(
                    "optional_projector_pair",
                    format!("optional projector pairing is {:?}", pairing.status),
                    pairing.reasons,
                ));
            }
        }
        ProjectorRequirement::Unknown => {
            status = CompatibilityStatus::Limited;
            reasons.push(CompatibilityReason::new(
                "projector_requirement_unknown",
                "projector requirements are not known for this model from current evidence",
                requirement.reasons,
            ));
        }
        ProjectorRequirement::NotRequired => reasons.push(CompatibilityReason::new(
            "projector_not_required",
            "no external projector requirement is known for this architecture",
            requirement.reasons,
        )),
    }

    result(model, installation, fingerprint, status, reasons)
}

fn result(
    model: &ModelInfo,
    installation: &LlamaInstallation,
    installation_fingerprint: String,
    status: CompatibilityStatus,
    reasons: Vec<CompatibilityReason>,
) -> CompatibilityResult {
    CompatibilityResult {
        model_id: model.id.clone(),
        installation_id: installation.id.clone(),
        model_sha256: model.sha256.clone(),
        installation_fingerprint,
        registry_revision: ARCHITECTURE_REGISTRY_REVISION.into(),
        status,
        reasons,
        computed_at_unix_ms: now_ms(),
    }
}

fn help_has_option(help: &str, expected: &str) -> bool {
    help.split_whitespace().any(|token| {
        token
            .trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '[' | ']' | '(' | ')' | '`'))
            == expected
    })
}

fn known_upstream_architecture(architecture: &str) -> bool {
    matches!(
        architecture,
        "llama"
            | "llama4"
            | "deci"
            | "falcon"
            | "falcon-h1"
            | "baichuan"
            | "grok"
            | "gpt2"
            | "gptj"
            | "gptneox"
            | "mpt"
            | "starcoder"
            | "starcoder2"
            | "refact"
            | "bert"
            | "modern-bert"
            | "nomic-bert"
            | "nomic-bert-moe"
            | "neo-bert"
            | "jina-bert-v2"
            | "jina-bert-v3"
            | "eurobert"
            | "bloom"
            | "stablelm"
            | "qwen"
            | "qwen2"
            | "qwen2moe"
            | "qwen2vl"
            | "qwen3"
            | "qwen3moe"
            | "qwen3next"
            | "qwen3vl"
            | "qwen3vlmoe"
            | "qwen35"
            | "qwen35moe"
            | "phi2"
            | "phi3"
            | "phimoe"
            | "plamo"
            | "plamo2"
            | "plamo3"
            | "codeshell"
            | "orion"
            | "internlm2"
            | "minicpm"
            | "minicpm3"
            | "gemma"
            | "gemma2"
            | "gemma3"
            | "gemma3n"
            | "gemma4"
            | "gemma4-assistant"
            | "gemma-embedding"
            | "rwkv6"
            | "rwkv6qwen2"
            | "rwkv7"
            | "arwkv7"
            | "mamba"
            | "mamba2"
            | "jamba"
            | "xverse"
            | "command-r"
            | "cohere2"
            | "cohere2moe"
            | "dbrx"
            | "olmo"
            | "olmo2"
            | "olmoe"
            | "openelm"
            | "arctic"
            | "deepseek"
            | "deepseek2"
            | "deepseek2-ocr"
            | "deepseek32"
            | "deepseek4"
            | "chatglm"
            | "glm4"
            | "glm4moe"
            | "glm-dsa"
            | "bitnet"
            | "t5"
            | "t5encoder"
            | "jais"
            | "jais2"
            | "nemotron"
            | "exaone"
            | "exaone4"
            | "granite"
            | "granitemoe"
            | "granitehybrid"
            | "chameleon"
            | "mistral3"
            | "mistral4"
            | "gpt-oss"
            | "lfm2"
            | "smollm3"
            | "eagle3"
            | "dflash"
            | "kimi-linear"
            | "step35"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{gguf::MetadataValue, llama::ToolEvidence};
    use std::{collections::{BTreeMap, BTreeSet}, path::PathBuf};

    fn model(architecture: Option<&str>) -> ModelInfo {
        ModelInfo {
            id: "model-test".into(),
            path: PathBuf::from(r"C:\Models\Model.gguf"),
            file_size: 1024,
            sha256: "a".repeat(64),
            gguf_version: 3,
            tensor_count: 1,
            metadata_count: 4,
            name: Some("Test".into()),
            architecture: architecture.map(str::to_owned),
            context_length: Some(4096),
            quantization_version: Some(2),
            general_type: Some("model".into()),
            file_type: Some(15),
            parameter_count: Some(16),
            tensor_type_counts: BTreeMap::from([(12, 1)]),
            metadata: BTreeMap::from([(
                "general.type".into(),
                MetadataValue::String("model".into()),
            )]),
            inspected_at_unix_ms: 1,
        }
    }

    fn installation(help: &str) -> LlamaInstallation {
        let tool = ToolEvidence {
            path: PathBuf::from(r"C:\llama cpp\llama-server.exe"),
            sha256: "b".repeat(64),
            version_output: "version-test".into(),
            help_output: help.into(),
            device_output: String::new(),
        };
        LlamaInstallation {
            id: "installation-test".into(),
            name: "test".into(),
            root_path: PathBuf::from(r"C:\llama cpp"),
            server: Some(tool),
            bench: None,
            fit_params: None,
            backend: Some("CPU".into()),
            capabilities: BTreeSet::from(["--model".into()]),
            discovered_at_unix_ms: 1,
        }
    }

    #[test]
    fn positive_text_model_is_compatible() {
        let result = evaluate_compatibility(&model(Some("qwen35")), &installation("--model FILE"), None);
        assert_eq!(result.status, CompatibilityStatus::Compatible);
    }

    #[test]
    fn unknown_architecture_is_not_silently_accepted() {
        let result = evaluate_compatibility(
            &model(Some("future-architecture")),
            &installation("--model FILE"),
            None,
        );
        assert_eq!(result.status, CompatibilityStatus::Unknown);
        assert!(result.reasons.iter().any(|reason| reason.code == "architecture_unknown"));
    }

    #[test]
    fn required_projector_without_mmproj_capability_is_limited_when_missing() {
        let result = evaluate_compatibility(&model(Some("qwen3vl")), &installation("--model FILE"), None);
        assert_eq!(result.status, CompatibilityStatus::Limited);
        assert!(result
            .reasons
            .iter()
            .any(|reason| reason.code == "projector_required_missing"));
    }

    #[test]
    fn installation_hash_change_marks_persisted_result_stale() {
        let model = model(Some("qwen35"));
        let first = installation("--model FILE");
        let result = evaluate_compatibility(&model, &first, None);
        let mut changed = first.clone();
        changed.server.as_mut().unwrap().sha256 = "c".repeat(64);
        assert!(result.is_stale(&model, &changed));
    }
}
