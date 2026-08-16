use std::{path::PathBuf, process::Command};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    error::{LlamaManagerError, Result},
    gguf::ModelInfo,
    llama::{now_ms, LlamaInstallation},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSample {
    pub test: String,
    pub backend: Option<String>,
    pub model_type: Option<String>,
    pub prompt_tokens: u64,
    pub generated_tokens: u64,
    pub avg_tokens_per_second: f64,
    pub stddev_tokens_per_second: Option<f64>,
    pub samples_tokens_per_second: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkRun {
    pub id: String,
    pub installation_id: String,
    pub model_id: String,
    #[serde(default)]
    pub model_sha256: String,
    pub bench_binary: PathBuf,
    #[serde(default)]
    pub bench_sha256: String,
    #[serde(default)]
    pub backend: Option<String>,
    pub arguments: Vec<String>,
    pub started_at_unix_ms: u128,
    pub finished_at_unix_ms: u128,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub samples: Vec<BenchmarkSample>,
}

impl BenchmarkRun {
    pub fn prompt_tps(&self) -> Option<f64> {
        self.samples
            .iter()
            .filter(|sample| sample.prompt_tokens > 0 && sample.generated_tokens == 0)
            .map(|sample| sample.avg_tokens_per_second)
            .reduce(f64::max)
    }

    pub fn decode_tps(&self) -> Option<f64> {
        self.samples
            .iter()
            .filter(|sample| sample.generated_tokens > 0 && sample.prompt_tokens == 0)
            .map(|sample| sample.avg_tokens_per_second)
            .reduce(f64::max)
    }

    pub fn command_preview(&self) -> String {
        format_command(&self.bench_binary, &self.arguments)
    }
}

pub fn default_benchmark_arguments(
    installation: &LlamaInstallation,
    model: &ModelInfo,
) -> Vec<String> {
    let mut args = vec![
        "-m".into(),
        model.path.to_string_lossy().into_owned(),
        "-r".into(),
        "3".into(),
    ];

    if installation.bench_has_capability("--output") || installation.bench_has_capability("-o") {
        args.extend(["-o".into(), "json".into()]);
    }

    args
}

pub fn run_default_benchmark(
    installation: &LlamaInstallation,
    model: &ModelInfo,
) -> Result<BenchmarkRun> {
    let bench = installation
        .bench
        .as_ref()
        .ok_or(LlamaManagerError::MissingTool("llama-bench"))?;
    let bench_binary = bench.path.clone();
    let bench_sha256 = bench.sha256.clone();
    let arguments = default_benchmark_arguments(installation, model);
    let started_at_unix_ms = now_ms();

    let output = Command::new(&bench_binary).args(&arguments).output()?;
    let finished_at_unix_ms = now_ms();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if !output.status.success() {
        return Err(LlamaManagerError::ProcessFailed {
            program: bench_binary.display().to_string(),
            code: output.status.code(),
            stderr,
        });
    }

    let samples = if arguments.iter().any(|arg| arg == "json") {
        parse_json_output(&stdout)?
    } else {
        parse_markdown_output(&stdout)?
    };

    if samples.is_empty() {
        return Err(LlamaManagerError::BenchmarkParse(
            "llama-bench completed but produced no recognized benchmark rows".into(),
        ));
    }

    Ok(BenchmarkRun {
        id: Uuid::new_v4().to_string(),
        installation_id: installation.id.clone(),
        model_id: model.id.clone(),
        model_sha256: model.sha256.clone(),
        bench_binary,
        bench_sha256,
        backend: installation.backend.clone(),
        arguments,
        started_at_unix_ms,
        finished_at_unix_ms,
        exit_code: output.status.code(),
        stdout,
        stderr,
        samples,
    })
}

fn parse_json_output(raw: &str) -> Result<Vec<BenchmarkSample>> {
    let rows: Value = serde_json::from_str(raw.trim())?;
    let rows = rows.as_array().ok_or_else(|| {
        LlamaManagerError::BenchmarkParse("JSON output root is not an array".into())
    })?;

    rows.iter().map(parse_json_row).collect()
}

fn parse_json_row(row: &Value) -> Result<BenchmarkSample> {
    let avg = row
        .get("avg_ts")
        .and_then(Value::as_f64)
        .ok_or_else(|| LlamaManagerError::BenchmarkParse("JSON row is missing avg_ts".into()))?;
    let prompt_tokens = row.get("n_prompt").and_then(Value::as_u64).unwrap_or(0);
    let generated_tokens = row.get("n_gen").and_then(Value::as_u64).unwrap_or(0);
    let test = test_label(prompt_tokens, generated_tokens, row.get("n_depth").and_then(Value::as_u64).unwrap_or(0));

    let samples_tokens_per_second = row
        .get("samples_ts")
        .and_then(Value::as_array)
        .map(|samples| samples.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();

    Ok(BenchmarkSample {
        test,
        backend: row.get("backends").and_then(Value::as_str).map(str::to_string),
        model_type: row.get("model_type").and_then(Value::as_str).map(str::to_string),
        prompt_tokens,
        generated_tokens,
        avg_tokens_per_second: avg,
        stddev_tokens_per_second: row.get("stddev_ts").and_then(Value::as_f64),
        samples_tokens_per_second,
    })
}

fn parse_markdown_output(raw: &str) -> Result<Vec<BenchmarkSample>> {
    let mut rows = Vec::new();

    for line in raw.lines().filter(|line| line.contains('|')) {
        let cells: Vec<_> = line
            .split('|')
            .map(str::trim)
            .filter(|cell| !cell.is_empty())
            .collect();

        let Some(test_cell) = cells.iter().find(|cell| {
            let lower = cell.to_ascii_lowercase();
            lower.starts_with("pp") || lower.starts_with("tg") || lower.starts_with("pg")
        }) else {
            continue;
        };

        let Some(speed_cell) = cells.last() else { continue };
        let speed_token = speed_cell
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<f64>().ok());
        let Some(avg_tokens_per_second) = speed_token else { continue };

        let (prompt_tokens, generated_tokens) = parse_test_counts(test_cell);
        rows.push(BenchmarkSample {
            test: (*test_cell).to_string(),
            backend: None,
            model_type: cells.first().map(|value| (*value).to_string()),
            prompt_tokens,
            generated_tokens,
            avg_tokens_per_second,
            stddev_tokens_per_second: None,
            samples_tokens_per_second: Vec::new(),
        });
    }

    Ok(rows)
}

fn parse_test_counts(label: &str) -> (u64, u64) {
    let lower = label.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("pp") {
        return (leading_number(rest), 0);
    }
    if let Some(rest) = lower.strip_prefix("tg") {
        return (0, leading_number(rest));
    }
    if let Some(rest) = lower.strip_prefix("pg") {
        let mut numbers = rest
            .split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u64>().ok());
        return (numbers.next().unwrap_or(0), numbers.next().unwrap_or(0));
    }
    (0, 0)
}

