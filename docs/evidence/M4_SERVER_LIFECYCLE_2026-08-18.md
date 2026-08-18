# Milestone 4 — managed llama-server lifecycle C5 evidence

Date: 2026-08-18

## Scope

Milestone 4 covers capability-backed `llama-server` command construction, Windows process-tree supervision, health + minimal-inference readiness, bounded stdout/stderr capture, truthful lifecycle state, managed start/stop/restart/force-kill controls, failure recovery, and real Windows lifecycle validation.

## Implementation evidence

Closed M4 child issues:

- #31 — capability-backed `llama-server` command builder
- #32 — Windows process supervisor and Job Object lifecycle
- #33 — port checks, readiness, health and minimal inference verification
- #34 — server log streaming, lifecycle state machine and actionable failures
- #35 — Server Lab lifecycle UI, command preview and recovery
- #36 — real `llama-server` lifecycle validation matrix

Key merged implementation/fix PRs:

- #112 — command builder for M4.1
- #113 — Windows Job Object process supervision for M4.2
- #115 — real readiness/minimal-inference path for M4.3
- #123 — bounded server logs and lifecycle evidence, merged as `a1700e89f913102f1fb513905cb0a5034e77c945`
- #124 — managed Server Lab lifecycle UI, merged as `6797875ea3805b4001bf7c378d0907c005daa9c9`
- #125 — classify normal llama.cpp stderr by content instead of treating stderr as an error severity, merged as `2f6d822e948b025e673f59775541b9ca5961716d`
- #126 — render visible INFO/WARN/ERR/FATAL labels from presentation severity, merged as `c25cda85c95fc96e897ecfa42997345199943d6e`

## Automated and CI evidence

The M4 implementation is covered by unit/integration/fixture tests for command construction, capability blocking, Windows process supervision, Job Object cleanup, readiness probing, real minimal inference, lifecycle/log state, bounded capture, redaction and failure handling.

PR #126 CI run `32111119407` passed:

```text
PowerShell syntax                                      PASS
cargo fmt --all -- --check                            PASS
cargo check --all-targets                             PASS
cargo test --all-targets                              PASS
cargo clippy --all-targets --all-features -D warnings PASS
cargo build --release                                 PASS
desktop process smoke                                 PASS
portable bundle assembly/upload                       PASS
```

The permanent Real Windows Runtime Validation workflow also exercised pinned upstream llama.cpp `b10472` and published GGUFs on `windows-2025`. Runtime validation run `32103995058` completed successfully on the M4 implementation path and included real server readiness + inference evidence.

## Interactive Windows lifecycle verification

The repository owner exercised the release Server Lab against:

```text
runtime: D:\llamacpp\bin\llama-server.exe
model:   D:\AI\models\andy-4.2\andy-4.2.Q6_K+qwen35-mtp.gguf
endpoint: 127.0.0.1:8080
```

Verified in the rendered UI:

- exact executable + argv preview before launch, with API-key redaction where configured;
- real start → `Ready` only after health plus a real `/completion` request returned HTTP 200;
- ownership reported `MANAGED` only for the process owned by the current LlamaWave instance;
- graceful stop returned to stopped state without leaking the listener;
- restart returned to real readiness without stale state;
- occupied port 8080 was represented as `Unknown` / unowned with an actionable collision message rather than assumed to be LlamaWave;
- external process termination was detected as `Crashed`, ownership reconciled to `NONE / UNKNOWN`, exit evidence remained visible, and retained logs survived;
- deliberately invalid GGUF bytes produced a real failed-start/crashed state with retained `llama_model_load`/GGUF errors;
- normal llama.cpp startup/inference/timing output sent over stderr is displayed as `INFO`, while real load failures are displayed as `ERR`;
- force-kill requires a separate destructive-action confirmation step;
- normal and narrow desktop layouts remained usable during lifecycle/error-state verification.

## Timeout and recovery verification

A real managed llama-server was suspended before readiness. LlamaWave did not fake success: after approximately 31.34 seconds the lifecycle became `Failed`, ownership remained `MANAGED`, logs remained retained, and the UI reported a readiness timeout during health probing.

The suspended real server still accepted Windows Ctrl+C and exited cooperatively, so a deterministic wrapper fixture was used to exercise the grace-period-expiry path. The fixture enabled Windows' inherited ignore-Ctrl+C state and launched the real `D:\llamacpp\bin\llama-server.exe` child inside the managed Job Object.

The child reached real health + `/completion` readiness. `STOP GRACEFULLY` then exercised the five-second expiry path and the UI truthfully remained `Ready` + `MANAGED` while reporting:

```text
Graceful stop timed out after 5.0s. The process is still owned and was NOT reported stopped.
```

`CONFIRM FORCE KILL` terminated the managed Job Object tree. Final verification found no managed listener remaining on port 8080 (`Port8080Listening=False`).

## Persistence, recovery and truthfulness

- Server logs are bounded in memory and on disk and remain available after stop/crash/failure.
- Redacted diagnostic command/log export paths do not expose configured API keys.
- Application restart does not claim ownership of a process it did not create; an already occupied endpoint is represented as unknown/unowned.
- Readiness requires successful minimal inference rather than process existence or an open port alone.
- Failed start, crash, readiness timeout, natural stop and explicit force-kill paths remain distinct in lifecycle/evidence handling.

## Known non-blocking limitation

Interactive runs sometimes surfaced a warning that the managed console window could not be hidden automatically because Windows console attachment returned `ERROR_INVALID_HANDLE`. This warning did not change process ownership, readiness, logging, shutdown or cleanup truth and is not used as success evidence. It is presentation polish rather than a lifecycle correctness or leak defect.

## Result

All M4 child issues #31–#36 are closed with evidence. The implementation, automated suite, strict CI, real Windows runtime, failure/recovery matrix, process-tree cleanup, persistence/reproducibility and rendered UX truthfulness satisfy the applicable G1–G10 gates.

Milestone 4 is closure-ready at **C5 — Complete**.