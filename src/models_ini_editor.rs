use thiserror::Error;

use crate::{
    llama::LlamaInstallation,
    models_ini::{LineEnding, ModelsIniDocument, ModelsIniLineKind, ModelsIniParseError},
    models_ini_effective::{EffectiveConfig, compute_effective_config},
    models_ini_validation::{ConfigDiff, ValidationReport, diff_configs, validate_semantics},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Structured,
    Raw,
}

#[derive(Debug, Error)]
pub enum EditorSessionError {
    #[error("raw editor contains invalid models.ini text: {0}")]
    InvalidRawDraft(ModelsIniParseError),

    #[error("structured edit could not produce a valid models.ini document: {0}")]
    StructuredEditInvalid(ModelsIniParseError),
}

#[derive(Debug, Clone)]
pub struct ModelsIniEditorSession {
    loaded_source: String,
    canonical_source: String,
    raw_draft: String,
    document: ModelsIniDocument,
    raw_error: Option<ModelsIniParseError>,
    mode: EditorMode,
}

impl ModelsIniEditorSession {
    pub fn load(source: impl Into<String>) -> Result<Self, ModelsIniParseError> {
        let source = source.into();
        let document = ModelsIniDocument::parse(&source)?;
        Ok(Self {
            loaded_source: source.clone(),
            canonical_source: source.clone(),
            raw_draft: source,
            document,
            raw_error: None,
            mode: EditorMode::Structured,
        })
    }

    pub fn mode(&self) -> EditorMode {
        self.mode
    }

    pub fn switch_mode(&mut self, mode: EditorMode) -> Result<(), EditorSessionError> {
        if mode == EditorMode::Structured
            && let Some(error) = &self.raw_error
        {
            return Err(EditorSessionError::InvalidRawDraft(error.clone()));
        }
        self.mode = mode;
        Ok(())
    }

    pub fn document(&self) -> &ModelsIniDocument {
        &self.document
    }

    pub fn canonical_source(&self) -> &str {
        &self.canonical_source
    }

    pub fn raw_draft(&self) -> &str {
        &self.raw_draft
    }

    pub fn raw_error(&self) -> Option<&ModelsIniParseError> {
        self.raw_error.as_ref()
    }

    pub fn is_dirty(&self) -> bool {
        self.raw_draft != self.loaded_source
    }

