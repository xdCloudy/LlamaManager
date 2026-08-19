# M6 native Windows telemetry visual verification

Issue: #49

This checklist makes the remaining M6 telemetry acceptance reproducible. It does **not** convert headless CI, automated tests, or the existence of screenshots into visual acceptance. The rendered states below must be observed on an interactive Windows desktop against real provider/runtime evidence.

## Preconditions

- Use the exact source commit/release build being evaluated.
- Run the strict Rust gates required by `AGENTS.md`.
- Build `target\release\llamamanager.exe`.
- Have the selected llama.cpp router/server available on the intended endpoint.
- Use a real supported Windows hardware telemetry environment.
- Never write an API key into validation artifacts or include it in screenshots.

The preparation helper can create a session checklist and launch the release build:

```powershell
pwsh -File .\scripts\prepare-m6-telemetry-validation.ps1
```

Use `-RouterHost` / `-RouterPort` when the runtime is not on `127.0.0.1:8080`. `-SkipBuild` reuses an existing release build and `-NoLaunch` prepares evidence metadata/checklist without opening the application.

The helper records the release executable SHA-256 and a non-secret TCP reachability sanity check under `artifacts\m6-telemetry-validation`. It deliberately does not accept or persist an API key.

## A — Live passive and request-bound evidence

- [ ] attach passive monitoring to the stable user-facing router/server endpoint
- [ ] verify PASSIVE LIVE references the actual resolved runtime/child rather than a guessed port
- [ ] verify prompt/decode/runtime counters change during real inference where exported
- [ ] when an inference slot is free, run the 4-token request-bound probe
- [ ] verify prompt rate, decode rate, TTFT and request latency are real request evidence
- [ ] verify MTP generated/accepted/acceptance/mean-run fields become live only when the runtime actually exports sufficient evidence
- [ ] if MTP/cache/spec fields remain unsupported, keep them UNAVAILABLE and record the limitation rather than treating it as zero

## B — Disconnect and stale truthfulness

Begin with at least one known-good passive sample.

- [ ] stop the same router/server using its normal managed/runtime stop path
- [ ] after more than one telemetry cadence, verify the UI becomes DISCONNECTED or PASSIVE STALE
- [ ] retained prior values are explicitly labelled STALE, not left visually LIVE
- [ ] latest poll/connect failure reason remains visible enough to explain why data is stale
- [ ] request-bound evidence is not silently refreshed or relabelled live during the outage
- [ ] capture the complete telemetry state without exposing secrets

## C — Reconnect without fake continuity

- [ ] restart the runtime on the same configured endpoint
- [ ] passive monitoring automatically recovers to PASSIVE LIVE with a fresh observation timestamp/value set
- [ ] no manual ephemeral child-port entry is required after router child replacement
- [ ] pre-disconnect request-bound evidence remains stale until a new successful request probe runs
- [ ] a new successful 4-token probe establishes fresh request-bound evidence
- [ ] capture the recovered state

## D — Live-history charts at normal and narrow sizes

Let enough real CPU/GPU samples accumulate to populate LIVE HISTORY.

At a normal desktop size:

- [ ] CPU/GPU chart headings, state, source identity, line/gap rendering and legend are readable
- [ ] charts do not clip or overflow their cards
- [ ] disconnect/unavailable/error/missing gaps remain distinguishable from measured line segments

At a narrow desktop size (approximately the 650 px responsive boundary or the narrowest usable native window):

- [ ] chart cards reflow without horizontal escape
- [ ] SVG plots remain visible and readable
- [ ] labels/meta/legend remain legible
- [ ] bottom workspace navigation remains usable
- [ ] capture one normal and one narrow history state

## E — Evidence-backed alert fire and clear

Use the on-screen threshold controls and **real live provider samples**. Do not inject fabricated samples or run unsafe load solely to force an alert.

- [ ] identify the current CPU alert instance and live CPU value
- [ ] adjust trigger/clear thresholds only within the UI's valid hysteresis rules so ambient/safe real CPU can sustain a trigger
- [ ] observe the sustained-window progression and an eventual ACTIVE/FIRED transition
- [ ] FIRED history includes the rule identity, evidence identity, thresholds, timestamp, retained sample count and reason
- [ ] adjust thresholds back so the real value can remain below CLEAR
- [ ] after the debounce/sustained requirements, observe RESOLVED
- [ ] RESOLVED history remains tied to the same evidence-backed rule/identity
- [ ] capture fired and resolved states

If safe ambient conditions cannot produce the transition, leave this human gate open. Automated AlertEngine tests are not a substitute for this rendered acceptance item.

## Evidence record

When the run is actually performed, retain:

- source commit SHA
- release executable SHA-256
- Windows version/display scaling
- llama.cpp build/version identity
- model identity
- stable router/server endpoint and resolved child endpoint where applicable
- screenshots/attachments plus SHA-256 where practical
- observed live prompt/decode/TTFT/MTP support state
- disconnect/stale result
- reconnect result
- normal/narrow chart result
- alert fired/resolved result
- unsupported fields and any runtime-specific limitation

Update `docs/evidence/M6_TELEMETRY_2026-08-19.md`, `docs/WORKLOG.md`, issue #49, and the M6 completion matrix only with evidence actually observed.

## Closure rule

Issue #49 remains open until all applicable rendered/runtime acceptance items are genuinely observed and recorded. Existing automated coverage proves parser/state/retry/history/alert mechanics, but it cannot by itself claim the final human visual gates.
