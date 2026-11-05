# Picker scrolling validation — 2026-11-05

## Outcome

PR139 now targets main after PR138 merged. Merging main produced commit8fc565dd without changing the already-tested application source. The asynchronous catalog and continuous picker-height handoff fixes remain; **no speculative scrolling optimization or diagnostic UI hook was added to the PR**.

Scrolling is not universally jank-free. The two suggestion lists had no observed app-deadline misses or dropped presentations in these captures. The animated-heavy grid retained presentation problems, while the demo grid had occasional deadline misses in one of two runs.

## Measured results

Two fresh-process captures per row; numbers show run1/run2, not averages. Counts use complete app SurfaceFrames in the active gesture envelope.

| Widget / dataset | Complete SurfaceFrames | App Deadline Missed | Dropped presentations | Largest main doFrame,ms |
|---|---:|---:|---:|---:|
| Picker grid / demo | 291 / 276 | 4 / 0 | 0 / 0 | 9.94 / 10.19 |
| Picker grid / sticker-heavy | 840 / 852 | 0 / 1 | 53 / 56 | 11.74 / 17.93 |
| Username suggestions / demo | 245 / 243 | 0 / 0 | 0 / 0 | 3.57 / 2.98 |
| Username suggestions / large | 218 / 214 | 0 / 0 | 0 / 0 | 3.41 / 3.09 |
| Sticker/emoji suggestions / demo | 215 / 217 | 0 / 0 | 0 / 0 | 2.42 / 2.25 |
| Sticker/emoji suggestions / large | 331 / 325 | 0 / 0 | 0 / 0 | 3.37 / 4.68 |

All12 traces contained actual scroll-index/offset changes. All four grid captures kept panel height fixed at1703px; resize/expand/collapse was outside the measured scrolling workload. Demo-grid indices reached165; sticker-heavy grid indices reached163/169. Large suggestion lists exposed only early rows (maximum visible index7), so deep-position behavior remains unverified.

Dropped presentations are **not** interchangeable with App Deadline Missed, nor a measured percentage of visibly skipped display refreshes. Animated drawables can produce rendering work independently of main-thread list layout. In the first heavy-grid capture, RenderThread consumed1596.7ms scheduled CPU versus1182.2ms on Main over the14.58s envelope. That identifies important rendering-side work, but does not establish a complete causal chain for the dropped presentations or prove GPU saturation. Main doFrame duration is not input-to-display latency. Inclusive parent/child slices overlap and must not be added.

## Workload and controls

- Existing API37 x86_64 phone emulator,1080×2400; non-debuggable, shell-profileable benchmark with the unchanged optimized native backend. R8 remained disabled as in the audited build. No production-packaging or physical-device claim.
- Real PapoApp, theme/Haze backdrop and production list/grid/row components. A local diagnostic-only snapshot override held data constant. Native live updates and concurrent incoming messages were not tested.
- Demo:4 members,1 custom animated sticker and264 Unicode entries. Large username fixture added512 members with256×256 PNG avatars. Large sticker fixture added512 distinct cache identities:128 real-demo GIF instances and384 generated128×128 PNGs.
- Encoded assets were seeded and byte-verified in the real disk cache before UI exposure; real Coil decoding/rendering and first-exposure/reverse-pass reuse remained. This does not cover remote downloads, huge assets, cache-miss network handling or image failures.
- Five450ms gestures at2/5/8/11/14s in each20s trace; first three forward, two shorter reverse. Semantic bounds selected the actual list, not the chat timeline. Variant order reversed for repetition2.
- Geometry and scroll telemetry ran off Main without subscribing composition to scroll pixels. Schema/quality queries preceded metrics in pinned Perfetto v58.3 warm sessions. No nonzero error/data-loss flags, duplicate token/layer groups or incomplete/right-boundary exclusions appeared in these analyzed envelopes.

## Limitations and disposition

API37 frame-present parser info warnings remain a compatibility caveat; they were not treated as proof of frame loss or conclusively decoded. The emulator's unrelated UWB-service failure, host load, frequency/thermal state, JIT/AOT history and small sample size limit broad comparisons. The initial pilot's mismatched IME policy and a pre-recording draft-guard abort were excluded; the measured fixture matched MainActivity's adjustResize policy.

The investigation used only a separate local validation package. No normal app/account was cleared, upgraded or accessed, and no real messages/calls were sent. The original staged source tree remains unchanged.

Per the requested stopping point, optional animation/deep-position controls were stopped and are not used to claim a fix. No animation-freezing policy, rendering rewrite or unmeasured content-type change was shipped. Previously validated source tests remain22 selected instrumented tests plus7 JVM tests; this update is documentation only and does not claim a new test run.
