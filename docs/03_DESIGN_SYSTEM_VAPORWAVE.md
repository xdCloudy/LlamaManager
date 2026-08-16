# Design System — Restrained Vaporwave Workstation

## 1. Design intent

LlamaManager should look like a **premium futuristic AI workstation**, not a novelty retro website.

The visual formula is:

```text
professional dark desktop system utility
+
restrained vaporwave identity
+
dense technical information architecture
```

The vaporwave identity should be unmistakable but controlled.

The interface must remain usable for long technical sessions.

## 2. Mood

Desired:

- precise
- technical
- premium
- nocturnal
- futuristic
- calm
- instrument-like
- slightly synthetic
- high information density
- visually distinctive without being distracting

Avoid:

- giant retro sunsets
- palm trees
- VHS overlays
- fake scan distortion
- heavy chromatic aberration
- excessive neon
- rainbow gradients
- large glowing text
- decorative noise
- cyberpunk clutter
- unreadable bloom
- arcade-game presentation

Think **high-end instrumentation with vaporwave DNA**.

## 3. Color system

Do not hard-code component colors individually. Use semantic design tokens.

Suggested starting palette:

```text
Background / deepest       #090A0F
Background raised          #0D0E16
Surface                    #12131D
Surface elevated           #181A27
Surface hover              #1E2030
Border subtle              #2A2D3D
Border strong              #3B3F55

Text primary               #F4F1F8
Text secondary             #B8B3C4
Text muted                 #777386

Purple                     #8B5CF6
Magenta                    #E65CAD
Cyan                       #48D7E8
Orange accent              #E98A48

Success                    #58D6A5
Warning                    #E8B35A
Danger                     #F06B79
Info                       #64B5F6
```

These are design-direction values, not immutable requirements.

### Accent hierarchy

Purple:
- primary navigation selection
- active focus
- major actions

Cyan:
- telemetry
- live/connected status
- secondary technical highlights

Magenta:
- selected experiment/tuning states
- identity accent
- sparklines/highlights

Orange:
- restrained callout
- experimental feature indicator
- unusual but non-error state

Do not use all accents at once in every component.

## 4. Gradients

Use gradients sparingly.

Good uses:

- top-level active indicator
- thin panel accent
- selected benchmark comparison
- hero/onboarding accent
- subtle chart fill

Example:

```text
purple → magenta
cyan → purple
purple → restrained orange
```

Avoid large saturated gradient backgrounds behind dense text.

## 5. Glow

Glow is an accent, not a layout primitive.

Allow:

- 1–4 px soft colored edge illumination
- focused control halo
- active navigation marker
- selected card edge
- small live-status indicator

Avoid:

- giant text shadows
- blooming whole panels
- blurred backgrounds that reduce contrast
- glow around every interactive control

## 6. CRT / grid motifs

Permitted only as extremely subtle atmospheric layers.

Examples:

- 1–2% opacity grid behind an empty state
- faint scanline texture in a dashboard header
- thin perspective-grid illustration in onboarding

Never place a visible CRT effect over text, charts, tables, logs, or code.

Respect reduced-motion and high-contrast accessibility.

## 7. Typography

Use clean modern system-friendly typography.

Recommended hierarchy:

```text
Display / page title    24–28 px, semibold
Section title           16–18 px, semibold
Card metric             20–28 px, semibold/medium
Body                    13–14 px
Dense table             12–13 px
Caption/meta            11–12 px
Monospace               12–13 px
```

Use monospaced text for:

- model paths
- CLI arguments
- hashes
- raw config
- logs
- benchmark samples

Do not use decorative vaporwave fonts in the working UI.

## 8. Layout

Primary shell:

```text
┌───────────────────────────────────────────────────────────────┐
│ Top title / breadcrumbs / contextual actions                 │
├───────────────┬───────────────────────────────────────────────┤
│ Persistent    │                                               │
│ left sidebar  │ Main workspace                                │
│               │                                               │
│               │                                               │
│               │                                               │
├───────────────┴───────────────────────────────────────────────┤
│ Optional status / activity strip                             │
└───────────────────────────────────────────────────────────────┘
```

Sidebar width should remain compact.

Main workspace should support dense dashboards and split views.

## 9. Navigation

Suggested sections:

```text
OVERVIEW
  Dashboard

MODELS
  Library
  Active Models
  Profiles

RUNTIME
  Server
  Router
  Sessions
  Context Cache

PERFORMANCE
  Monitor
  Benchmarks
  Autotune
  Experiments
  Compare

SYSTEM
  Hardware
  llama.cpp
  Storage
  Logs

CONFIGURATION
  models.ini
  Presets
  Snapshots

APP
  Settings
```

