use std::{collections::HashMap, error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    None,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::None => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelsIniLineKind {
    Blank,
    Comment,
    Section {
        name: String,
    },
    KeyValue {
        section: String,
        key: String,
        value: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsIniLine {
    pub line_number: usize,
    pub raw: String,
    pub ending: LineEnding,
    pub kind: ModelsIniLineKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsIniDiagnosticKind {
    DuplicateSection,
    DuplicateKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsIniDiagnostic {
    pub kind: ModelsIniDiagnosticKind,
    pub line: usize,
    pub previous_line: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsIniParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub context: String,
}

impl fmt::Display for ModelsIniParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "models.ini parse error at line {}, column {}: {} (input: {:?})",
            self.line, self.column, self.message, self.context
        )
    }
}

impl Error for ModelsIniParseError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsIniDocument {
    lines: Vec<ModelsIniLine>,
    diagnostics: Vec<ModelsIniDiagnostic>,
}

impl ModelsIniDocument {
    pub fn parse(source: &str) -> Result<Self, ModelsIniParseError> {
        parse_models_ini(source)
    }

    pub fn lines(&self) -> &[ModelsIniLine] {
        &self.lines
    }

    pub fn diagnostics(&self) -> &[ModelsIniDiagnostic] {
        &self.diagnostics
    }

    pub fn serialize(&self) -> String {
        let mut output = String::new();
        for line in &self.lines {
            output.push_str(&line.raw);
            output.push_str(line.ending.as_str());
        }
        output
    }

    pub fn section_names(&self) -> Vec<&str> {
        let mut sections = Vec::new();
        for line in &self.lines {
            if let ModelsIniLineKind::Section { name } = &line.kind
                && !sections.contains(&name.as_str())
            {
                sections.push(name.as_str());
            }
        }
        sections
    }

    pub fn entries_in(&self, section: &str) -> Vec<(&str, &str)> {
        self.lines
            .iter()
            .filter_map(|line| match &line.kind {
                ModelsIniLineKind::KeyValue {
                    section: line_section,
                    key,
                    value,
                } if line_section == section => Some((key.as_str(), value.as_str())),
                _ => None,
            })
            .collect()
    }

    /// Returns the last definition of a key in a logical section.
    ///
    /// Duplicate sections and keys are preserved losslessly and reported through
    /// diagnostics. Semantic lookup intentionally uses last-definition-wins so
    /// callers never have to guess which duplicate is effective.
    pub fn last_value(&self, section: &str, key: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|line| match &line.kind {
            ModelsIniLineKind::KeyValue {
                section: line_section,
                key: line_key,
                value,
            } if line_section == section && line_key == key => Some(value.as_str()),
            _ => None,
        })
    }
}

pub fn parse_models_ini(source: &str) -> Result<ModelsIniDocument, ModelsIniParseError> {
    let mut lines = Vec::new();
    let mut diagnostics = Vec::new();
    let mut current_section: Option<String> = None;
    let mut first_section_line: HashMap<String, usize> = HashMap::new();
    let mut last_key_line: HashMap<(String, String), usize> = HashMap::new();

    for (index, raw_with_ending) in source.split_inclusive('\n').enumerate() {
        let line_number = index + 1;
        let (raw, ending) = split_line_ending(raw_with_ending);
        let trimmed = raw.trim();

        let kind = if trimmed.is_empty() {
            ModelsIniLineKind::Blank
        } else if trimmed.starts_with('#') || trimmed.starts_with(';') {
            ModelsIniLineKind::Comment
        } else if trimmed.starts_with('[') || trimmed.ends_with(']') {
            if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
                return Err(parse_error(
                    line_number,
                    1,
                    "malformed section header; expected [section]",
                    raw,
                ));
            }

            let name = trimmed[1..trimmed.len() - 1].trim();
            if name.is_empty() {
                return Err(parse_error(
                    line_number,
                    2,
                    "section name cannot be empty",
                    raw,
                ));
            }
            if name.contains('[') || name.contains(']') {
                return Err(parse_error(
                    line_number,
                    2,
                    "section name contains an unexpected bracket",
                    raw,
                ));
            }

            let name = name.to_owned();
            if let Some(previous_line) = first_section_line.get(&name).copied() {
                diagnostics.push(ModelsIniDiagnostic {
                    kind: ModelsIniDiagnosticKind::DuplicateSection,
                    line: line_number,
                    previous_line,
                    message: format!(
                        "section [{name}] is repeated; entries are treated as one logical section and later key definitions win"
                    ),
                });
            } else {
                first_section_line.insert(name.clone(), line_number);
            }
            current_section = Some(name.clone());
            ModelsIniLineKind::Section { name }
        } else if let Some((raw_key, raw_value)) = raw.split_once('=') {
            let Some(section) = current_section.as_ref() else {
                return Err(parse_error(
                    line_number,
                    1,
                    "key/value entry appears before the first section",
                    raw,
                ));
            };

            let key = raw_key.trim();
            if key.is_empty() {
                return Err(parse_error(
                    line_number,
                    1,
                    "configuration key cannot be empty",
                    raw,
                ));
            }

            let key = key.to_owned();
            let value = raw_value.trim().to_owned();
            let identity = (section.clone(), key.clone());
            if let Some(previous_line) = last_key_line.insert(identity, line_number) {
                diagnostics.push(ModelsIniDiagnostic {
                    kind: ModelsIniDiagnosticKind::DuplicateKey,
                    line: line_number,
                    previous_line,
                    message: format!(
                        "key {key:?} is repeated in section [{section}]; the last definition is effective"
                    ),
                });
            }

            ModelsIniLineKind::KeyValue {
                section: section.clone(),
                key,
                value,
            }
        } else {
            return Err(parse_error(
                line_number,
                1,
                "expected a comment, section header, blank line, or key=value entry",
                raw,
            ));
        };

        lines.push(ModelsIniLine {
            line_number,
            raw: raw.to_owned(),
            ending,
            kind,
        });
    }

    Ok(ModelsIniDocument { lines, diagnostics })
}