    pub fn apply_raw_edit(&mut self, source: impl Into<String>) -> Result<(), ModelsIniParseError> {
        let source = source.into();
        self.raw_draft = source.clone();
        match ModelsIniDocument::parse(&source) {
            Ok(document) => {
                self.document = document;
                self.canonical_source = source;
                self.raw_error = None;
                Ok(())
            }
            Err(error) => {
                self.raw_error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub fn set_value(
        &mut self,
        section: &str,
        key: &str,
        value: &str,
    ) -> Result<(), EditorSessionError> {
        self.ensure_structured_edit_safe()?;
        let source = set_value_in_source(&self.document, section, key, value);
        self.commit_structured_source(source)
    }

    pub fn reset_to_inherited(
        &mut self,
        section: &str,
        key: &str,
    ) -> Result<(), EditorSessionError> {
        self.ensure_structured_edit_safe()?;
        let source = remove_key_from_section(&self.document, section, key);
        self.commit_structured_source(source)
    }

    pub fn effective_config(&self, section: &str) -> Result<EffectiveConfig, EditorSessionError> {
        self.ensure_canonical_available()?;
        Ok(compute_effective_config(&self.document, section))
    }

    pub fn validation(
        &self,
        section: &str,
        installation: Option<&LlamaInstallation>,
    ) -> Result<ValidationReport, EditorSessionError> {
        self.ensure_canonical_available()?;
        Ok(validate_semantics(&self.document, section, installation))
    }

    pub fn diff_from_loaded(&self, section: &str) -> Result<ConfigDiff, EditorSessionError> {
        self.ensure_canonical_available()?;
        let loaded = ModelsIniDocument::parse(&self.loaded_source)
            .expect("loaded source was parsed when the editor session was created");
        Ok(diff_configs(&loaded, &self.document, section))
    }

    pub fn revert_to_loaded(&mut self) {
        let document = ModelsIniDocument::parse(&self.loaded_source)
            .expect("loaded source was parsed when the editor session was created");
        self.document = document;
        self.canonical_source.clone_from(&self.loaded_source);
        self.raw_draft.clone_from(&self.loaded_source);
        self.raw_error = None;
    }

    fn ensure_structured_edit_safe(&self) -> Result<(), EditorSessionError> {
        self.ensure_canonical_available()
    }

    fn ensure_canonical_available(&self) -> Result<(), EditorSessionError> {
        if let Some(error) = &self.raw_error {
            return Err(EditorSessionError::InvalidRawDraft(error.clone()));
        }
        Ok(())
    }

    fn commit_structured_source(&mut self, source: String) -> Result<(), EditorSessionError> {
        let document =
            ModelsIniDocument::parse(&source).map_err(EditorSessionError::StructuredEditInvalid)?;
        self.document = document;
        self.canonical_source = source.clone();
        self.raw_draft = source;
        self.raw_error = None;
        Ok(())
    }
}

fn set_value_in_source(
    document: &ModelsIniDocument,
    section: &str,
    key: &str,
    value: &str,
) -> String {
    let target_index = document
        .lines()
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, line)| match &line.kind {
            ModelsIniLineKind::KeyValue {
                section: line_section,
                key: line_key,
                ..
            } if line_section == section && line_key == key => Some(index),
            _ => None,
        });

    if let Some(target_index) = target_index {
        let mut output = String::new();
        for (index, line) in document.lines().iter().enumerate() {
            if index == target_index {
                output.push_str(&replace_value_preserving_whitespace(&line.raw, value));
            } else {
                output.push_str(&line.raw);
            }
            output.push_str(line.ending.as_str());
        }
        return output;
    }

    insert_new_value(document, section, key, value)
}

fn insert_new_value(document: &ModelsIniDocument, section: &str, key: &str, value: &str) -> String {
    let ending = preferred_line_ending(document);
    let lines = document.lines();
    let last_section_index =
        lines
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, line)| match &line.kind {
                ModelsIniLineKind::Section { name } if name == section => Some(index),
                _ => None,
            });

    match last_section_index {
        Some(section_index) => {
            let insert_index = lines
                .iter()
                .enumerate()
                .skip(section_index + 1)
                .find_map(|(index, line)| {
                    matches!(&line.kind, ModelsIniLineKind::Section { .. }).then_some(index)
                })
                .unwrap_or(lines.len());
            let mut output = String::new();
            for (index, line) in lines.iter().enumerate() {
                if index == insert_index {
                    output.push_str(key);
                    output.push('=');
                    output.push_str(value);
                    output.push_str(ending);
                }
                output.push_str(&line.raw);
                output.push_str(line.ending.as_str());
            }
            if insert_index == lines.len() {
                ensure_trailing_line_boundary(&mut output, ending);
                output.push_str(key);
                output.push('=');
                output.push_str(value);
                output.push_str(ending);
            }
            output
        }
        None => {
            let mut output = document.serialize();
            ensure_trailing_line_boundary(&mut output, ending);
            output.push('[');
            output.push_str(section);
            output.push(']');
            output.push_str(ending);
            output.push_str(key);
            output.push('=');
            output.push_str(value);
            output.push_str(ending);
            output
        }
    }
}

fn remove_key_from_section(document: &ModelsIniDocument, section: &str, key: &str) -> String {
    let mut output = String::new();
    for line in document.lines() {
        let should_remove = matches!(
            &line.kind,
            ModelsIniLineKind::KeyValue {
                section: line_section,
                key: line_key,
                ..
            } if line_section == section && line_key == key
        );
        if !should_remove {
            output.push_str(&line.raw);
            output.push_str(line.ending.as_str());
        }
    }
    output
}

fn replace_value_preserving_whitespace(raw: &str, value: &str) -> String {
    let Some((left, right)) = raw.split_once('=') else {
        return raw.to_owned();
    };
    let leading_len = right.len() - right.trim_start().len();
    let trailing_len = right.len() - right.trim_end().len();
    let trailing_len = trailing_len.min(right.len().saturating_sub(leading_len));

    let mut output =
        String::with_capacity(left.len() + value.len() + leading_len + trailing_len + 1);
    output.push_str(left);
    output.push('=');
    output.push_str(&right[..leading_len]);
    output.push_str(value);
    if trailing_len > 0 {
        output.push_str(&right[right.len() - trailing_len..]);
    }
    output
}

fn preferred_line_ending(document: &ModelsIniDocument) -> &'static str {
    document
        .lines()
        .iter()
        .find_map(|line| match line.ending {
            LineEnding::CrLf => Some("\r\n"),
            LineEnding::Lf => Some("\n"),
            LineEnding::None => None,
        })
        .unwrap_or("\n")
}

