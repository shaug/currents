#!/usr/bin/env python3
"""Hosted partial canonical evidence collection, never a full qualification claim."""
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time


def require(condition, message):
    if not condition:
        raise ValueError(message)


def validate_collection(root, role, head, tree, result, existing):
    require(role in ["macos-minimum", "ios"], "unsupported collection role")
    require(result.returncode == 1, "partial emit must return its documented nonzero status")
    report = json.loads(result.stdout)  # Exactly one JSON document, no generic blocked fallback.
    require(report.get("schema") == "QualificationReportV1" and report.get("schema_version") == 1,
            "wrong report schema")
    require(report.get("status") == "Blocked" and report.get("gate") == "qualify native-platform", "wrong gate/status")
    inputs = report.get("inputs", {})
    require(inputs.get("command") == ["qualify", "native-platform", "--emit", role]
            and inputs.get("candidate_sha") == head and inputs.get("dirty") is False,
            "wrong command, candidate or dirty report")
    evidence = report.get("evidence", {})
    require(evidence.get("result") is None, "partial emit cannot claim an aggregate result")
    prefix = f"BLOCKED: collected {role} evidence at "
    suffix = "; complete same-candidate matrix aggregation still required. Production qualification remains Blocked."
    error = evidence.get("error")
    require(isinstance(error, str) and error.startswith(prefix) and error.endswith(suffix),
            "emit did not finish canonical entry validation")
    path = Path(error[len(prefix):-len(suffix)])
    root = root.resolve()
    require(path.is_absolute(), "artifact path must be absolute")
    try:
        relative = path.relative_to(root)
    except ValueError as error:
        raise ValueError("artifact outside checkout") from error
    require(re.fullmatch(r"target/qualification/native-[0-9]+-[0-9]+/evidence\.json", relative.as_posix()),
            "artifact outside fresh canonical directory")
    require(path.resolve() == path and path not in existing and path.is_file(),
            "artifact missing, symlinked or preexisting")
    commands = evidence.get("commands")
    require(isinstance(commands, list) and commands and all(c.get("exit") == 0 for c in commands),
            "missing or failed captured commands")
    artifact = json.loads(path.read_text())
    require(artifact.get("schema_version") == 1 and artifact.get("role") == role
            and artifact.get("directory") == relative.parent.as_posix(), "wrong artifact role/directory")
    for phase in ["before", "after"]:
        snapshot = artifact.get(phase, {})
        require(snapshot.get("head", {}).get("stdout", "").strip() == head
                and snapshot.get("tree", {}).get("stdout", "").strip() == tree
                and snapshot.get("status", {}).get("stdout") == "", "stale or dirty artifact identity")
    # The just-executed canonical emitter validates raw receipts and the entry itself;
    # do not duplicate that evaluator or execute anything read from this artifact.
    return path


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root, text=True).strip()


def verify_expected_candidate(value, head):
    require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{40}", value) and value == head,
            "missing, malformed or mismatched expected candidate SHA")


def run_collection(command, root, output, role, timeout=600, grace=2):
    stdout_path, stderr_path = output / f"{role}.stdout.json", output / f"{role}.stderr.log"
    timed_out = False
    group_gone = False
    # The session belongs solely to this invocation, never the surrounding runner
    # or shared simulator services. Files retain partial output even on timeout.
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        process = subprocess.Popen(command, cwd=root, stdout=stdout, stderr=stderr, start_new_session=True)
        try:
            process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=grace)
            except subprocess.TimeoutExpired:
                pass
            finally:
                # Always kill the remaining group, even when its leader already
                # exited: an owned descendant may ignore TERM and close its pipes.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()  # Reap our child; orphaned descendants are reaped by the OS.
                deadline = time.monotonic() + grace
                while True:
                    try:
                        os.killpg(process.pid, 0)
                    except ProcessLookupError:
                        group_gone = True
                        break
                    if time.monotonic() >= deadline:
                        break
                    time.sleep(0.01)
    (output / f"{role}.exit").write_text(("timeout" if timed_out else str(process.returncode)) + "\n")
    if timed_out:
        (output / f"{role}.timeout.json").write_text(json.dumps({
            "status": "TimedOut", "timed_out": True, "timeout_seconds": timeout,
            "returncode": process.returncode, "process_group_gone": group_gone,
            "signal": -process.returncode if process.returncode < 0 else None,
            "scope": "owned process group terminated; no collected-evidence claim",
        }, indent=2) + "\n")
        cleanup = "owned process group stopped" if group_gone else "owned group kill requested; disappearance unverified"
        raise TimeoutError(f"{role} collection timed out; {cleanup}; partial diagnostics retained")
    return subprocess.CompletedProcess(command, process.returncode, stdout_path.read_text(), stderr_path.read_text())


def main():
    require(os.environ.get("GITHUB_ACTIONS") == "true", "hosted collector only")
    root = Path(__file__).resolve().parents[1]
    head, tree = git(root, "rev-parse", "HEAD"), git(root, "rev-parse", "HEAD^{tree}")
    verify_expected_candidate(os.environ.get("CURRENTS_CANDIDATE_SHA"), head)
    require(not git(root, "status", "--porcelain", "--untracked-files=all"),
            "checkout must be the clean exact workflow candidate")
    output = root / "target/native-canonical"
    output.mkdir(parents=True, exist_ok=False)
    collected = []
    for role in ["macos-minimum", "ios"]:
        existing = {p.resolve() for p in (root / "target/qualification").glob("native-*/evidence.json")}
        result = run_collection(["cargo", "xtask", "qualify", "native-platform", "--emit", role],
                                root, output, role)
        artifact = validate_collection(root, role, head, tree, result, existing)
        require(git(root, "rev-parse", "HEAD") == head and git(root, "rev-parse", "HEAD^{tree}") == tree
                and not git(root, "status", "--porcelain", "--untracked-files=all"), "checkout changed during collection")
        collected.append(str(artifact.relative_to(root)))
    (output / "artifacts.json").write_text(json.dumps(collected, indent=2) + "\n")
    print("Collected partial evidence; full matrix and production remain Blocked.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"BLOCKED: {error}", file=sys.stderr)
        sys.exit(1)
