#!/usr/bin/env python3
"""Orchestrator for 14c desktop budgeted Rune + papo-call playback verification."""

import argparse
import json
import math
import os
import pathlib
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request

BUDGETS = {
    "360p": {
        "tiles": 1,
        "cpu_mean_pct_max": 15.0,
        "p50_ms_max": 25.0,
        "p95_ms_max": 50.0,
        "drop_pct_max": 0.5,
        "peak_rss_mib_max": 80.0,
        "peak_vram_mib_max": 48.0,
        "growth_rss_mib_max": 8.0,
        "growth_vram_mib_max": 16.0,
        "require_audio": False,
    },
    "720p": {
        "tiles": 1,
        "cpu_mean_pct_max": 30.0,
        "p50_ms_max": 25.0,
        "p95_ms_max": 50.0,
        "drop_pct_max": 0.5,
        "peak_rss_mib_max": 120.0,
        "peak_vram_mib_max": 96.0,
        "growth_rss_mib_max": 8.0,
        "growth_vram_mib_max": 16.0,
        "require_audio": False,
    },
    "1080p": {
        "tiles": 1,
        "cpu_mean_pct_max": 50.0,
        "p50_ms_max": 25.0,
        "p95_ms_max": 50.0,
        "drop_pct_max": 0.5,
        "peak_rss_mib_max": 176.0,
        "peak_vram_mib_max": 144.0,
        "growth_rss_mib_max": 8.0,
        "growth_vram_mib_max": 16.0,
        "require_audio": False,
    },
    "four-720p": {
        "tiles": 4,
        "cpu_mean_pct_max": 120.0,
        "p50_ms_max": 25.0,
        "p95_ms_max": 50.0,
        "drop_pct_max": 0.5,
        "peak_rss_mib_max": 280.0,
        "peak_vram_mib_max": 224.0,
        "growth_rss_mib_max": 8.0,
        "growth_vram_mib_max": 16.0,
        "require_audio": True,
    },
}

CLK_TCK = os.sysconf("SC_CLK_TCK")


def parse_size_mib(raw: str) -> float:
    parts = raw.strip().split()
    val = float(parts[0])
    unit = parts[1] if len(parts) > 1 else "B"
    if unit in ("KiB", "kB"):
        return val / 1024.0
    if unit in ("MiB", "MB"):
        return val
    if unit in ("GiB", "GB"):
        return val * 1024.0
    return val / (1024.0 * 1024.0)


