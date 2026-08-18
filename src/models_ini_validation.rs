use std::collections::{BTreeMap, BTreeSet};

use crate::{
    llama::LlamaInstallation,
    models_ini::{ModelsIniDocument, ModelsIniLineKind, ModelsIniParseError},
    models_ini_effective::{EffectiveValueSource, compute_effective_config},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ValidationSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub severity: ValidationSeverity,
    pub code: String,
    pub key: Option<String>,
    pub message: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn can_apply(&self) -> bool {
        !self
            .issues
            .iter()
            .any(|issue| issue.severity == ValidationSeverity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &ValidationIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &ValidationIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Warning)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxValidation {
    Valid(ModelsIniDocument),
    Invalid(ModelsIniParseError),
}

pub fn validate_syntax(source: &str) -> SyntaxValidation {
    match ModelsIniDocument::parse(source) {
        Ok(document) => SyntaxValidation::Valid(document),
        Err(error) => SyntaxValidation::Invalid(error),
    }
}

pub fn validate_semantics(
    document: &ModelsIniDocument,
    section: &str,
    installation: Option<&LlamaInstallation>,
) -> ValidationReport {
    let effective = compute_effective_config(document, section);
    let mut issues = Vec::new();

    for diagnostic in document.diagnostics() {
        issues.push(ValidationIssue {
            severity: ValidationSeverity::Warning,
            code: "duplicate_definition".into(),
            key: None,
            message: diagnostic.message.clone(),
            evidence: vec![
                format!("line={}", diagnostic.line),
                format!("previous_line={}", diagnostic.previous_line),
            ],
        });
    }

    let server_help = installation
        .and_then(|value| value.server.as_ref())
        .map(|tool| tool.help_output.as_str());

    if installation.is_some() && server_help.is_none() {
        issues.push(ValidationIssue {
            severity: ValidationSeverity::Error,
            code: "server_missing".into(),
            key: None,
            message: "selected llama.cpp installation has no llama-server capability evidence"
                .into(),
            evidence: installation
                .map(|value| vec![format!("installation={}", value.root_path.display())])
                .unwrap_or_default(),
        });
    }

    for (key, value) in &effective.values {
        if let Some(help) = server_help {
            let flag = normalize_flag(key);
            if !help_has_option(help, &flag) {
                issues.push(ValidationIssue {
                    severity: ValidationSeverity::Warning,
                    code: "capability_unknown".into(),
                    key: Some(key.clone()),
                    message: format!(
                        "selected llama-server help does not expose {flag}; support is unknown and must not be assumed"
                    ),
                    evidence: installation
                        .and_then(|value| value.server.as_ref())
                        .map(|tool| vec![format!("server_sha256={}", tool.sha256)])
                        .unwrap_or_default(),
                });
                continue;
            }
        } else if installation.is_none() {
            issues.push(ValidationIssue {
                severity: ValidationSeverity::Warning,
                code: "installation_unselected".into(),
                key: Some(key.clone()),
                message:
                    "no llama.cpp installation is selected; runtime capability cannot be proven"
                        .into(),
                evidence: Vec::new(),
            });
        }

        validate_known_value(key, &value.value, &mut issues);
    }

    validate_cross_field_constraints(&effective.values, &mut issues);
    issues.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.key.cmp(&right.key))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
    });

    ValidationReport { issues }
}

fn validate_known_value(key: &str, value: &str, issues: &mut Vec<ValidationIssue>) {
    match canonical_key(key) {
        "ctx-size" | "threads" | "threads-batch" | "batch-size" | "ubatch-size" => {
            match value.parse::<u64>() {
                Ok(number) if number > 0 => {}
                _ => issues.push(value_error(
                    key,
                    "positive_integer_required",
                    format!("{key} must be a positive integer"),
                    value,
                )),
            }
        }
        "n-gpu-layers" => {
            if value.parse::<u64>().is_err() {
                issues.push(value_error(
                    key,
                    "nonnegative_integer_required",
                    "n-gpu-layers must be a non-negative integer".into(),
                    value,
                ));
            }
        }
        "port" => match value.parse::<u16>() {
            Ok(port) if port > 0 => {}
            _ => issues.push(value_error(
                key,
                "port_out_of_range",
                "port must be an integer in the range 1..=65535".into(),
                value,
            )),
        },
        "host" | "model" | "mmproj" => {
            if value.trim().is_empty() {
                issues.push(value_error(
                    key,
                    "nonempty_value_required",
                    format!("{key} cannot be empty"),
                    value,
                ));
            }
        }
        _ => {}
    }
}

