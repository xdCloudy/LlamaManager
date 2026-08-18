use std::collections::BTreeMap;

use crate::models_ini::{ModelsIniDiagnostic, ModelsIniDocument, ModelsIniLineKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveValueSource {
    GlobalDefault { line: usize },
    ModelOverride { section: String, line: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveValue {
    pub key: String,
    pub value: String,
    pub source: EffectiveValueSource,
}

impl EffectiveValue {
    pub fn is_inherited(&self) -> bool {
        matches!(self.source, EffectiveValueSource::GlobalDefault { .. })
    }

    pub fn is_override(&self) -> bool {
        matches!(self.source, EffectiveValueSource::ModelOverride { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
    pub section: String,
    pub values: BTreeMap<String, EffectiveValue>,
    pub parser_diagnostics: Vec<ModelsIniDiagnostic>,
}

impl EffectiveConfig {
    pub fn get(&self, key: &str) -> Option<&EffectiveValue> {
        self.values.get(key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideEdit {
    /// Create or replace a per-model value. An empty string is still an explicit
    /// override and must not be confused with reset-to-inherited.
    Set(String),
    /// Remove the per-model override so the global value, if any, becomes
    /// effective. Document mutation is intentionally left to the editor layer.
    ResetToInherited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveEditPreview {
    Override {
        key: String,
        value: String,
    },
    Inherited {
        key: String,
        value: String,
        global_line: usize,
    },
    Unset {
        key: String,
    },
}

pub fn compute_effective_config(document: &ModelsIniDocument, section: &str) -> EffectiveConfig {
    let mut values = BTreeMap::new();

    // Apply global defaults first. Because the parser preserves duplicates and
    // the semantic contract is explicit last-definition-wins, later global
    // definitions replace earlier ones deterministically.
    for line in document.lines() {
        if let ModelsIniLineKind::KeyValue {
            section: line_section,
            key,
            value,
        } = &line.kind
            && line_section == "*"
        {
            values.insert(
                key.clone(),
                EffectiveValue {
                    key: key.clone(),
                    value: value.clone(),
                    source: EffectiveValueSource::GlobalDefault {
                        line: line.line_number,
                    },
                },
            );
        }
    }

    // Overlay the requested model section. Unknown keys receive exactly the
    // same treatment as known ones; this layer deliberately invents no
    // llama.cpp option semantics.
    for line in document.lines() {
        if let ModelsIniLineKind::KeyValue {
            section: line_section,
            key,
            value,
        } = &line.kind
            && line_section == section
        {
            values.insert(
                key.clone(),
                EffectiveValue {
                    key: key.clone(),
                    value: value.clone(),
                    source: EffectiveValueSource::ModelOverride {
                        section: section.to_owned(),
                        line: line.line_number,
                    },
                },
            );
        }
    }

    EffectiveConfig {
        section: section.to_owned(),
        values,
        parser_diagnostics: document.diagnostics().to_vec(),
    }
}

pub fn preview_override_edit(
    document: &ModelsIniDocument,
    _section: &str,
    key: &str,
    edit: OverrideEdit,
) -> EffectiveEditPreview {
    match edit {
        OverrideEdit::Set(value) => EffectiveEditPreview::Override {
            key: key.to_owned(),
            value,
        },
        OverrideEdit::ResetToInherited => document
            .lines()
            .iter()
            .rev()
            .find_map(|line| match &line.kind {
                ModelsIniLineKind::KeyValue {
                    section: line_section,
                    key: line_key,
                    value,
                } if line_section == "*" && line_key == key => {
                    Some(EffectiveEditPreview::Inherited {
                        key: key.to_owned(),
                        value: value.clone(),
                        global_line: line.line_number,
                    })
                }
                _ => None,
            })
            .unwrap_or_else(|| EffectiveEditPreview::Unset {
                key: key.to_owned(),
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models_ini::{ModelsIniDiagnosticKind, ModelsIniDocument};

    #[test]
    fn model_overrides_global_defaults_and_retains_provenance() {
        let document = ModelsIniDocument::parse(
            "[*]\nthreads=6\nctx-size=65536\n[agent]\nthreads=10\nmodel=C:\\Models\\Agent.gguf\n",
        )
        .unwrap();

        let effective = compute_effective_config(&document, "agent");

        let threads = effective.get("threads").unwrap();
        assert_eq!(threads.value, "10");
        assert_eq!(
            threads.source,
            EffectiveValueSource::ModelOverride {
                section: "agent".into(),
                line: 5
            }
        );
        assert!(threads.is_override());

        let context = effective.get("ctx-size").unwrap();
        assert_eq!(context.value, "65536");
        assert_eq!(
            context.source,
            EffectiveValueSource::GlobalDefault { line: 3 }
        );
        assert!(context.is_inherited());
    }

    #[test]
    fn unknown_keys_are_preserved_and_inherited_without_interpretation() {
        let document = ModelsIniDocument::parse(
            "[*]\nfuture-option = globally unknown\n[模型 with spaces]\nother-new-key = 值\n",
        )
        .unwrap();

        let effective = compute_effective_config(&document, "模型 with spaces");
        assert_eq!(
            effective.get("future-option").unwrap().value,
            "globally unknown"
        );
        assert_eq!(effective.get("other-new-key").unwrap().value, "值");
    }

    #[test]
    fn empty_value_is_an_explicit_override_not_a_reset() {
        let document = ModelsIniDocument::parse("[*]\nfoo=global\n[model]\nfoo=\n").unwrap();
        let effective = compute_effective_config(&document, "model");
        let value = effective.get("foo").unwrap();

        assert_eq!(value.value, "");
        assert!(value.is_override());
    }

    #[test]
    fn reset_to_inherited_is_explicit_and_does_not_conflate_empty_string() {
        let document = ModelsIniDocument::parse("[*]\nfoo=global\n[model]\nfoo=local\n").unwrap();

        assert_eq!(
            preview_override_edit(&document, "model", "foo", OverrideEdit::ResetToInherited),
            EffectiveEditPreview::Inherited {
                key: "foo".into(),
                value: "global".into(),
                global_line: 2,
            }
        );
        assert_eq!(
            preview_override_edit(&document, "model", "foo", OverrideEdit::Set(String::new())),
            EffectiveEditPreview::Override {
                key: "foo".into(),
                value: String::new(),
            }
        );
    }

    #[test]
    fn reset_without_global_default_becomes_explicitly_unset() {
        let document = ModelsIniDocument::parse("[model]\nfoo=local\n").unwrap();
        assert_eq!(
            preview_override_edit(&document, "model", "foo", OverrideEdit::ResetToInherited),
            EffectiveEditPreview::Unset { key: "foo".into() }
        );
    }

    #[test]
    fn repeated_sections_and_keys_are_deterministic_last_definition_wins() {
        let document = ModelsIniDocument::parse(
            "[*]\nthreads=4\nthreads=6\n[agent]\nthreads=8\n[agent]\nthreads=12\n",
        )
        .unwrap();
        let effective = compute_effective_config(&document, "agent");

        assert_eq!(effective.get("threads").unwrap().value, "12");
        assert_eq!(
            effective.get("threads").unwrap().source,
            EffectiveValueSource::ModelOverride {
                section: "agent".into(),
                line: 7,
            }
        );
        assert!(
            effective
                .parser_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == ModelsIniDiagnosticKind::DuplicateSection)
        );
        assert!(
            effective
                .parser_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == ModelsIniDiagnosticKind::DuplicateKey)
        );
    }

    #[test]
    fn missing_model_section_still_exposes_global_defaults() {
        let document = ModelsIniDocument::parse("[*]\nthreads=8\nunknown=yes\n").unwrap();
        let effective = compute_effective_config(&document, "not-created-yet");

        assert_eq!(effective.get("threads").unwrap().value, "8");
        assert_eq!(effective.get("unknown").unwrap().value, "yes");
    }
}
