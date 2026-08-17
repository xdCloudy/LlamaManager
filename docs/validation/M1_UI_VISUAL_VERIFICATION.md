# M1 native Windows UI visual verification

Issue: #12

This checklist exists to make M1.1 evidence reproducible. It does **not** convert automated capture into visual acceptance: a human must inspect the real rendered application on an interactive Windows desktop.

## Preconditions

- Use a clean checkout/branch containing the exact commit being evaluated.
- Run the strict Rust gates required by `AGENTS.md`.
- Build the release executable with `cargo build --release`.
- Use a physical or VM Windows desktop with a primary working area of at least 1600×900.
- Close or move windows that could cover the LlamaWave window during capture.

## Capture

From the repository root in PowerShell:

```powershell
pwsh -File .\scripts\capture-m1-ui.ps1
```

The harness launches `target\release\llamamanager.exe`, waits for the native window, captures the complete window at 1280×720 and 1600×900, and writes a manifest containing the executable SHA-256 and capture metadata.

Expected output:

```text
artifacts/m1-ui/llamawave-1280x720.png
artifacts/m1-ui/llamawave-1600x900.png
artifacts/m1-ui/manifest.json
```

Do not treat the PNG files merely existing as a pass. Screen capture requires an interactive desktop and the window must be unobscured.

## Manual inspection checklist

Inspect both screenshots and the live application. Record pass/fail plus a short observation for every item.

- [ ] release build launches interactively without a developer terminal being required for normal use
- [ ] complete application window is visible in the 1280×720 capture
- [ ] complete application window is visible in the 1600×900 capture
- [ ] primary navigation and current page title remain readable at both sizes
- [ ] no controls, status text, badges, tables, command/evidence areas, or action rows are clipped unexpectedly
- [ ] scrollable regions scroll rather than pushing controls off-screen
- [ ] spacing and hierarchy remain coherent rather than collapsing at the narrow size
- [ ] body text, muted text, control labels, and status text remain legible against their backgrounds
- [ ] active, disabled, error, success, loading/busy, and unavailable states do not masquerade as each other
- [ ] no knowingly dead control is presented as operational
- [ ] no placeholder/fabricated metric or runtime state is presented as real evidence
- [ ] restrained vaporwave styling remains consistent with `docs/03_DESIGN_SYSTEM_VAPORWAVE.md`
- [ ] resizing between the two target sizes does not leave stale layout, detached overlays, or unusable controls

## Evidence record

Record the following in `docs/WORKLOG.md` when the run is actually performed:

- source commit SHA
- release executable SHA-256 from `manifest.json`
- Windows version and display scaling
- screenshot paths or durable issue/PR attachment references
- 1280×720 inspection result
- 1600×900 inspection result
- defects found and the fixing commit/PR, if any
- re-verification result after every visual fix

If a defect is found, keep #12 open, fix only the demonstrated defect, rerun strict CI, capture new screenshots from the fixed release build, and inspect them again.

## Closure rule

#12 must remain open until the real rendered release UI has been captured and visually inspected at both target sizes and any discovered defect has been fixed and re-verified. Source review, headless CI, generated mocks, or screenshot-file existence alone are not sufficient evidence.
