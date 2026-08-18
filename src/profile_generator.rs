use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use thiserror::Error;

use crate::{
    compatibility::{CompatibilityResult, CompatibilityStatus},
    config_write::{
        ConfigWriteReceipt, DEFAULT_BACKUP_RETENTION, write_external_models_ini,
        write_managed_models_ini,
    },
    gguf::ModelInfo,
    llama::LlamaInstallation,
    models_ini::ModelsIniDocument,
    models_ini_validation::{ConfigDiff, ValidationReport, diff_configs, validate_semantics},
    multimodal::{ProjectorInfo, ProjectorRequirement, projector_requirement},
    paths::AppPaths,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileDestination {
    Managed,
    External(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendationBasis {
    Measured,
    Derived,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecommendedSetting {
    pub key: String,
    pub value: String,
    pub capability_aliases: Vec<String>,
    pub basis: RecommendationBasis,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredSetting {
    pub key: String,
    pub value: String,
    pub capability_aliases: Vec<String>,
    pub evidence: Vec<String>,
    pub explicitly_disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileProvenanceSource {
    ModelEvidence,
    ProjectorEvidence,
    MeasuredHardware,
    DerivedHardware,
    RequiredCapability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileProvenance {
    pub source: ProfileProvenanceSource,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedProfileChoice {
    pub key: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ProfileGenerationRequest<'a> {
    pub section: &'a str,
    pub baseline_source: &'a str,
    pub installation: &'a LlamaInstallation,
    pub model: &'a ModelInfo,
    pub compatibility: &'a CompatibilityResult,
    pub projector: Option<&'a ProjectorInfo>,
    pub recommendations: &'a [RecommendedSetting],
    pub required_settings: &'a [RequiredSetting],
    pub explicitly_disabled_features: &'a BTreeSet<String>,
    pub destination: ProfileDestination,
}

#[derive(Debug, Clone)]
pub struct GeneratedProfile {
    pub section: String,
    pub source: String,
    pub destination: ProfileDestination,
    pub provenance: BTreeMap<String, ProfileProvenance>,
    pub unresolved: Vec<UnresolvedProfileChoice>,
    pub validation: ValidationReport,
    pub diff: ConfigDiff,
}

impl GeneratedProfile {
    pub fn can_apply(&self) -> bool {
        self.validation.can_apply()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProfileGenerationError {
    #[error("profile section name cannot be empty")]
    EmptySection,

    #[error("profile section [{0}] already exists; generator will not overwrite an existing section")]
    SectionAlreadyExists(String),

    #[error("baseline models.ini is invalid: {0}")]
    InvalidBaseline(String),

    #[error("compatibility evidence belongs to a different model or installation")]
    CompatibilityIdentityMismatch,

    #[error("compatibility evidence is stale for the selected model/runtime")]
    StaleCompatibility,

    #[error("selected model/runtime pairing is {status}; a safe starting profile cannot be generated")]
    UnsafeCompatibility { status: String },

    #[error("selected llama.cpp installation does not contain llama-server")]
    ServerMissing,

    #[error("required runtime option {option} is not supported by selected llama-server evidence")]
    RequiredCapabilityMissing { option: String },

    #[error("required multimodal projector is missing and multimodal was not explicitly disabled")]
    RequiredProjectorMissing,

    #[error("required projector is present but selected llama-server does not expose --mmproj")]
    ProjectorCapabilityMissing,

    #[error("recommendation {key} has no measured/derived evidence")]
    RecommendationEvidenceMissing { key: String },

    #[error("required setting {key} has no evidence")]
    RequiredSettingEvidenceMissing { key: String },

    #[error("generated profile could not be parsed: {0}")]
    GeneratedParseFailure(String),
}

pub fn generate_profile(
    request: &ProfileGenerationRequest<'_>,
) -> Result<GeneratedProfile, ProfileGenerationError> {
    let section = request.section.trim();
    if section.is_empty() {
        return Err(ProfileGenerationError::EmptySection);
    }

    let baseline = ModelsIniDocument::parse(request.baseline_source)
        .map_err(|error| ProfileGenerationError::InvalidBaseline(error.to_string()))?;
    if baseline.section_names().contains(&section) {
        return Err(ProfileGenerationError::SectionAlreadyExists(section.into()));
    }

    validate_compatibility_identity(request)?;
    let server = request
        .installation
        .server
        .as_ref()
        .ok_or(ProfileGenerationError::ServerMissing)?;
    let server_options = extract_options(&server.help_output);

    let mut emitted = BTreeMap::<String, String>::new();
    let mut provenance = BTreeMap::<String, ProfileProvenance>::new();
    let mut unresolved = Vec::new();

    require_supported(&server_options, &["--model", "-m"], "--model")?;
    insert_setting(
        &mut emitted,
        &mut provenance,
        "model",
        request.model.path.to_string_lossy().into_owned(),
        ProfileProvenance {
            source: ProfileProvenanceSource::ModelEvidence,
            evidence: vec![
                format!("model_id={}", request.model.id),
                format!("model_sha256={}", request.model.sha256),
                format!("model_path={}", request.model.path.display()),
            ],
        },
    );

    add_projector_setting(
        request,
        &server_options,
        &mut emitted,
        &mut provenance,
        &mut unresolved,
    )?;

    for required in request.required_settings {
        if required.evidence.is_empty() {
            return Err(ProfileGenerationError::RequiredSettingEvidenceMissing {
                key: required.key.clone(),
            });
        }
        if required.explicitly_disabled {
            unresolved.push(UnresolvedProfileChoice {
                key: required.key.clone(),
                reason: "required feature was explicitly disabled by the user; setting was not emitted"
                    .into(),
            });
            continue;
        }
        if !has_any_option(&server_options, &required.capability_aliases) {
            return Err(ProfileGenerationError::RequiredCapabilityMissing {
                option: display_required_option(&required.capability_aliases, &required.key),
            });
        }
        insert_setting(
            &mut emitted,
            &mut provenance,
            &required.key,
            required.value.clone(),
            ProfileProvenance {
                source: ProfileProvenanceSource::RequiredCapability,
                evidence: required.evidence.clone(),
            },
        );
    }

    for recommendation in request.recommendations {
        if recommendation.evidence.is_empty() {
            return Err(ProfileGenerationError::RecommendationEvidenceMissing {
                key: recommendation.key.clone(),
            });
        }
        if !has_any_option(&server_options, &recommendation.capability_aliases) {
            unresolved.push(UnresolvedProfileChoice {
                key: recommendation.key.clone(),
                reason: format!(
                    "measured/derived recommendation exists, but selected llama-server does not expose any of: {}",
                    recommendation.capability_aliases.join(", ")
                ),
            });
            continue;
        }

        let source = match recommendation.basis {
            RecommendationBasis::Measured => ProfileProvenanceSource::MeasuredHardware,
            RecommendationBasis::Derived => ProfileProvenanceSource::DerivedHardware,
        };
        insert_setting(
            &mut emitted,
            &mut provenance,
            &recommendation.key,
            recommendation.value.clone(),
            ProfileProvenance {
                source,
                evidence: recommendation.evidence.clone(),
            },
        );
    }

    let source = append_generated_section(&baseline, section, &emitted);
    let generated = ModelsIniDocument::parse(&source)
        .map_err(|error| ProfileGenerationError::GeneratedParseFailure(error.to_string()))?;
    let validation = validate_semantics(&generated, section, Some(request.installation));
    let diff = diff_configs(&baseline, &generated, section);

    Ok(GeneratedProfile {
        section: section.into(),
        source,
        destination: request.destination.clone(),
        provenance,
        unresolved,
        validation,
        diff,
    })
}

pub fn apply_generated_profile(
    paths: &AppPaths,
    profile: &GeneratedProfile,
) -> Result<ConfigWriteReceipt, crate::config_write::ConfigWriteError> {
    match &profile.destination {
        ProfileDestination::Managed => {
            write_managed_models_ini(paths, &profile.source, &profile.validation)
        }
        ProfileDestination::External(path) => write_external_models_ini(
            path,
            &profile.source,
            &profile.validation,
            DEFAULT_BACKUP_RETENTION,
        ),
    }
}

fn validate_compatibility_identity(
    request: &ProfileGenerationRequest<'_>,
) -> Result<(), ProfileGenerationError> {
    if request.compatibility.model_id != request.model.id
        || request.compatibility.installation_id != request.installation.id
        || request.compatibility.model_sha256 != request.model.sha256
    {
        return Err(ProfileGenerationError::CompatibilityIdentityMismatch);
    }
    if request
        .compatibility
        .is_stale(request.model, request.installation)
    {
        return Err(ProfileGenerationError::StaleCompatibility);
    }
    match request.compatibility.status {
        CompatibilityStatus::Compatible | CompatibilityStatus::Limited => Ok(()),
        CompatibilityStatus::Incompatible | CompatibilityStatus::Unknown => {
            Err(ProfileGenerationError::UnsafeCompatibility {
                status: request.compatibility.status.as_str().into(),
            })
        }
    }
}

fn add_projector_setting(
    request: &ProfileGenerationRequest<'_>,
    server_options: &BTreeSet<String>,
    emitted: &mut BTreeMap<String, String>,
    provenance: &mut BTreeMap<String, ProfileProvenance>,
    unresolved: &mut Vec<UnresolvedProfileChoice>,
) -> Result<(), ProfileGenerationError> {
    let requirement = projector_requirement(request.model);
    let multimodal_disabled = request
        .explicitly_disabled_features
        .contains("multimodal");

    match requirement.requirement {
        ProjectorRequirement::Required if multimodal_disabled => {
            unresolved.push(UnresolvedProfileChoice {
                key: "mmproj".into(),
                reason: "model metadata requires multimodal projector support, but multimodal was explicitly disabled"
                    .into(),
            });
            return Ok(());
        }
        ProjectorRequirement::Required if request.projector.is_none() => {
            return Err(ProfileGenerationError::RequiredProjectorMissing);
        }
        ProjectorRequirement::Optional if request.projector.is_none() => {
            unresolved.push(UnresolvedProfileChoice {
                key: "mmproj".into(),
                reason: "projector requirement is optional/variant-dependent; no projector was explicitly associated"
                    .into(),
            });
            return Ok(());
        }
        ProjectorRequirement::Unknown if request.projector.is_none() => {
            unresolved.push(UnresolvedProfileChoice {
                key: "mmproj".into(),
                reason: "projector requirement is unknown from current metadata; user/runtime evidence is required"
                    .into(),
            });
            return Ok(());
        }
        ProjectorRequirement::NotRequired if request.projector.is_none() => return Ok(()),
        _ => {}
    }

    let Some(projector) = request.projector else {
        return Ok(());
    };
    if multimodal_disabled {
        unresolved.push(UnresolvedProfileChoice {
            key: "mmproj".into(),
            reason: "an associated projector exists, but multimodal was explicitly disabled".into(),
        });
        return Ok(());
    }
    if !has_any_option(server_options, &["--mmproj".into(), "-mm".into()]) {
        if requirement.requirement == ProjectorRequirement::Required {
            return Err(ProfileGenerationError::ProjectorCapabilityMissing);
        }
        unresolved.push(UnresolvedProfileChoice {
            key: "mmproj".into(),
            reason: "projector is associated, but selected llama-server does not expose --mmproj"
                .into(),
        });
        return Ok(());
    }

    insert_setting(
        emitted,
        provenance,
        "mmproj",
        projector.path.to_string_lossy().into_owned(),
        ProfileProvenance {
            source: ProfileProvenanceSource::ProjectorEvidence,
            evidence: vec![
                format!("projector_id={}", projector.id),
                format!("projector_sha256={}", projector.sha256),
                format!("projector_path={}", projector.path.display()),
                format!("requirement={:?}", requirement.requirement),
            ],
        },
    );
    Ok(())
}

fn insert_setting(
    emitted: &mut BTreeMap<String, String>,
    provenance: &mut BTreeMap<String, ProfileProvenance>,
    key: &str,
    value: String,
    source: ProfileProvenance,
) {
    emitted.insert(key.into(), value);
    provenance.insert(key.into(), source);
}

fn require_supported(
    options: &BTreeSet<String>,
    aliases: &[&str],
    display: &str,
) -> Result<(), ProfileGenerationError> {
    if aliases.iter().any(|alias| options.contains(*alias)) {
        Ok(())
    } else {
        Err(ProfileGenerationError::RequiredCapabilityMissing {
            option: display.into(),
        })
    }
}

fn has_any_option(options: &BTreeSet<String>, aliases: &[String]) -> bool {
    aliases.iter().any(|alias| options.contains(alias))
}

fn display_required_option(aliases: &[String], key: &str) -> String {
    aliases
        .first()
        .cloned()
        .unwrap_or_else(|| format!("--{key}"))
}

fn extract_options(help: &str) -> BTreeSet<String> {
    help.split_whitespace()
        .filter_map(|token| {
            let option = token.trim_matches(|character: char| {
                matches!(
                    character,
                    ',' | ';' | ':' | '[' | ']' | '(' | ')' | '`' | '<' | '>'
                )
            });
            option.starts_with('-').then(|| option.to_owned())
        })
        .collect()
}

fn append_generated_section(
    baseline: &ModelsIniDocument,
    section: &str,
    emitted: &BTreeMap<String, String>,
) -> String {
    let ending = baseline
        .lines()
        .iter()
        .find_map(|line| match line.ending {
            crate::models_ini::LineEnding::CrLf => Some("\r\n"),
            crate::models_ini::LineEnding::Lf => Some("\n"),
            crate::models_ini::LineEnding::None => None,
        })
        .unwrap_or("\n");

    let mut source = baseline.serialize();
    if !source.is_empty() && !source.ends_with('\n') {
        source.push_str(ending);
    }
    if !source.is_empty() && !source.ends_with(&format!("{ending}{ending}")) {
        source.push_str(ending);
    }
    source.push('[');
    source.push_str(section);
    source.push(']');
    source.push_str(ending);
    for (key, value) in emitted {
        source.push_str(key);
        source.push('=');
        source.push_str(value);
        source.push_str(ending);
    }
    source
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::{
        compatibility::{ARCHITECTURE_REGISTRY_REVISION, installation_fingerprint},
        gguf::MetadataValue,
        llama::ToolEvidence,
        multimodal::Modality,
    };

    fn installation(help: &str) -> LlamaInstallation {
        let root = PathBuf::from(r"C:\runtime evidence 外部");
        LlamaInstallation {
            id: "runtime-test".into(),
            name: "runtime".into(),
            root_path: root.clone(),
            server: Some(ToolEvidence {
                path: root.join("llama-server.exe"),
                sha256: "a".repeat(64),
                version_output: "b10472".into(),
                help_output: help.into(),
                device_output: "CUDA device 0".into(),
            }),
            bench: None,
            fit_params: None,
            backend: Some("CUDA".into()),
            capabilities: BTreeSet::new(),
            discovered_at_unix_ms: 1,
        }
    }

    fn model(architecture: &str) -> ModelInfo {
        ModelInfo {
            id: "model-test".into(),
            path: PathBuf::from(r"D:\Models 外部\my model.gguf"),
            file_size: 123,
            sha256: "b".repeat(64),
            gguf_version: 3,
            tensor_count: 1,
            metadata_count: 3,
            name: Some("Evidence Model".into()),
            architecture: Some(architecture.into()),
            context_length: Some(32768),
            quantization_version: Some(2),
            general_type: Some("model".into()),
            file_type: Some(7),
            parameter_count: Some(1_000_000),
            tensor_type_counts: BTreeMap::from([(2, 1)]),
            metadata: BTreeMap::from([
                ("general.architecture".into(), MetadataValue::String(architecture.into())),
                ("general.file_type".into(), MetadataValue::UInt(7)),
                ("general.quantization_version".into(), MetadataValue::UInt(2)),
            ]),
            inspected_at_unix_ms: 1,
        }
    }

    fn compatible(model: &ModelInfo, installation: &LlamaInstallation) -> CompatibilityResult {
        CompatibilityResult {
            model_id: model.id.clone(),
            installation_id: installation.id.clone(),
            model_sha256: model.sha256.clone(),
            installation_fingerprint: installation_fingerprint(installation),
            registry_revision: ARCHITECTURE_REGISTRY_REVISION.into(),
            status: CompatibilityStatus::Compatible,
            reasons: Vec::new(),
            computed_at_unix_ms: 1,
        }
    }

    fn request<'a>(
        installation: &'a LlamaInstallation,
        model: &'a ModelInfo,
        compatibility: &'a CompatibilityResult,
        recommendations: &'a [RecommendedSetting],
        required_settings: &'a [RequiredSetting],
    ) -> ProfileGenerationRequest<'a> {
        static EMPTY_FEATURES: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
        ProfileGenerationRequest {
            section: "Evidence Profile",
            baseline_source: "[*]\r\n# preserve this\r\nthreads=4\r\n",
            installation,
            model,
            compatibility,
            projector: None,
            recommendations,
            required_settings,
            explicitly_disabled_features: EMPTY_FEATURES.get_or_init(BTreeSet::new),
            destination: ProfileDestination::Managed,
        }
    }

    #[test]
    fn emits_only_explicit_evidence_backed_supported_recommendations() {
        let runtime = installation("--model FILE --threads N --ctx-size N --batch-size N");
        let selected_model = model("qwen35");
        let compat = compatible(&selected_model, &runtime);
        let recommendations = vec![
            RecommendedSetting {
                key: "threads".into(),
                value: "8".into(),
                capability_aliases: vec!["--threads".into(), "-t".into()],
                basis: RecommendationBasis::Measured,
                evidence: vec!["benchmark run abc: 8 threads was stable winner".into()],
            },
            RecommendedSetting {
                key: "ctx-size".into(),
                value: "16384".into(),
                capability_aliases: vec!["--ctx-size".into(), "-c".into()],
                basis: RecommendationBasis::Derived,
                evidence: vec!["requested context=16384; model metadata max=32768".into()],
            },
            RecommendedSetting {
                key: "n-gpu-layers".into(),
                value: "99".into(),
                capability_aliases: vec!["--n-gpu-layers".into()],
                basis: RecommendationBasis::Derived,
                evidence: vec!["memory-fit calculation".into()],
            },
        ];

        let generated = generate_profile(&request(
            &runtime,
            &selected_model,
            &compat,
            &recommendations,
            &[],
        ))
        .unwrap();

        assert!(generated.source.contains("model=D:\\Models 外部\\my model.gguf\r\n"));
        assert!(generated.source.contains("threads=8\r\n"));
        assert!(generated.source.contains("ctx-size=16384\r\n"));
        assert!(!generated.source.contains("n-gpu-layers=99"));
        assert!(
            generated
                .unresolved
                .iter()
                .any(|item| item.key == "n-gpu-layers")
        );
        assert_eq!(
            generated.provenance["threads"].source,
            ProfileProvenanceSource::MeasuredHardware
        );
        assert!(generated.can_apply());
        assert!(!generated.diff.is_empty());
        assert!(generated.source.starts_with("[*]\r\n# preserve this\r\nthreads=4\r\n"));
    }

    #[test]
    fn recommendation_without_evidence_is_rejected_instead_of_fabricated() {
        let runtime = installation("--model FILE --threads N");
        let selected_model = model("qwen35");
        let compat = compatible(&selected_model, &runtime);
        let recommendations = vec![RecommendedSetting {
            key: "threads".into(),
            value: "8".into(),
            capability_aliases: vec!["--threads".into()],
            basis: RecommendationBasis::Derived,
            evidence: Vec::new(),
        }];
        assert_eq!(
            generate_profile(&request(
                &runtime,
                &selected_model,
                &compat,
                &recommendations,
                &[],
            ))
            .unwrap_err(),
            ProfileGenerationError::RecommendationEvidenceMissing {
                key: "threads".into()
            }
        );
    }

    #[test]
    fn unsupported_required_feature_is_hard_failure_unless_explicitly_disabled() {
        let runtime = installation("--model FILE");
        let selected_model = model("qwen35");
        let compat = compatible(&selected_model, &runtime);
        let required = vec![RequiredSetting {
            key: "spec-type".into(),
            value: "draft-mtp".into(),
            capability_aliases: vec!["--spec-type".into()],
            evidence: vec!["user requires native MTP workflow".into()],
            explicitly_disabled: false,
        }];
        assert_eq!(
            generate_profile(&request(
                &runtime,
                &selected_model,
                &compat,
                &[],
                &required,
            ))
            .unwrap_err(),
            ProfileGenerationError::RequiredCapabilityMissing {
                option: "--spec-type".into()
            }
        );

        let disabled = vec![RequiredSetting {
            explicitly_disabled: true,
            ..required[0].clone()
        }];
        let generated = generate_profile(&request(
            &runtime,
            &selected_model,
            &compat,
            &[],
            &disabled,
        ))
        .unwrap();
        assert!(!generated.source.contains("spec-type"));
        assert!(generated.unresolved.iter().any(|item| item.key == "spec-type"));
    }

    #[test]
    fn required_multimodal_projector_is_never_silently_dropped() {
        let runtime = installation("--model FILE --mmproj FILE");
        let selected_model = model("qwen3vl");
        let compat = compatible(&selected_model, &runtime);
        assert_eq!(
            generate_profile(&request(
                &runtime,
                &selected_model,
                &compat,
                &[],
                &[],
            ))
            .unwrap_err(),
            ProfileGenerationError::RequiredProjectorMissing
        );

        let projector = ProjectorInfo {
            id: "projector-test".into(),
            path: PathBuf::from(r"D:\Models 外部\视觉 projector.gguf"),
            file_size: 10,
            sha256: "c".repeat(64),
            name: Some("Vision projector".into()),
            general_type: Some("mmproj".into()),
            architecture: Some("clip".into()),
            projector_type: Some("mlp".into()),
            modalities: BTreeSet::from([Modality::Vision]),
            source_model_hint: None,
            inspected_at_unix_ms: 1,
        };
        let mut req = request(&runtime, &selected_model, &compat, &[], &[]);
        req.projector = Some(&projector);
        let generated = generate_profile(&req).unwrap();
        assert!(generated.source.contains("mmproj=D:\\Models 外部\\视觉 projector.gguf"));
        assert_eq!(
            generated.provenance["mmproj"].source,
            ProfileProvenanceSource::ProjectorEvidence
        );
    }

    #[test]
    fn stale_unknown_or_mismatched_compatibility_cannot_generate_profile() {
        let runtime = installation("--model FILE");
        let selected_model = model("qwen35");
        let mut compat = compatible(&selected_model, &runtime);
        compat.status = CompatibilityStatus::Unknown;
        assert_eq!(
            generate_profile(&request(&runtime, &selected_model, &compat, &[], &[])).unwrap_err(),
            ProfileGenerationError::UnsafeCompatibility {
                status: "unknown".into()
            }
        );

        let mut mismatch = compatible(&selected_model, &runtime);
        mismatch.model_id = "other".into();
        assert_eq!(
            generate_profile(&request(&runtime, &selected_model, &mismatch, &[], &[])).unwrap_err(),
            ProfileGenerationError::CompatibilityIdentityMismatch
        );
    }

    #[test]
    fn destination_and_preapply_diff_are_retained_without_writing() {
        let runtime = installation("--model FILE");
        let selected_model = model("qwen35");
        let compat = compatible(&selected_model, &runtime);
        let mut req = request(&runtime, &selected_model, &compat, &[], &[]);
        req.destination = ProfileDestination::External(PathBuf::from(
            r"D:\User Configs 外部\models.ini",
        ));
        let generated = generate_profile(&req).unwrap();
        assert_eq!(generated.destination, req.destination);
        assert!(!generated.diff.is_empty());
        assert!(generated.can_apply());
    }

    #[test]
    fn existing_section_is_not_overwritten_by_generator() {
        let runtime = installation("--model FILE");
        let selected_model = model("qwen35");
        let compat = compatible(&selected_model, &runtime);
        let mut req = request(&runtime, &selected_model, &compat, &[], &[]);
        req.baseline_source = "[*]\nthreads=4\n[Evidence Profile]\nmodel=old.gguf\n";
        assert_eq!(
            generate_profile(&req).unwrap_err(),
            ProfileGenerationError::SectionAlreadyExists("Evidence Profile".into())
        );
    }
}
