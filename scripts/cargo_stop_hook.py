"""Queue Cargo checks and report their exit status from the Codex Stop hook."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
QUEUE = ROOT / "target" / "cargo-stop"
ERROR_TAIL_BYTES = 2048
# A single Rust crate can compile for over ten minutes without writing output.
IDLE_SECONDS = 1200
STEP_SECONDS = 1500
TEST_SECONDS = 3900
CHECKS = {
    "fmt": ["fmt", "--check"],
    "check": ["check", "--workspace", "-j", "1"],
    "build": ["build", "--workspace", "-j", "1"],
    "clippy": ["clippy", "--workspace", "--all-targets", "-j", "1", "--", "-D", "warnings"],
    "test": ["test", "--workspace", "-j", "1"],
}


def run_check(check, output, log):
    step_seconds = TEST_SECONDS if check == "test" else STEP_SECONDS
    with subprocess.Popen(
        ["cargo", *CHECKS[check]], cwd=ROOT,
        stdout=output, stderr=subprocess.STDOUT,
    ) as process:
        start = last_output = time.monotonic()
        size = log.stat().st_size
        while True:
            try:
                return process.wait(timeout=1), None
            except subprocess.TimeoutExpired:
                pass
            now = time.monotonic()
            current_size = log.stat().st_size
            if current_size != size:
                size = current_size
                last_output = now
            if now - start >= step_seconds:
                reason = f"step exceeded {step_seconds} seconds"
            elif check != "test" and now - last_output >= IDLE_SECONDS:
                reason = f"no output for {IDLE_SECONDS} seconds"
            else:
                continue
            try:
                subprocess.run(
                    ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                    stdout=output, stderr=subprocess.STDOUT, timeout=10, check=True,
                )
            except (OSError, subprocess.SubprocessError) as error:
                process.kill()
                reason += f"; process tree cleanup failed: {error}"
            return process.wait(), reason


def main():
    if len(sys.argv) > 1:
        checks = sys.argv[1:]
        if len(checks) > 3 or any(check not in CHECKS for check in checks):
            raise SystemExit(f"Queue up to three Cargo steps: {', '.join(CHECKS)}")
        QUEUE.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(dir=QUEUE, suffix=".tmp", delete=False, mode="w") as job:
            json.dump(checks, job)
        Path(job.name).rename(Path(job.name).with_suffix(".pending"))
        return 0

    event = json.loads(sys.stdin.read(64001) or "{}")
    if event.get("stop_hook_active"):
        return 0
    for pending in sorted(QUEUE.glob("*.pending")):
        running = pending.with_suffix(".running")
        try:
            pending.rename(running)
        except FileNotFoundError:
            continue
        log = running.with_suffix(".log")
        result = "completed"
        failed = False
        try:
            with log.open("wb") as output:
                for check in json.loads(running.read_text()):
                    code, timeout = run_check(check, output, log)
                    result = f"{check}: {timeout}" if timeout else f"{check} exited {code}"
                    if code or timeout:
                        failed = True
                        break
        except (OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
            result = str(error)
            failed = True
        running.rename(running.with_suffix(".done"))
        print(f"Cargo checks finished: {result}. Log: {log}. Do not resume a paused goal.", file=sys.stderr)
        if failed:
            try:
                with log.open("rb") as output:
                    size = output.seek(0, 2)
                    output.seek(max(0, size - ERROR_TAIL_BYTES))
                    tail = output.read(ERROR_TAIL_BYTES).decode("utf-8", errors="replace").strip()
                if tail:
                    print(f"Cargo log tail:\n{tail}", file=sys.stderr)
            except OSError:
                pass
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
