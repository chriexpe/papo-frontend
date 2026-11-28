//! Process-wide glibc allocator configuration for the Linux desktop client.
//!
//! By default glibc creates up to `8 * cores` secondary 64 MiB malloc arenas and
//! raises both `mmap_threshold` (up to 32 MiB) and `trim_threshold` (up to
//! 64 MiB) dynamically after large allocations are freed. Once raised, transient
//! media buffers (decoded images, GIF frames, GStreamer/libav video frames) stop
//! returning to the OS via `munmap` and accumulate across dozens of secondary
//! arenas whose top chunks `malloc_trim(0)` does not shrink.
//!
//! This module locks glibc's `mmap_threshold` and `trim_threshold` to their
//! 128 KiB initial values (disabling dynamic escalation) and bounds the arena
//! count before background threads start, unless the operator already configured
//! the corresponding parameter via `GLIBC_TUNABLES` or `MALLOC_*`.
//! Post-pipeline trimming of smaller fragmented allocations stays in
//! `playback::memory`; non-glibc platforms are a no-op.
//!
//! Safety: glibc marks `mallopt` as `MT-Unsafe init const:mallopt` because it
//! mutates global `mp_` parameters while secondary arenas may read them without
//! taking `main_arena.mutex`. [`configure`] is guarded by [`std::sync::Once`]
//! and called once at the start of [`crate::run`] before any worker threads are
//! spawned; `mallopt` takes two plain `c_int` values and dereferences no
//! pointers.

#![cfg_attr(
    all(target_os = "linux", target_env = "gnu"),
    expect(
        unsafe_code,
        reason = "glibc's mallopt takes plain integer tuning parameters and is called once before threads spawn"
    )
)]

#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
use std::ffi::OsString;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::ffi::c_int;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::sync::Once;

/// Maximum number of glibc malloc arenas (main arena plus up to three secondary
/// heaps), matching the desktop's concurrent decoder/SDK worker pools while
/// preventing `8 * cores` arena multiplication on high-core-count machines.
#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
const MAX_ARENAS: i32 = 4;

/// Threshold above which glibc serves allocations via direct `mmap`/`munmap`
/// instead of arena heaps (glibc's initial default). Setting it explicitly via
/// `mallopt` disables dynamic threshold escalation up to 32 MiB.
#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
const MMAP_THRESHOLD_BYTES: i32 = 128 * 1024;

/// Threshold above which glibc's `heap_trim`/`systrim` returns free top-of-heap
/// memory to the OS (glibc's initial default). Setting it explicitly prevents
/// dynamic escalation up to 64 MiB on secondary arenas.
#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
const TRIM_THRESHOLD_BYTES: i32 = 128 * 1024;

/// glibc `<malloc.h>` `M_TRIM_THRESHOLD`.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const M_TRIM_THRESHOLD: c_int = -1;

/// glibc `<malloc.h>` `M_MMAP_THRESHOLD`.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const M_MMAP_THRESHOLD: c_int = -3;

/// glibc `<malloc.h>` `M_ARENA_MAX`.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const M_ARENA_MAX: c_int = -8;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    /// glibc: sets an allocator tuning parameter; returns 1 on success, 0 on
    /// failure.
    fn mallopt(param: c_int, value: c_int) -> c_int;
}

/// Which glibc tuning parameters startup should apply, given the process
/// environment.
#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Plan {
    arena_max: Option<i32>,
    mmap_threshold_bytes: Option<i32>,
    trim_threshold_bytes: Option<i32>,
}

/// Decides which `mallopt` settings to apply without overriding explicit
/// operator configuration (`GLIBC_TUNABLES` entries or `MALLOC_*` variables).
#[cfg(any(all(target_os = "linux", target_env = "gnu"), test))]
fn plan_from_env<F>(mut get_var: F) -> Plan
where
    F: FnMut(&str) -> Option<OsString>,
{
    let is_set = |value: Option<OsString>| value.is_some_and(|v| !v.is_empty());
    let tunables = get_var("GLIBC_TUNABLES")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let has_tunable = |target: &str| {
        tunables.split(':').any(|entry| {
            let name = entry.split_once('=').map_or(entry, |(name, _)| name);
            name == target
        })
    };
    let arena_overridden =
        is_set(get_var("MALLOC_ARENA_MAX")) || has_tunable("glibc.malloc.arena_max");
    let mmap_overridden =
        is_set(get_var("MALLOC_MMAP_THRESHOLD_")) || has_tunable("glibc.malloc.mmap_threshold");
    let trim_overridden =
        is_set(get_var("MALLOC_TRIM_THRESHOLD_")) || has_tunable("glibc.malloc.trim_threshold");
    Plan {
        arena_max: (!arena_overridden).then_some(MAX_ARENAS),
        mmap_threshold_bytes: (!mmap_overridden).then_some(MMAP_THRESHOLD_BYTES),
        trim_threshold_bytes: (!trim_overridden).then_some(TRIM_THRESHOLD_BYTES),
    }
}

