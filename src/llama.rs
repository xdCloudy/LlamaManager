use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::error::{LlamaManagerError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolEvidence {
    pub path: PathBuf,
    pub sha256: String,
    pub version_output: String,
    pub help_output: String,
    #[serde(default)]
    pub device_output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaInstallation {
    pub id: String,
    pub name: String,
    pub root_path: PathBuf,
    pub server: Option<ToolEvidence>,
    pub bench: Option<ToolEvidence>,
    pub fit_params: Option<ToolEvidence>,
    pub backend: Option<String>,
    pub capabilities: BTreeSet<String>,
    pub discovered_at_unix_ms: u128,
}

impl LlamaInstallation {
    pub fn has_capability(&self, option: &str) -> bool {
        self.capabilities.contains(option)
    }

    pub fn bench_path(&self) -> Result<&Path> {
        self.bench
            .as_ref()
            .map(|tool| tool.path.as_path())
            .ok_or(LlamaManagerError::MissingTool("llama-bench"))
    }

    pub fn bench_has_capability(&self, option: &str) -> bool {
        self.bench
            .as_ref()
            .is_some_and(|tool| extract_cli_options(&tool.help_output).contains(option))
    }
}

pub fn inspect_installation(root: &Path) -> Result<LlamaInstallation> {
    if !root.is_dir() {
        return Err(LlamaManagerError::InvalidPath(root.to_path_buf()));
    }

    let discovered = discover_tools(root);
    let server_path = discovered.get("llama-server").cloned();
    let bench_path = discovered.get("llama-bench").cloned();
    let fit_path = discovered.get("llama-fit-params").cloned();

    if server_path.is_none() && bench_path.is_none() {
        return Err(LlamaManagerError::NoLlamaBinaries(root.to_path_buf()));
    }

    let server = server_path.map(inspect_tool).transpose()?;
    let mut bench = bench_path.map(inspect_tool).transpose()?;
    let fit_params = fit_path.map(inspect_tool).transpose()?;

    if let Some(bench_tool) = bench.as_mut() {
        let options = extract_cli_options(&bench_tool.help_output);
        if options.contains("--list-devices") {
            if let Ok(output) = run_probe(&bench_tool.path, &["--list-devices"]) {
                bench_tool.device_output = output_text(output);
            }
        }
    }

    let mut capabilities = BTreeSet::new();
    for tool in [&server, &bench, &fit_params].into_iter().flatten() {
        capabilities.extend(extract_cli_options(&tool.help_output));
    }

    let backend_evidence = [&server, &bench, &fit_params]
        .into_iter()
        .flatten()
        .flat_map(|tool| [&tool.device_output, &tool.version_output])
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    let backend = detect_backend(&backend_evidence);
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("llama.cpp")
        .to_string();

    Ok(LlamaInstallation {
        id: stable_installation_id(root),
        name,
        root_path: root.to_path_buf(),
        server,
        bench,
        fit_params,
        backend,
        capabilities,
        discovered_at_unix_ms: now_ms(),
    })
}

fn discover_tools(root: &Path) -> BTreeMap<String, PathBuf> {
    let wanted = ["llama-server", "llama-bench", "llama-fit-params"];
    let mut found = BTreeMap::new();

    for entry in WalkDir::new(root)
        .max_depth(5)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        let stem = path.file_stem().and_then(|value| value.to_str());
        let Some(stem) = stem else { continue };
        let normalized = stem.to_ascii_lowercase();
        if wanted.contains(&normalized.as_str()) {
            found
                .entry(normalized)
                .or_insert_with(|| path.to_path_buf());
        }
    }

    found
}

fn inspect_tool(path: PathBuf) -> Result<ToolEvidence> {
    let sha256 = sha256_file(&path)?;
    let version_output = run_probe(&path, &["--version"])
        .map(output_text)
        .unwrap_or_default();
    let help_output = output_text(run_probe(&path, &["--help"])?);

    Ok(ToolEvidence {
        path,
        sha256,
        version_output,
        help_output,
        device_output: String::new(),
    })
}

fn run_probe(path: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(path).args(args).output()
}

fn output_text(output: Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
        (false, false) => format!("{}\n{}", stdout.trim(), stderr.trim()),
        (false, true) => stdout.trim().to_string(),
        (true, false) => stderr.trim().to_string(),
        (true, true) => String::new(),
    }
}

fn extract_cli_options(help: &str) -> BTreeSet<String> {
    help.split_whitespace()
        .filter_map(|token| {
            let cleaned = token
                .trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '[' | ']' | '(' | ')' | '`'));
            if cleaned.starts_with("--") && cleaned.len() > 2 {
                Some(cleaned.to_string())
            } else if cleaned.starts_with('-')
                && !cleaned.starts_with("--")
                && cleaned.len() > 1
                && cleaned.chars().skip(1).all(|c| c.is_ascii_alphanumeric())
            {
                Some(cleaned.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn detect_backend(text: &str) -> Option<String> {
    for (needle, label) in [
        ("cuda", "CUDA"),
        ("vulkan", "Vulkan"),
        ("metal", "Metal"),
        ("sycl", "SYCL"),
        ("hip", "HIP"),
        ("rpc", "RPC"),
    ] {
        if text.contains(needle) {
            return Some(label.to_string());
        }
    }
    if text.contains("cpu") {
        return Some("CPU".to_string());
    }
    None
}

fn stable_installation_id(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"llamamanager:installation:");
    hasher.update(root.to_string_lossy().as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("installation-{}", &digest[..32])
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_long_and_short_options() {
        let help = "-m, --model <file>  -ngl, --n-gpu-layers <n> --future-flag";
        let options = extract_cli_options(help);
        assert!(options.contains("--model"));
        assert!(options.contains("--n-gpu-layers"));
        assert!(options.contains("--future-flag"));
        assert!(options.contains("-m"));
        assert!(options.contains("-ngl"));
    }

    #[test]
    fn detects_backend_from_real_evidence_text_only() {
        assert_eq!(detect_backend("ggml_cuda init"), Some("CUDA".into()));
        assert_eq!(detect_backend("unknown custom backend"), None);
    }
}
