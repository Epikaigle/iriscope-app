#!/usr/bin/env python3
"""Profile the real Linux camera through the application's isolated capture test.

Samples CPU/RSS from /proc; preserves captures and the full validation report.
This measures the running application, not GPU frame timings or battery power.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import threading
import time


def read_process(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    ticks = int(fields[11]) + int(fields[12])
    status = Path(f"/proc/{pid}/status").read_text()
    rss = re.search(r"^VmRSS:\s+(\d+)\s+kB$", status, re.MULTILINE)
    return ticks / os.sysconf("SC_CLK_TCK"), int(rss.group(1)) if rss else 0


def phase_summary(samples, phase):
    selected = [sample for sample in samples if sample["phase"] == phase]
    # Exclude initial setup/frequency changes consistently in both phases.
    if selected:
        first = selected[0]["elapsed_s"]
        selected = [sample for sample in selected if sample["elapsed_s"] >= first + 3]
    if not selected:
        return {"samples": 0}
    decodes = sorted(sample["decode_us"] for sample in selected if "decode_us" in sample)
    return {
        "samples": len(selected),
        "cpu_percent_mean": statistics.mean(sample["cpu_percent"] for sample in selected),
        "rss_kib_min": min(sample["rss_kib"] for sample in selected),
        "rss_kib_max": max(sample["rss_kib"] for sample in selected),
        "rss_kib_first": selected[0]["rss_kib"],
        "rss_kib_last": selected[-1]["rss_kib"],
        "decode_us_median": statistics.median(decodes) if decodes else None,
        "decode_us_p95": decodes[min(int(len(decodes) * .95), len(decodes) - 1)] if decodes else None,
        "dropped_preview_first": selected[0].get("dropped_preview"),
        "dropped_preview_last": selected[-1].get("dropped_preview"),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path, help="New empty validation directory; captures are kept")
    parser.add_argument("--seconds", type=int, default=30, help="Seconds for EACH phase (5..1800)")
    args = parser.parse_args()
    if platform.system() != "Linux":
        parser.error("CPU/RSS sampling requires Linux /proc")
    if not 5 <= args.seconds <= 1800:
        parser.error("--seconds must be between 5 and 1800")
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    if output.exists() and (not output.is_dir() or any(output.iterdir())):
        parser.error("output must be absent or an empty directory; existing files are preserved")
    output.parent.mkdir(parents=True, exist_ok=True)
    logfile = Path(str(output) + ".log")
    reportfile = Path(str(output) + ".profile.json")
    if logfile.exists() or reportfile.exists():
        parser.error("profile artifacts already exist; choose a new output directory")
    binary_sha256 = hashlib.sha256(binary.read_bytes()).hexdigest()
    state = {"phase": "startup"}
    lock = threading.Lock()
    samples = []
    started = time.monotonic()
    with logfile.open("x") as log:
        process = subprocess.Popen(
            [str(binary), "--profile-camera", str(output), str(args.seconds)],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        )

        def collect_output():
            for line in process.stdout:
                log.write(line)
                log.flush()
                with lock:
                    if line.startswith("PERF_PHASE "):
                        state["phase"] = line.split()[1]
                    elif line.startswith("BUTTON_PHOTO_READY "):
                        # Older instrumented builds did not emit capture_setup.
                        # Exclude their photo writes from the preview comparison.
                        state["phase"] = "capture_setup"
                    elif line.startswith("PERF_SAMPLE "):
                        state.update({key: int(value) for key, value in re.findall(r"(\w+)=(\d+)", line)})

        reader = threading.Thread(target=collect_output)
        reader.start()
        previous = None
        try:
            while process.poll() is None:
                now = time.monotonic()
                if now - started > args.seconds * 3 + 120:
                    raise TimeoutError("Camera validation exceeded its bounded timeout")
                try:
                    cpu, rss = read_process(process.pid)
                except (FileNotFoundError, ProcessLookupError):
                    break
                if previous is not None:
                    old_time, old_cpu = previous
                    with lock:
                        sample = dict(state)
                    sample.update(elapsed_s=now - started, cpu_percent=(cpu - old_cpu) * 100 / (now - old_time), rss_kib=rss)
                    samples.append(sample)
                previous = now, cpu
                time.sleep(1)
            returncode = process.wait()
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            reader.join(timeout=10)
            process.stdout.close()
    report = {
        "binary": str(binary), "binary_sha256": binary_sha256,
        "seconds_per_phase": args.seconds, "returncode": returncode,
        "scope": "Real camera, production UI/workers; CPU percent is relative to one core; RSS is process resident memory. GPU latency, power and long-term stability are not inferred.",
        "platform": platform.platform(),
        "phases": {phase: phase_summary(samples, phase) for phase in ("preview", "recording")},
        "samples": samples,
    }
    with reportfile.open("x") as report_output:
        json.dump(report, report_output, indent=2)
        report_output.write("\n")
    print(json.dumps({"returncode": returncode, "phases": report["phases"], "report": str(reportfile)}, indent=2))
    raise SystemExit(returncode)


if __name__ == "__main__":
    main()