fn split_line_ending(line: &str) -> (&str, LineEnding) {
    if let Some(content) = line.strip_suffix("\r\n") {
        (content, LineEnding::CrLf)
    } else if let Some(content) = line.strip_suffix('\n') {
        (content, LineEnding::Lf)
    } else {
        (line, LineEnding::None)
    }
}

fn parse_error(line: usize, column: usize, message: &str, context: &str) -> ModelsIniParseError {
    ModelsIniParseError {
        line,
        column,
        message: message.to_owned(),
        context: context.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_op_round_trip_preserves_comments_unknown_keys_unicode_and_mixed_endings() {
        let source = concat!(
            "; user-owned file\r\n",
            "# keep this comment exactly\r\n",
            "\r\n",
            "[*]\r\n",
            "ctx-size = 131072\r\n",
            "unknown.future.flag = value with spaces 模型\r\n",
            "\r\n",
            "[Qwen 3.5 模型]\n",
            "model = D:\\\\AI Models\\\\Qwen 模型.gguf\n",
            "threads=8"
        );

        let document = ModelsIniDocument::parse(source).unwrap();
        assert_eq!(document.serialize(), source);
        assert_eq!(document.section_names(), vec!["*", "Qwen 3.5 模型"]);
        assert_eq!(
            document.last_value("*", "unknown.future.flag"),
            Some("value with spaces 模型")
        );
        assert_eq!(document.lines().last().unwrap().ending, LineEnding::None);
    }

    #[test]
    fn duplicate_sections_and_keys_are_preserved_with_explicit_last_wins_semantics() {
        let source = "[*]\nthreads=4\n[*]\nthreads=8\nthreads=12\n";
        let document = ModelsIniDocument::parse(source).unwrap();

        assert_eq!(document.serialize(), source);
        assert_eq!(document.last_value("*", "threads"), Some("12"));
        assert_eq!(document.diagnostics().len(), 3);
        assert_eq!(
            document.diagnostics()[0].kind,
            ModelsIniDiagnosticKind::DuplicateSection
        );
        assert_eq!(
            document.diagnostics()[1].kind,
            ModelsIniDiagnosticKind::DuplicateKey
        );
        assert_eq!(
            document.diagnostics()[2].kind,
            ModelsIniDiagnosticKind::DuplicateKey
        );
    }

    #[test]
    fn malformed_input_reports_line_column_and_context_without_partial_document() {
        let error =
            ModelsIniDocument::parse("[*]\nthreads=8\nthis is broken\nmodel=x.gguf\n").unwrap_err();

        assert_eq!(error.line, 3);
        assert_eq!(error.column, 1);
        assert_eq!(error.context, "this is broken");
        assert!(error.message.contains("key=value"));
    }

    #[test]
    fn key_before_section_is_rejected() {
        let error = ModelsIniDocument::parse("threads=8\n[*]\n").unwrap_err();
        assert_eq!(error.line, 1);
        assert!(error.message.contains("before the first section"));
    }

    #[test]
    fn malformed_and_empty_section_headers_are_rejected() {
        for source in ["[*\n", "*]\n", "[]\n", "[[model]]\n"] {
            assert!(ModelsIniDocument::parse(source).is_err(), "{source:?}");
        }
    }

    #[test]
    fn heavily_commented_fixture_round_trips_exactly() {
        let source = concat!(
            "; generated elsewhere - preserve\r\n",
            "; another comment\r\n",
            "\r\n",
            "[*]\r\n",
            "# global defaults\r\n",
            "threads = 8\r\n",
            "cache-prompt = true\r\n",
            "\r\n",
            "; per-model override\r\n",
            "[agent-main]\r\n",
            "model = D:\\\\models\\\\Agent Main.gguf\r\n",
            "ctx-size = 262144\r\n",
            "experimental-key = untouched\r\n"
        );
        let document = ModelsIniDocument::parse(source).unwrap();
        assert_eq!(document.serialize().as_bytes(), source.as_bytes());
        assert_eq!(
            document.entries_in("agent-main"),
            vec![
                ("model", "D:\\\\models\\\\Agent Main.gguf"),
                ("ctx-size", "262144"),
                ("experimental-key", "untouched"),
            ]
        );
    }

    #[test]
    fn generated_round_trip_matrix_is_stable() {
        let endings = ["\n", "\r\n"];
        let comments = ["# comment", "; comment"];
        let values = ["simple", "value with spaces", "路径 模型"];

        for ending in endings {
            for comment in comments {
                for value in values {
                    for final_newline in [false, true] {
                        let mut source = format!(
                            "{comment}{ending}[*]{ending}unknown-key = {value}{ending}[model]{ending}model = C:\\\\Models\\\\模型.gguf"
                        );
                        if final_newline {
                            source.push_str(ending);
                        }

                        let document = ModelsIniDocument::parse(&source).unwrap();
                        assert_eq!(document.serialize(), source);
                    }
                }
            }
        }
    }
}