fn leading_number(value: &str) -> u64 {
    value
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn test_label(prompt: u64, generated: u64, depth: u64) -> String {
    let base = match (prompt, generated) {
        (p, 0) if p > 0 => format!("pp{p}"),
        (0, g) if g > 0 => format!("tg{g}"),
        (p, g) => format!("pg{p},{g}"),
    };
    if depth > 0 { format!("{base} @ d{depth}") } else { base }
}

pub fn format_command(program: &std::path::Path, args: &[String]) -> String {
    std::iter::once(program.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .map(|part| quote_for_display(&part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_for_display(value: &str) -> String {
    if value.contains(char::is_whitespace) || value.contains('"') {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_llama_bench_json_shape() {
        let raw = r#"[
          {"backends":"CUDA","model_type":"qwen","n_prompt":512,"n_gen":0,"n_depth":0,"avg_ts":7100.0,"stddev_ts":140.0,"samples_ts":[7000.0,7200.0]},
          {"backends":"CUDA","model_type":"qwen","n_prompt":0,"n_gen":128,"n_depth":0,"avg_ts":120.5,"stddev_ts":0.4,"samples_ts":[120.0,121.0]}
        ]"#;
        let rows = parse_json_output(raw).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].test, "pp512");
        assert_eq!(rows[1].test, "tg128");
        assert_eq!(rows[1].avg_tokens_per_second, 120.5);
    }

    #[test]
    fn command_preview_quotes_paths() {
        let preview = format_command(
            std::path::Path::new(r"D:\llama cpp\llama-bench.exe"),
            &["-m".into(), r"F:\Models\My Model.gguf".into()],
        );
        assert!(preview.contains(r#""D:\llama cpp\llama-bench.exe""#));
        assert!(preview.contains(r#""F:\Models\My Model.gguf""#));
    }
}