/// Configures the process allocator on Linux glibc once, before background
/// threads start.
///
/// Must be called from [`crate::run`] before any worker threads are spawned
/// because glibc documents `mallopt` as `MT-Unsafe init const:mallopt`.
/// Subsequent calls are ignored. No-op on non-glibc targets.
pub(crate) fn configure() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        static CONFIGURED: Once = Once::new();
        CONFIGURED.call_once(|| {
            let plan = plan_from_env(|name| std::env::var_os(name));
            apply(plan);
        });
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn apply(plan: Plan) {
    if let Some(arenas) = plan.arena_max {
        // SAFETY: `configure` runs once via `Once` at the start of `crate::run` before
        // background threads spawn (satisfying glibc's `MT-Unsafe init const:mallopt`),
        // and `mallopt` takes plain `c_int` values and no pointers.
        unsafe {
            let _ = mallopt(M_ARENA_MAX, arenas);
        }
    }
    if let Some(mmap_bytes) = plan.mmap_threshold_bytes {
        // SAFETY: same single-threaded startup precondition and plain `c_int` arguments.
        unsafe {
            let _ = mallopt(M_MMAP_THRESHOLD, mmap_bytes);
        }
    }
    if let Some(trim_bytes) = plan.trim_threshold_bytes {
        // SAFETY: same single-threaded startup precondition and plain `c_int` arguments.
        unsafe {
            let _ = mallopt(M_TRIM_THRESHOLD, trim_bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Environment override rules for glibc allocator configuration.

    use std::collections::HashMap;
    use std::ffi::OsString;

    use super::{MAX_ARENAS, MMAP_THRESHOLD_BYTES, Plan, TRIM_THRESHOLD_BYTES, plan_from_env};

    fn plan_with(vars: &[(&str, &str)]) -> Plan {
        let map: HashMap<&str, OsString> = vars
            .iter()
            .map(|(key, value)| (*key, OsString::from(value)))
            .collect();
        plan_from_env(|name| map.get(name).cloned())
    }

    #[test]
    fn default_environment_applies_all_glibc_limits() {
        assert_eq!(
            plan_with(&[]),
            Plan {
                arena_max: Some(MAX_ARENAS),
                mmap_threshold_bytes: Some(MMAP_THRESHOLD_BYTES),
                trim_threshold_bytes: Some(TRIM_THRESHOLD_BYTES),
            }
        );
    }

    #[test]
    fn empty_env_values_do_not_suppress_defaults() {
        assert_eq!(
            plan_with(&[
                ("GLIBC_TUNABLES", ""),
                ("MALLOC_ARENA_MAX", ""),
                ("MALLOC_MMAP_THRESHOLD_", ""),
                ("MALLOC_TRIM_THRESHOLD_", ""),
            ]),
            Plan {
                arena_max: Some(MAX_ARENAS),
                mmap_threshold_bytes: Some(MMAP_THRESHOLD_BYTES),
                trim_threshold_bytes: Some(TRIM_THRESHOLD_BYTES),
            }
        );
    }

    #[test]
    fn unrelated_glibc_tunables_do_not_suppress_malloc_limits() {
        assert_eq!(
            plan_with(&[(
                "GLIBC_TUNABLES",
                "glibc.pthread.rseq=0:glibc.cpu.hwcaps=-AVX512F",
            )]),
            Plan {
                arena_max: Some(MAX_ARENAS),
                mmap_threshold_bytes: Some(MMAP_THRESHOLD_BYTES),
                trim_threshold_bytes: Some(TRIM_THRESHOLD_BYTES),
            }
        );
    }

    #[test]
    fn matching_glibc_tunables_override_only_their_parameter() {
        assert_eq!(
            plan_with(&[(
                "GLIBC_TUNABLES",
                "glibc.pthread.rseq=0:glibc.malloc.arena_max=2:glibc.malloc.trim_threshold=65536",
            )]),
            Plan {
                arena_max: None,
                mmap_threshold_bytes: Some(MMAP_THRESHOLD_BYTES),
                trim_threshold_bytes: None,
            }
        );
        assert_eq!(
            plan_with(&[("GLIBC_TUNABLES", "glibc.malloc.mmap_threshold=262144")]),
            Plan {
                arena_max: Some(MAX_ARENAS),
                mmap_threshold_bytes: None,
                trim_threshold_bytes: Some(TRIM_THRESHOLD_BYTES),
            }
        );
    }

    #[test]
    fn individual_malloc_env_vars_override_only_their_parameter() {
        assert_eq!(
            plan_with(&[("MALLOC_ARENA_MAX", "8")]),
            Plan {
                arena_max: None,
                mmap_threshold_bytes: Some(MMAP_THRESHOLD_BYTES),
                trim_threshold_bytes: Some(TRIM_THRESHOLD_BYTES),
            }
        );
        assert_eq!(
            plan_with(&[("MALLOC_MMAP_THRESHOLD_", "262144")]),
            Plan {
                arena_max: Some(MAX_ARENAS),
                mmap_threshold_bytes: None,
                trim_threshold_bytes: Some(TRIM_THRESHOLD_BYTES),
            }
        );
        assert_eq!(
            plan_with(&[("MALLOC_TRIM_THRESHOLD_", "65536")]),
            Plan {
                arena_max: Some(MAX_ARENAS),
                mmap_threshold_bytes: Some(MMAP_THRESHOLD_BYTES),
                trim_threshold_bytes: None,
            }
        );
    }
}
