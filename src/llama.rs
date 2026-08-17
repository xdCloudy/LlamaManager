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
        if options.contains("--list-devices")
            && let Ok(output) = run_probe(&bench_tool.path, &["--list-devices"])
            && output.status.success()
        {
            bench_tool.device_output = output_text(output);
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
    let mut found = BTreeMap::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        let Some(tool_name) = canonical_tool_name(path) else {
            continue;
        };
        found
            .entry(tool_name.to_string())
            .or_insert_with(|| path.to_path_buf());
    }

    found
}

fn canonical_tool_name(path: &Path) -> Option<&'static str> {
    let file_name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let normalized = if cfg!(windows) {
        file_name.strip_suffix(".exe")?
    } else {
        if file_name.contains('.') {
            return None;
        }
        file_name.as_str()
    };

    match normalized {
        "llama-server" => Some("llama-server"),
        "llama-bench" => Some("llama-bench"),
        "llama-fit-params" => Some("llama-fit-params"),
        _ => None,
    }
}

fn inspect_tool(path: PathBuf) -> Result<ToolEvidence> {
    let sha256 = sha256_file(&path)?;
    let version_output = run_probe(&path, &["--version"])
        .ok()
        .filter(|output| output.status.success())
        .map(output_text)
        .unwrap_or_default();
    let help_output = output_text(run_required_probe(&path, &["--help"])?);

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

fn run_required_probe(path: &Path, args: &[&str]) -> Result<Output> {
    let output = run_probe(path, args).map_err(|error| {
        LlamaManagerError::State(format!(
            "failed to execute {} {}: {error}",
            path.display(),
            args.join(" ")
        ))
    })?;

    if !output.status.success() {
        return Err(LlamaManagerError::ProcessFailed {
            program: path.display().to_string(),
            code: output.status.code(),
            stderr: output_text(output),
        });
    }

    Ok(output)
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
            let is_long = cleaned.starts_with("--") && cleaned.len() > 2;
            let is_short = cleaned.starts_with('-')
                && !cleaned.starts_with("--")
                && cleaned.len() > 1
                && cleaned.chars().skip(1).all(|c| c.is_ascii_alphanumeric());

            (is_long || is_short).then(|| cleaned.to_string())
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
    use std::{fs, process};

    fn tool_filename(tool: &str) -> String {
        if cfg!(windows) {
            format!("{tool}.exe")
        } else {
            tool.to_string()
        }
    }

    fn fixture_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "llamamanager-{name}-{}-{}",
            process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

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

    #[test]
    fn rejects_build_sidecars_as_tool_candidates() {
        assert_eq!(canonical_tool_name(Path::new("llama-server.pdb")), None);
        assert_eq!(canonical_tool_name(Path::new("llama-server.lib")), None);
        assert_eq!(canonical_tool_name(Path::new("llama-bench.exp")), None);
        assert_eq!(canonical_tool_name(Path::new("llama-fit-params.obj")), None);

        let executable = tool_filename("llama-server");
        assert_eq!(
            canonical_tool_name(Path::new(&executable)),
            Some("llama-server")
        );
    }

    #[test]
    fn discovery_is_recursive_deterministic_and_ignores_sidecars() {
        let root = fixture_root("tool-discovery");
        let preferred_dir = root.join("a-build").join("bin").join("Release");
        let deep_dir = root
            .join("z-package")
            .join("one")
            .join("two")
            .join("three")
            .join("four")
            .join("five")
            .join("six");
        fs::create_dir_all(&preferred_dir).unwrap();
        fs::create_dir_all(&deep_dir).unwrap();

        let preferred_server = preferred_dir.join(tool_filename("llama-server"));
        let duplicate_server = deep_dir.join(tool_filename("llama-server"));
        let deep_bench = deep_dir.join(tool_filename("llama-bench"));
        fs::write(&preferred_server, b"fixture").unwrap();
        fs::write(&duplicate_server, b"fixture").unwrap();
        fs::write(&deep_bench, b"fixture").unwrap();
        fs::write(preferred_dir.join("llama-server.pdb"), b"sidecar").unwrap();
        fs::write(preferred_dir.join("llama-bench.lib"), b"sidecar").unwrap();

        let discovered = discover_tools(&root);
        assert_eq!(
            discovered.get("llama-server").map(PathBuf::as_path),
            Some(preferred_server.as_path())
        );
        assert_eq!(
            discovered.get("llama-bench").map(PathBuf::as_path),
            Some(deep_bench.as_path())
        );
        assert!(!discovered.values().any(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("pdb" | "lib")
            )
        }));

        fs::remove_dir_all(root).unwrap();
    }
}