def sanitize_json(obj):
    if isinstance(obj, float) and not math.isfinite(obj):
        return None
    if isinstance(obj, dict):
        return {k: sanitize_json(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return [sanitize_json(v) for v in obj]
    return obj


def sample_proc(pid: int) -> dict | None:
    try:
        ts_ns = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
        stat = pathlib.Path(f"/proc/{pid}/stat").read_text()
        fields = stat[stat.rfind(")") + 2 :].split()
        cpu_s = (int(fields[11]) + int(fields[12])) / CLK_TCK

        rss_mib = 0.0
        for line in pathlib.Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                rss_mib = parse_size_mib(line.split(":", 1)[1])
                break

        clients: dict[tuple[str, str], float] = {}
        fdinfo_dir = pathlib.Path(f"/proc/{pid}/fdinfo")
        for entry in fdinfo_dir.iterdir():
            try:
                text = entry.read_text()
            except OSError:
                continue
            if "drm-memory-vram:" not in text:
                continue
            drm_dev = "drm"
            client_id = None
            vram_mib = None
            for line in text.splitlines():
                if line.startswith("drm-pdev:") or line.startswith("drm-driver:"):
                    drm_dev = line.split(":", 1)[1].strip()
                elif line.startswith("drm-client-id:"):
                    client_id = line.split(":", 1)[1].strip()
                elif line.startswith("drm-memory-vram:"):
                    vram_mib = parse_size_mib(line.split(":", 1)[1])
            if client_id is not None and vram_mib is not None:
                key = (drm_dev, client_id)
                clients[key] = max(clients.get(key, 0.0), vram_mib)

        return {
            "t_ns": ts_ns,
            "cpu_s": cpu_s,
            "rss_mib": rss_mib,
            "vram_mib": sum(clients.values()),
        }
    except OSError:
        return None


def percentile(sorted_vals: list[float], pct: float) -> float:
    if not sorted_vals:
        return math.nan
    idx = min(len(sorted_vals) - 1, max(0, math.ceil(len(sorted_vals) * pct / 100.0) - 1))
    return sorted_vals[idx]


def parse_kv_line(line: str) -> dict[str, str]:
    parsed: dict[str, str] = {}
    for token in line.split(",")[1:]:
        if "=" in token:
            k, v = token.split("=", 1)
            parsed[k] = v
    return parsed


def parse_vulkan_log(path: pathlib.Path) -> dict:
    events = []
    summaries = []
    swapchains = []
    swapchain_info = []
    for line in path.read_text().splitlines():
        if line.startswith("SWAPCHAIN,"):
            swapchains.append(line)
            swapchain_info.append(parse_kv_line(line))
        elif line.startswith("SUMMARY,"):
            summaries.append(line)
        elif line.startswith("EVENT,"):
            parts = line.split(",")
            if len(parts) < 17:
                continue
            events.append(
                {
                    "present_id": int(parts[1]),
                    "latch_seq": int(parts[2]),
                    "submit_mono_ns": int(parts[3]),
                    "tile_count": int(parts[4]),
                    "valid_mask": int(parts[5]),
                    "tile_pts": [int(parts[6]), int(parts[7]), int(parts[8]), int(parts[9])],
                    "first_pixel_out_ns": int(parts[10]),
                    "first_pixel_visible_ns": int(parts[11]),
                    "queue_end_ns": int(parts[12]),
                    "stages_mask": int(parts[13], 16),
                    "domain": parts[14],
                    "report_complete": int(parts[15]),
                    "presented": int(parts[16]),
                }
            )
    overflow = any("overflow=1" in s for s in summaries)
    return {
        "swapchains": swapchains,
        "swapchain_info": swapchain_info,
        "summaries": summaries,
        "events": events,
        "overflow": overflow,
    }


def parse_emit_log(path: pathlib.Path) -> list[tuple[int, int, int]]:
    rows = []
    for line in path.read_text().splitlines():
        if not line:
            continue
        p, rtp, ns = line.split(",")
        rows.append((int(p), int(rtp), int(ns)))
    return rows


def evaluate_run(
    profile: str,
    soak: bool,
    client_report: dict,
    samples: list[dict],
    sfu_emits: list[tuple[int, int, int]],
    vk_data: dict,
) -> dict:
    if not samples:
        raise RuntimeError("No /proc samples collected during benchmark run")

    cfg = dict(BUDGETS[profile])
    if soak:
        cfg["growth_rss_mib_max"] = 16.0
        cfg["growth_vram_mib_max"] = 32.0

    b0 = client_report["baseline_start_ns"]
    b1 = client_report["baseline_end_ns"]
    w0 = client_report["warmup_start_ns"]
    m0 = client_report["measure_start_ns"]
    m1 = client_report["measure_end_ns"]

    base_samples = [s for s in samples if b0 <= s["t_ns"] <= b1]
    if not base_samples:
        base_samples = samples[:3]
    base_rss = statistics.median(s["rss_mib"] for s in base_samples)
    base_vram = statistics.median(s["vram_mib"] for s in base_samples)

    active_samples = [s for s in samples if w0 <= s["t_ns"] <= m1]
    meas_samples = [s for s in samples if m0 <= s["t_ns"] <= m1]
    if len(meas_samples) < 2 or not active_samples:
        raise RuntimeError("Insufficient /proc samples during active/measured windows")

    s_start = min(samples, key=lambda s: abs(s["t_ns"] - m0))
    s_end = min(samples, key=lambda s: abs(s["t_ns"] - m1))
    dt_s = (s_end["t_ns"] - s_start["t_ns"]) / 1e9
    cpu_mean_pct = ((s_end["cpu_s"] - s_start["cpu_s"]) / dt_s) * 100.0 if dt_s > 0 else math.nan

    peak_delta_rss = max(s["rss_mib"] for s in active_samples) - base_rss
    peak_delta_vram = max(s["vram_mib"] for s in active_samples) - base_vram

    dur_ns = m1 - m0
    growth_rss = 0.0
    growth_vram = 0.0
    growth_checked = False
    if dur_ns >= 30_000_000_000:
        win_a = [s for s in meas_samples if m0 + 20_000_000_000 <= s["t_ns"] <= m0 + 25_000_000_000]
        win_b = [s for s in meas_samples if m1 - 5_000_000_000 <= s["t_ns"] <= m1]
        if win_a and win_b:
            growth_checked = True
            growth_rss = statistics.median(s["rss_mib"] for s in win_b) - statistics.median(
                s["rss_mib"] for s in win_a
            )
            growth_vram = statistics.median(s["vram_mib"] for s in win_b) - statistics.median(
                s["vram_mib"] for s in win_a
            )

    jitter_map: dict[tuple[int, int], tuple[int | None, int]] = {}
    for tile, _assign, rtp_ts, pts_ns, exit_ns in client_report["jitter_records"]:
        jitter_map.setdefault((tile, rtp_ts), (pts_ns, exit_ns))

    calibrated_local_domains = {
        sc.get("domain")
        for sc in vk_data["swapchain_info"]
        if sc.get("cal_ok") == "1"
        and abs(int(sc.get("cal_delta_ns", "0"))) < 1_000_000
        and sc.get("domain") in ("PRESENT_STAGE_LOCAL", "SWAPCHAIN_LOCAL")
    }
    invalid_domains: set[str] = set()
    present_map: dict[tuple[int, int], int] = {}
    for ev in vk_data["events"]:
        if ev["report_complete"] != 1 or ev["presented"] != 1:
            continue
        domain = ev["domain"]
        if domain != "CLOCK_MONOTONIC" and domain not in calibrated_local_domains:
            invalid_domains.add(domain)
            continue
        scanout_ns = ev["first_pixel_out_ns"] or ev["first_pixel_visible_ns"]
        if scanout_ns <= 0:
            continue
        for t in range(min(ev["tile_count"], 4)):
            if (ev["valid_mask"] & (1 << t)) != 0:
                key = (t, ev["tile_pts"][t])
                prev = present_map.get(key)
                if prev is None or scanout_ns < prev:
                    present_map[key] = scanout_ns

    tiles = cfg["tiles"]
    emitted_per_tile = [0] * tiles
    upstream_loss_per_tile = [0] * tiles
    drops_per_tile = [0] * tiles
    presented_per_tile = [0] * tiles
    max_consec_per_tile = [0] * tiles
    cur_consec_per_tile = [0] * tiles
    presented_times_per_tile: list[list[int]] = [[] for _ in range(tiles)]
    post_jitter_ms: list[float] = []
    whole_path_ms: list[float] = []

    for pub_idx, rtp_ts, sfu_ns in sfu_emits:
        if pub_idx < 0 or pub_idx >= tiles or sfu_ns < m0 or sfu_ns > m1:
            continue
        emitted_per_tile[pub_idx] += 1
        jit = jitter_map.get((pub_idx, rtp_ts))
        if jit is None:
            upstream_loss_per_tile[pub_idx] += 1
            cur_consec_per_tile[pub_idx] += 1
            if cur_consec_per_tile[pub_idx] > max_consec_per_tile[pub_idx]:
                max_consec_per_tile[pub_idx] = cur_consec_per_tile[pub_idx]
            continue
        pts_ns, exit_ns = jit
        disp_ns = present_map.get((pub_idx, pts_ns)) if pts_ns is not None else None
        if disp_ns is not None and disp_ns >= exit_ns and (disp_ns - exit_ns) <= 100_000_000:
            presented_per_tile[pub_idx] += 1
            cur_consec_per_tile[pub_idx] = 0
            post_jitter_ms.append((disp_ns - exit_ns) / 1e6)
            whole_path_ms.append((disp_ns - sfu_ns) / 1e6)
            presented_times_per_tile[pub_idx].append(disp_ns)
        else:
            drops_per_tile[pub_idx] += 1
            cur_consec_per_tile[pub_idx] += 1
            if cur_consec_per_tile[pub_idx] > max_consec_per_tile[pub_idx]:
                max_consec_per_tile[pub_idx] = cur_consec_per_tile[pub_idx]

    post_jitter_sorted = sorted(post_jitter_ms)
    whole_path_sorted = sorted(whole_path_ms)
    p95_gap_per_tile_ms: list[float] = []
    all_gaps_ms: list[float] = []
    for ts_list in presented_times_per_tile:
        ts_list.sort()
        tile_gaps = sorted((b - a) / 1e6 for a, b in zip(ts_list, ts_list[1:]))
        p95_gap_per_tile_ms.append(percentile(tile_gaps, 95.0))
        all_gaps_ms.extend(tile_gaps)
    all_gaps_sorted = sorted(all_gaps_ms)

    drop_pct_per_tile = [
        (drops_per_tile[i] / emitted_per_tile[i] * 100.0) if emitted_per_tile[i] > 0 else 100.0
        for i in range(tiles)
    ]
    total_loss_pct_per_tile = [
        ((drops_per_tile[i] + upstream_loss_per_tile[i]) / emitted_per_tile[i] * 100.0)
        if emitted_per_tile[i] > 0
        else 100.0
        for i in range(tiles)
    ]
    p50_ms = percentile(post_jitter_sorted, 50.0)
    p95_ms = percentile(post_jitter_sorted, 95.0)
    p95_gap_ms = percentile(all_gaps_sorted, 95.0)
    max_consec = max(max_consec_per_tile)

    audio = client_report["audio"]
    failures = []
    if not vk_data["summaries"]:
        failures.append("Missing Vulkan SUMMARY line (incomplete teardown)")
    if vk_data["overflow"]:
        failures.append("Vulkan present timing ring overflowed")
    if invalid_domains:
        failures.append(f"Uncalibrated/incompatible Vulkan time domains: {sorted(invalid_domains)}")
    if client_report["telemetry_overflowed"]:
        failures.append("Rust jitter telemetry ring overflowed")
    if client_report.get("telemetry_clock_unavailable"):
        failures.append("Rust monotonic telemetry clock unavailable")
    if not vk_data["events"] or not present_map:
        failures.append("No VK_EXT_present_timing scanout timestamps recorded")
    if base_rss <= 10.0:
        failures.append(f"Expected base_rss_mib > 10 from /proc/<pid>/status, got {base_rss}")
    if base_vram <= 0.0:
        failures.append(f"Expected base_vram_mib > 0 from /proc/<pid>/fdinfo, got {base_vram}")
    if client_report["decoder_factory"] != "vp8dec":
        failures.append(f"Expected software vp8dec, got {client_report['decoder_factory']}")
    if client_report["bgra_uploads"] != 0 or client_report["nv12_uploads"] == 0:
        failures.append(
            f"Pixel path violation: nv12={client_report['nv12_uploads']} bgra={client_report['bgra_uploads']}"
        )
    if math.isnan(cpu_mean_pct) or cpu_mean_pct > cfg["cpu_mean_pct_max"]:
        failures.append(f"CPU mean {cpu_mean_pct:.2f}% > {cfg['cpu_mean_pct_max']}%")
    if math.isnan(p50_ms) or p50_ms > cfg["p50_ms_max"]:
        failures.append(f"Post-jitter p50 {p50_ms:.2f} ms > {cfg['p50_ms_max']} ms")
    if math.isnan(p95_ms) or p95_ms > cfg["p95_ms_max"]:
        failures.append(f"Post-jitter p95 {p95_ms:.2f} ms > {cfg['p95_ms_max']} ms")
    if any(pct > cfg["drop_pct_max"] for pct in drop_pct_per_tile):
        failures.append(f"Post-jitter drop% {drop_pct_per_tile} > {cfg['drop_pct_max']}%")
    if any(pct > cfg["drop_pct_max"] for pct in total_loss_pct_per_tile):
        failures.append(f"Total loss% (post-jitter + upstream) {total_loss_pct_per_tile} > {cfg['drop_pct_max']}%")
    if max_consec > 2:
        failures.append(f"Max consecutive drops {max_consec} > 2")
    if any(math.isnan(g) or g > 50.0 for g in p95_gap_per_tile_ms):
        failures.append(f"Per-tile p95 gap {p95_gap_per_tile_ms} > 50 ms")
    if peak_delta_rss > cfg["peak_rss_mib_max"]:
        failures.append(f"Peak dRSS {peak_delta_rss:.2f} MiB > {cfg['peak_rss_mib_max']} MiB")
    if peak_delta_vram > cfg["peak_vram_mib_max"]:
        failures.append(f"Peak dVRAM {peak_delta_vram:.2f} MiB > {cfg['peak_vram_mib_max']} MiB")
    if dur_ns >= 30_000_000_000 and not growth_checked:
        failures.append("Missing samples for growth comparison window ([20s,25s] vs last 5s)")
    if growth_checked and growth_rss > cfg["growth_rss_mib_max"]:
        failures.append(f"RSS growth {growth_rss:.2f} MiB > {cfg['growth_rss_mib_max']} MiB")
    if growth_checked and growth_vram > cfg["growth_vram_mib_max"]:
        failures.append(f"VRAM growth {growth_vram:.2f} MiB > {cfg['growth_vram_mib_max']} MiB")
    if cfg["require_audio"]:
        if audio["sink_factory"] != "pulsesink":
            failures.append(f"Expected clocked pulsesink, got {audio['sink_factory']}")
        if not audio["clock_name"]:
            failures.append("Audio pipeline clock missing")
        if audio["rendered"] == 0:
            failures.append("Audio sink rendered 0 samples")
        if audio["underruns_measured"] != 0:
            failures.append(f"Audio underruns_measured={audio['underruns_measured']} != 0")

    return sanitize_json(
        {
            "profile": profile,
            "soak": soak,
            "passed": len(failures) == 0,
            "failures": failures,
            "cpu_mean_pct": round(cpu_mean_pct, 2),
            "post_jitter_p50_ms": round(p50_ms, 2),
            "post_jitter_p95_ms": round(p95_ms, 2),
            "whole_path_p50_ms_context": round(percentile(whole_path_sorted, 50.0), 2),
            "whole_path_p95_ms_context": round(percentile(whole_path_sorted, 95.0), 2),
            "p95_gap_ms": round(p95_gap_ms, 2),
            "p95_gap_per_tile_ms": [round(g, 2) for g in p95_gap_per_tile_ms],
            "emitted_per_tile": emitted_per_tile,
            "presented_per_tile": presented_per_tile,
            "drops_per_tile": drops_per_tile,
            "drop_pct_per_tile": [round(x, 3) for x in drop_pct_per_tile],
            "upstream_loss_per_tile": upstream_loss_per_tile,
            "total_loss_pct_per_tile": [round(x, 3) for x in total_loss_pct_per_tile],
            "max_consecutive_drops_per_tile": max_consec_per_tile,
            "base_rss_mib": round(base_rss, 2),
            "base_vram_mib": round(base_vram, 2),
            "peak_delta_rss_mib": round(peak_delta_rss, 2),
            "peak_delta_vram_mib": round(peak_delta_vram, 2),
            "growth_checked": growth_checked,
            "growth_rss_mib": round(growth_rss, 2),
            "growth_vram_mib": round(growth_vram, 2),
            "nv12_uploads": client_report["nv12_uploads"],
            "bgra_uploads": client_report["bgra_uploads"],
            "decoder_factory": client_report["decoder_factory"],
            "decoded_frames": client_report["decoded_frames"],
            "fence_accepted_packets": client_report["fence_accepted_packets"],
            "fence_rejected_packets": client_report["fence_rejected_packets"],
            "audio": audio,
            "vk_swapchains": vk_data["swapchains"],
            "vk_summaries": vk_data["summaries"],
        }
    )


def run_once(
    repo_root: pathlib.Path,
    profile: str,
    warmup_s: int,
    measure_s: int,
    soak: bool,
    out_dir: pathlib.Path,
    run_idx: int,
) -> dict:
    out_dir.mkdir(parents=True, exist_ok=True)
    client_bin = repo_root / "target/release/papo-call-proof"
    layer_dir = repo_root / "tests/call-sfu/present-timing-layer"
    run_sh = repo_root / "tests/call-sfu/run.sh"
    gocache = os.environ.get("GOCACHE") or str(repo_root / "target/go-cache")

    with tempfile.TemporaryDirectory(prefix=f"papo-14c-{profile}-") as tmp_raw:
        tmp = pathlib.Path(tmp_raw)
        emit_log = tmp / "sfu_emit.csv"
        sfu_log = tmp / "sfu.log"
        latch_file = tmp / "pt_latch.bin"
        vk_out = tmp / "pt_out.csv"
        client_json = tmp / "client_report.json"
        latch_file.write_bytes(b"\x00" * 128)

        sfu_env = dict(os.environ, GOCACHE=gocache)
        with sfu_log.open("w") as sfu_out:
            sfu = subprocess.Popen(
                ["bash", str(run_sh), "-profile", profile, "-emit-log", str(emit_log)],
                env=sfu_env,
                stdout=sfu_out,
                stderr=subprocess.STDOUT,
                text=True,
            )
            try:
                for _ in range(150):
                    if sfu.poll() is not None:
                        err = sfu_log.read_text(errors="replace")
                        raise RuntimeError(f"SFU exited early: {err}")
                    try:
                        with urllib.request.urlopen("http://127.0.0.1:18765/stats", timeout=1):
                            break
                    except Exception:
                        time.sleep(0.1)
                else:
                    raise RuntimeError("SFU did not become ready")

                client_env = dict(
                    os.environ,
                    PAPO_CALL_PROOF_ADDRESS="http://127.0.0.1:18765",
                    PAPO_CALL_BENCH_PROFILE=profile,
                    PAPO_CALL_BENCH_WARMUP_SECS=str(warmup_s),
                    PAPO_CALL_BENCH_MEASURE_SECS=str(measure_s),
                    PAPO_CALL_BENCH_REPORT=str(client_json),
                    PAPO_PT_LATCH_FILE=str(latch_file),
                    PAPO_PT_OUT_FILE=str(vk_out),
                    VK_ADD_LAYER_PATH=str(layer_dir),
                    VK_INSTANCE_LAYERS="VK_LAYER_PAPO_present_timing",
                )
                client_env.pop("DISPLAY", None)

                samples = []
                proc = subprocess.Popen([str(client_bin)], env=client_env)
                try:
                    deadline = time.monotonic() + warmup_s + measure_s + 40
                    while proc.poll() is None:
                        if time.monotonic() > deadline:
                            raise RuntimeError("Client benchmark timed out")
                        snap = sample_proc(proc.pid)
                        if snap is not None:
                            samples.append(snap)
                        time.sleep(0.25)
                    if proc.returncode != 0:
                        raise RuntimeError(f"Client exited with code {proc.returncode}")
                finally:
                    if proc.poll() is None:
                        proc.terminate()
                        try:
                            proc.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            proc.kill()
                            proc.wait(timeout=5)
            finally:
                sfu.terminate()
                try:
                    sfu.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    sfu.kill()
                    sfu.wait(timeout=5)

        client_report = json.loads(client_json.read_text())
        sfu_emits = parse_emit_log(emit_log)
        vk_data = parse_vulkan_log(vk_out)
        summary = evaluate_run(profile, soak, client_report, samples, sfu_emits, vk_data)
        tag = f"{profile}-{'soak' if soak else f'run{run_idx}'}"
        (out_dir / f"{tag}.json").write_text(json.dumps(summary, indent=2, allow_nan=False) + "\n")
        return summary


def main() -> int:
    repo_root = pathlib.Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser()
    parser.add_argument("--profiles", nargs="+", default=["360p", "720p", "1080p", "four-720p"])
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--warmup", type=int, default=10)
    parser.add_argument("--measure", type=int, default=60)
    parser.add_argument("--soak", action="store_true")
    parser.add_argument("--out", default=str(repo_root / "target/14c-budget-results"))
    args = parser.parse_args()

    out_dir = pathlib.Path(args.out)
    all_ok = True
    for profile in args.profiles:
        for r in range(1, args.runs + 1):
            res = run_once(repo_root, profile, args.warmup, args.measure, args.soak, out_dir, r)
            status = "PASS" if res["passed"] else "FAIL"
            print(
                f"[{status}] {profile} run {r}: cpu={res['cpu_mean_pct']}% "
                f"p50/p95={res['post_jitter_p50_ms']}/{res['post_jitter_p95_ms']}ms "
                f"gap95={res['p95_gap_ms']}ms drops%={res['drop_pct_per_tile']} "
                f"max_consec={res['max_consecutive_drops_per_tile']} "
                f"dRSS/dVRAM={res['peak_delta_rss_mib']}/{res['peak_delta_vram_mib']}MiB "
                f"growth={res['growth_rss_mib']}/{res['growth_vram_mib']}MiB "
                f"nv12/bgra={res['nv12_uploads']}/{res['bgra_uploads']} "
                f"underruns={res['audio']['underruns_measured']}",
                flush=True,
            )
            if not res["passed"]:
                all_ok = False
                for fail in res["failures"]:
                    print(f"  FAIL: {fail}", flush=True)
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