fn validate_cross_field_constraints(
    values: &BTreeMap<String, crate::models_ini_effective::EffectiveValue>,
    issues: &mut Vec<ValidationIssue>,
) {
    let batch = effective_u64(values, "batch-size");
    let ubatch = effective_u64(values, "ubatch-size");
    if let (Some(batch), Some(ubatch)) = (batch, ubatch)
        && ubatch > batch
    {
        issues.push(ValidationIssue {
            severity: ValidationSeverity::Error,
            code: "ubatch_exceeds_batch".into(),
            key: Some("ubatch-size".into()),
            message: "ubatch-size cannot exceed batch-size".into(),
            evidence: vec![
                format!("batch-size={batch}"),
                format!("ubatch-size={ubatch}"),
            ],
        });
    }
}

fn effective_u64(
    values: &BTreeMap<String, crate::models_ini_effective::EffectiveValue>,
    canonical: &str,
) -> Option<u64> {
    values.iter().find_map(|(key, value)| {
        (canonical_key(key) == canonical)
            .then(|| value.value.parse::<u64>().ok())
            .flatten()
    })
}

fn value_error(key: &str, code: &str, message: String, value: &str) -> ValidationIssue {
    ValidationIssue {
        severity: ValidationSeverity::Error,
        code: code.into(),
        key: Some(key.into()),
        message,
        evidence: vec![format!("value={value:?}")],
    }
}

fn canonical_key(key: &str) -> &str {
    key.trim_start_matches('-')
}

fn normalize_flag(key: &str) -> String {
    if key.starts_with('-') {
        key.to_owned()
    } else {
        format!("--{key}")
    }
}