fn ensure_trailing_line_boundary(output: &mut String, ending: &str) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push_str(ending);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_raw_edit_updates_same_canonical_document_and_diff() {
        let mut session =
            ModelsIniEditorSession::load("[*]\nthreads=8\n[model]\nmodel=C:\\Models\\a.gguf\n")
                .unwrap();
        session.switch_mode(EditorMode::Raw).unwrap();
        session
            .apply_raw_edit("[*]\nthreads=10\n[model]\nmodel=C:\\Models\\a.gguf\n")
            .unwrap();

        assert_eq!(session.document().last_value("*", "threads"), Some("10"));
        assert!(session.is_dirty());
        assert!(!session.diff_from_loaded("model").unwrap().is_empty());
    }

    #[test]
    fn invalid_raw_draft_is_retained_and_blocks_structured_edits_and_switching() {
        let mut session = ModelsIniEditorSession::load("[*]\nthreads=8\n").unwrap();
        session.switch_mode(EditorMode::Raw).unwrap();
        let invalid = "[*]\nthis is invalid\n";
        assert!(session.apply_raw_edit(invalid).is_err());

        assert_eq!(session.raw_draft(), invalid);
        assert_eq!(session.document().last_value("*", "threads"), Some("8"));
        assert!(session.set_value("*", "threads", "12").is_err());
        assert!(session.switch_mode(EditorMode::Structured).is_err());
        assert!(session.diff_from_loaded("model").is_err());
    }

    #[test]
    fn structured_edit_preserves_comments_unknown_keys_crlf_and_spacing() {
        let source = "[*]\r\n# keep me\r\nfuture-option = unknown value  \r\nthreads = 8  \r\n[model]\r\nmodel = C:\\模型\\x.gguf\r\n";
        let mut session = ModelsIniEditorSession::load(source).unwrap();
        session.set_value("*", "threads", "12").unwrap();

        assert!(session.canonical_source().contains("# keep me\r\n"));
        assert!(
            session
                .canonical_source()
                .contains("future-option = unknown value  \r\n")
        );
        assert!(session.canonical_source().contains("threads = 12  \r\n"));
        assert!(session.canonical_source().contains("C:\\模型\\x.gguf"));
    }

    #[test]
    fn new_structured_value_is_inserted_into_existing_section() {
        let mut session = ModelsIniEditorSession::load(
            "[*]\nthreads=8\n[model]\nmodel=a.gguf\n[next]\nfoo=bar\n",
        )
        .unwrap();
        session.set_value("model", "ctx-size", "65536").unwrap();
        let source = session.canonical_source();

        let model = source.find("[model]").unwrap();
        let inserted = source.find("ctx-size=65536").unwrap();
        let next = source.find("[next]").unwrap();
        assert!(model < inserted && inserted < next);
    }

    #[test]
    fn missing_section_is_created_without_rewriting_existing_content() {
        let mut session = ModelsIniEditorSession::load("[*]\n# comment\nthreads=8").unwrap();
        session
            .set_value("模型 profile", "ctx-size", "8192")
            .unwrap();
        assert!(
            session
                .canonical_source()
                .starts_with("[*]\n# comment\nthreads=8\n")
        );
        assert!(
            session
                .canonical_source()
                .contains("[模型 profile]\nctx-size=8192\n")
        );
    }

    #[test]
    fn reset_removes_all_duplicate_overrides_so_global_value_is_inherited() {
        let mut session = ModelsIniEditorSession::load(
            "[*]\nthreads=6\n[model]\nthreads=8\nthreads=12\nother=x\n",
        )
        .unwrap();
        session.reset_to_inherited("model", "threads").unwrap();

        assert_eq!(session.document().last_value("model", "threads"), None);
        let effective = session.effective_config("model").unwrap();
        let threads = effective.get("threads").unwrap();
        assert_eq!(threads.value, "6");
        assert!(threads.is_inherited());
        assert_eq!(session.document().last_value("model", "other"), Some("x"));
    }

    #[test]
    fn revert_restores_loaded_state_even_after_invalid_raw_draft() {
        let source = "[*]\nthreads=8\n";
        let mut session = ModelsIniEditorSession::load(source).unwrap();
        session.switch_mode(EditorMode::Raw).unwrap();
        assert!(session.apply_raw_edit("[*]\nbad\n").is_err());
        session.revert_to_loaded();

        assert!(!session.is_dirty());
        assert_eq!(session.raw_draft(), source);
        assert!(session.raw_error().is_none());
        session.switch_mode(EditorMode::Structured).unwrap();
    }

    #[test]
    fn large_document_structured_edit_remains_deterministic() {
        let mut source = String::from("[*]\n");
        for index in 0..10_000 {
            source.push_str(&format!("unknown-{index}=value-{index}\n"));
        }
        source.push_str("[model]\nthreads=8\n");

        let mut first = ModelsIniEditorSession::load(source.clone()).unwrap();
        let mut second = ModelsIniEditorSession::load(source).unwrap();
        first.set_value("model", "threads", "12").unwrap();
        second.set_value("model", "threads", "12").unwrap();
        assert_eq!(first.canonical_source(), second.canonical_source());
        assert_eq!(first.document().last_value("model", "threads"), Some("12"));
    }
}