Bottom status region:

```text
llama.cpp    RUNNING
Backend      CUDA
Models       1 ACTIVE
VRAM         11.6 / 12 GB
```

## 10. Density

LlamaManager is a technical tool. It should not waste space.

Prefer:

- compact metric cards
- tight but readable tables
- property grids
- collapsible advanced sections
- side-by-side comparisons
- inline status badges
- tooltips
- drill-down panels

Avoid giant dashboard cards with one number and excessive empty space.

## 11. Core reusable components

Implement a coherent reusable component library:

- AppShell
- Sidebar
- PageHeader
- MetricCard
- StatusBadge
- Panel
- Tabs
- DataTable
- PropertyGrid
- Sparkline
- TimeSeriesChart
- Gauge
- BenchmarkComparison
- DiffView
- CommandPreview
- LogViewer
- Dialog
- Drawer
- Toast
- Tooltip
- SearchField
- SplitPane
- EmptyState
- ProgressStage
- RiskBadge
- CapabilityBadge
- EvidenceBadge

## 12. Status language

Use consistent semantic states.

Examples:

```text
READY
RUNNING
LOADING
STOPPED
STALE
INCOMPATIBLE
UNVERIFIED
BENCHMARKED
OPTIMIZED
DEGRADED
FAILED
EXPERIMENTAL
```

Badges should not rely on color alone.

## 13. Dashboard composition

Suggested first screen:

```text
┌ Active Model ────────────────────────────────────────────────┐
│ Qwen / Agents / etc.     READY             [Open] [Switch] │
└─────────────────────────────────────────────────────────────┘

┌ Decode TPS ┐ ┌ Prompt TPS ┐ ┌ TTFT ┐ ┌ VRAM ┐ ┌ Context ┐
│   45.4     │ │   312.8    │ │ 0.8s │ │ 92%  │ │ 38%     │
└────────────┘ └────────────┘ └───────┘ └───────┘ └─────────┘

┌ Performance history ───────────────────────┐
│ time series / sparklines                   │
└────────────────────────────────────────────┘

┌ Runtime ──────────────────┐ ┌ Alerts ───────────────────────┐
│ router, server, sessions  │ │ evidence-backed alerts       │
└───────────────────────────┘ └───────────────────────────────┘
```

## 14. Benchmark UI

Benchmark views should emphasize comparability.

Every result card/table should make it easy to see:

- model
- profile
- llama.cpp build
- workload
- prompt TPS
- decode TPS
- TTFT
- VRAM/RAM
- variance
- date
- confidence
- whether the result is stale

Do not hide the configuration behind the score.

## 15. Autotune UI

Autotune should feel like a controlled experiment, not a magic button.

Show:

```text
Stage 2 / 7 — Placement & Memory

Candidate 14 / 32

Current best:
Decode     45.4 t/s
Prompt    312.8 t/s
VRAM       11.3 GB

Testing:
n-cpu-moe = 22
ubatch     = 1024
KV         = q8_0

[Pause] [Stop Safely]
```

Advanced drill-down can expose:

- candidates
- eliminated configurations
- Pareto frontier
- objective weights
- measured distributions

## 16. Config diff UI

Before apply:

```diff
[Model]

-n-gpu-layers = auto
+n-gpu-layers = 99

+n-cpu-moe = 22
```

Beside each change show:

- reason
- benchmark evidence
- expected impact
- risk
- rollback point

## 17. Motion

Motion should be minimal and functional.

Allowed:

- 120–180 ms hover/focus transitions
- soft panel expansion
- progress interpolation
- chart update transitions
- route transition fade

Avoid looping decorative animation.

Respect reduced-motion.

## 18. Responsive targets

Must be usable at:

- 1280×720
- 1440×900
- 1920×1080
- 2560×1440

At lower resolutions:

- collapse secondary detail
- preserve primary controls
- use split-pane stacking
- keep navigation functional
- avoid horizontal scrolling except for intentionally wide tables/logs

## 19. Visual acceptance test

A page is not visually complete merely because it renders.

Check:

- hierarchy is obvious at a glance
- no excessive empty space
- no neon overload
- no accidental default-browser styling
- data remains readable
- all status colors have text/icon redundancy
- long paths do not destroy layout
- 1280×720 remains usable
- loading/error/empty states match the design system
- advanced controls remain discoverable without overwhelming beginners

The finished UI should look like a serious native performance workstation that happens to have a vaporwave identity, not a vaporwave demo that happens to run llama.cpp.