fn help_has_option(help: &str, expected: &str) -> bool {
    help.split_whitespace().any(|token| {
        token.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '[' | ']' | '(' | ')' | '`'))
            == expected
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigValueSnapshot {
    pub value: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiffEntry {
    pub key: String,
    pub global_before: Option<String>,
    pub global_after: Option<String>,
    pub model_before: Option<String>,
    pub model_after: Option<String>,
    pub effective_before: Option<ConfigValueSnapshot>,
    pub effective_after: Option<ConfigValueSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigDiff {
    pub entries: Vec<ConfigDiffEntry>,
}

impl ConfigDiff {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn redacted(&self, secret_keys: &BTreeSet<String>) -> Self {
        let mut copy = self.clone();
        for entry in &mut copy.entries {
            if secret_keys.contains(&entry.key) || looks_sensitive_key(&entry.key) {
                redact_option(&mut entry.global_before);
                redact_option(&mut entry.global_after);
                redact_option(&mut entry.model_before);
                redact_option(&mut entry.model_after);
                if let Some(value) = &mut entry.effective_before {
                    value.value = "<redacted>".into();
                }
                if let Some(value) = &mut entry.effective_after {
                    value.value = "<redacted>".into();
                }
            }
        }
        copy
    }
}

pub fn diff_configs(
    before: &ModelsIniDocument,
    after: &ModelsIniDocument,
    section: &str,
) -> ConfigDiff {
    let before_effective = compute_effective_config(before, section);
    let after_effective = compute_effective_config(after, section);
    let before_source = source_values(before, section);
    let after_source = source_values(after, section);

    let mut keys = BTreeSet::new();
    keys.extend(before_source.keys().cloned());
    keys.extend(after_source.keys().cloned());
    keys.extend(before_effective.values.keys().cloned());
    keys.extend(after_effective.values.keys().cloned());

    let mut entries = Vec::new();
    for key in keys {
        let before_sources = before_source.get(&key).cloned().unwrap_or_default();
        let after_sources = after_source.get(&key).cloned().unwrap_or_default();
        let entry = ConfigDiffEntry {
            key: key.clone(),
            global_before: before_sources.global,
            global_after: after_sources.global,
            model_before: before_sources.model,
            model_after: after_sources.model,
            effective_before: before_effective.values.get(&key).map(snapshot),
            effective_after: after_effective.values.get(&key).map(snapshot),
        };

        if entry.global_before != entry.global_after
            || entry.model_before != entry.model_after
            || entry.effective_before != entry.effective_after
        {
            entries.push(entry);
        }
    }

    ConfigDiff { entries }
}

#[derive(Debug, Clone, Default)]
struct SourceValues {
    global: Option<String>,
    model: Option<String>,
}

fn source_values(document: &ModelsIniDocument, section: &str) -> BTreeMap<String, SourceValues> {
    let mut values = BTreeMap::<String, SourceValues>::new();
    for line in document.lines() {
        if let ModelsIniLineKind::KeyValue {
            section: line_section,
            key,
            value,
        } = &line.kind
        {
            let entry = values.entry(key.clone()).or_default();
            if line_section == "*" {
                entry.global = Some(value.clone());
            }
            if line_section == section {
                entry.model = Some(value.clone());
            }
        }
    }
    values
}

fn snapshot(value: &crate::models_ini_effective::EffectiveValue) -> ConfigValueSnapshot {
    ConfigValueSnapshot {
        value: value.value.clone(),
        source: match &value.source {
            EffectiveValueSource::GlobalDefault { line } => format!("global line {line}"),
            EffectiveValueSource::ModelOverride { section, line } => {
                format!("model [{section}] line {line}")
            }
        },
    }
}

fn looks_sensitive_key(key: &str) -> bool {
    let normalized = canonical_key(key).to_ascii_lowercase();
    ["api-key", "apikey", "token", "password", "secret", "bearer"]
        .iter()
        .any(|needle| normalized.contains(needle))
}

fn redact_option(value: &mut Option<String>) {
    if value.is_some() {
        *value = Some("<redacted>".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llama::ToolEvidence;
    use std::{collections::BTreeSet, path::PathBuf};

    fn installation(help: &str) -> LlamaInstallation {
        let tool = ToolEvidence {
            path: PathBuf::from(r"C:\llama cpp 外部\llama-server.exe"),
            sha256: "a".repeat(64),
            version_output: "version".into(),
            help_output: help.into(),
            device_output: "CPU".into(),
        };
        LlamaInstallation {
            id: "installation-test".into(),
            name: "test".into(),
            root_path: PathBuf::from(r"C:\llama cpp 外部"),
            server: Some(tool),
            bench: None,
            fit_params: None,
            backend: Some("CPU".into()),
            capabilities: BTreeSet::new(),
            discovered_at_unix_ms: 1,
        }
    }

    #[test]
    fn syntax_and_semantic_validation_are_separate() {
        assert!(matches!(
            validate_syntax("[*]\nthis is malformed\n"),
            SyntaxValidation::Invalid(_)
        ));

        let SyntaxValidation::Valid(document) = validate_syntax("[*]\nthreads=0\n") else {
            panic!("source must parse before semantic validation")
        };
        let report = validate_semantics(&document, "model", Some(&installation("--threads N")));
        assert!(!report.can_apply());
        assert!(
            report
                .errors()
                .any(|issue| issue.code == "positive_integer_required")
        );
    }

    #[test]
    fn unknown_runtime_capability_is_warning_not_fake_support() {
        let document = ModelsIniDocument::parse("[*]\nfuture-option=42\n").unwrap();
        let report = validate_semantics(&document, "model", Some(&installation("--model FILE")));
        assert!(report.can_apply());
        assert!(
            report
                .warnings()
                .any(|issue| issue.code == "capability_unknown")
        );
    }

    #[test]
    fn missing_server_is_hard_error_when_installation_selected() {
        let document = ModelsIniDocument::parse("[*]\nthreads=8\n").unwrap();
        let mut selected = installation("--threads N");
        selected.server = None;
        let report = validate_semantics(&document, "model", Some(&selected));
        assert!(!report.can_apply());
        assert!(report.errors().any(|issue| issue.code == "server_missing"));
    }

    #[test]
    fn cross_field_batch_constraint_is_deterministic_error() {
        let document = ModelsIniDocument::parse(
            "[*]\nbatch-size=128\nubatch-size=256\n[model]\nbatch-size=64\n",
        )
        .unwrap();
        let report = validate_semantics(
            &document,
            "model",
            Some(&installation("--batch-size N --ubatch-size N")),
        );
        assert!(!report.can_apply());
        assert!(
            report
                .errors()
                .any(|issue| issue.code == "ubatch_exceeds_batch")
        );
    }

    #[test]
    fn valid_known_values_are_applicable() {
        let document = ModelsIniDocument::parse(
            "[*]\nthreads=8\nctx-size=65536\nbatch-size=512\nubatch-size=128\nport=8080\n",
        )
        .unwrap();
        let report = validate_semantics(
            &document,
            "model",
            Some(&installation(
                "--threads N --ctx-size N --batch-size N --ubatch-size N --port N",
            )),
        );
        assert!(report.can_apply(), "{:?}", report.issues);
    }

    #[test]
    fn diff_shows_source_and_effective_change_with_provenance() {
        let before =
            ModelsIniDocument::parse("[*]\nthreads=8\nctx-size=4096\n[model]\nthreads=12\n")
                .unwrap();
        let after =
            ModelsIniDocument::parse("[*]\nthreads=10\nctx-size=8192\n[model]\nthreads=12\n")
                .unwrap();

        let diff = diff_configs(&before, &after, "model");
        assert_eq!(diff.entries.len(), 2);
        let threads = diff
            .entries
            .iter()
            .find(|entry| entry.key == "threads")
            .unwrap();
        assert_eq!(threads.global_before.as_deref(), Some("8"));
        assert_eq!(threads.global_after.as_deref(), Some("10"));
        assert_eq!(threads.effective_before, threads.effective_after);

        let context = diff
            .entries
            .iter()
            .find(|entry| entry.key == "ctx-size")
            .unwrap();
        assert_eq!(context.effective_before.as_ref().unwrap().value, "4096");
        assert_eq!(context.effective_after.as_ref().unwrap().value, "8192");
        assert!(
            context
                .effective_after
                .as_ref()
                .unwrap()
                .source
                .contains("global")
        );
    }

    #[test]
    fn diagnostic_redaction_never_mutates_actual_documents_or_unredacted_diff() {
        let before = ModelsIniDocument::parse("[*]\napi-key=old-secret\n").unwrap();
        let after = ModelsIniDocument::parse("[*]\napi-key=new-secret\n").unwrap();
        let diff = diff_configs(&before, &after, "model");
        let redacted = diff.redacted(&BTreeSet::new());

        assert_eq!(before.serialize(), "[*]\napi-key=old-secret\n");
        assert_eq!(after.serialize(), "[*]\napi-key=new-secret\n");
        assert_eq!(diff.entries[0].global_after.as_deref(), Some("new-secret"));
        assert_eq!(
            redacted.entries[0].global_after.as_deref(),
            Some("<redacted>")
        );
        assert_eq!(
            redacted.entries[0].effective_after.as_ref().unwrap().value,
            "<redacted>"
        );
    }

    #[test]
    fn validation_output_order_is_stable() {
        let document =
            ModelsIniDocument::parse("[*]\nz-future=x\na-future=y\nthreads=0\n").unwrap();
        let selected = installation("--model FILE --threads N");
        let first = validate_semantics(&document, "model", Some(&selected));
        let second = validate_semantics(&document, "model", Some(&selected));
        assert_eq!(first, second);
    }
}
