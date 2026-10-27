# CI cache policy

The desktop and Android CI workflows cancel superseded runs of the same PR.
Different PRs remain independent. Pushes to `main` run both workflows and finish
without cancellation so they can warm caches shared by subsequent PR builds.
Release workflows retain their existing lifecycle.

## Storage and reuse

- Rust dependency caches are restored on PRs and written only on `main`.
  Per-PR multi-gigabyte copies cannot be reused by other PRs and previously
  displaced shared entries near the repository cache quota.
- CI sets `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` to reduce
  Rust artifact size. Optimization and the build/test/lint commands are unchanged.
  This is a workflow-only override; local and release profile settings stay intact.
- `gradle/actions/setup-gradle` restores Gradle dependencies, wrapper distributions
  and reusable build state. The full Compose job writes from `main`, with cleanup
  after successful builds. Formatting and Java jobs only read, avoiding duplicate
  cache writers. The action's default restore behavior permits reuse between jobs.
- The pinned two-ABI GStreamer SDK is restored everywhere and saved immediately
  after successful installation on `main`, rather than waiting for APK/lint success.
  Its existing checksum validation and SDK version key are retained. The release
  workflow also uses this action and can restore the same main-branch SDK cache.

A PR cannot publish a cache into the default branch's namespace. Initial runs of
this policy may still be cold, including the first successful `main` build after
merging it. Subsequent PRs can reuse those base caches. Cache eviction and Rust,
Gradle or dependency upgrades can still require fresh work; no warm-build duration
is promised before it is measured. Existing older caches are left to normal eviction.

## Coverage

Both `arm64-v8a` and `x86_64` native libraries remain in the Compose APK build.
Compose lint, formatting, the Android Java compile, Android Rust Clippy, and all
existing desktop checks remain enabled. This change adds no emulator test or
additional ABI. Local validation uses actionlint and checks that the existing job
names and build/check commands are preserved; full builds run in GitHub Actions.

References:

- [GitHub cache scope and eviction](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching)
- [Gradle cache policy and cleanup](https://github.com/gradle/actions/blob/main/docs/setup-gradle.md)
- [Rust cache options](https://github.com/Swatinem/rust-cache)
- [Workflow concurrency](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency)
