import copy
import importlib.util
import json
import os
import signal
import sys
import time
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("native_canonical", Path(__file__).with_name("native-canonical.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class CanonicalTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.path = self.root / "target/qualification/native-123-456/evidence.json"
        self.path.parent.mkdir(parents=True)
        self.head, self.tree = "a" * 40, "b" * 40
        self.artifact = {"schema_version": 1, "role": "ios", "directory": "target/qualification/native-123-456"}
        for phase in ["before", "after"]:
            self.artifact[phase] = {"head": {"stdout": self.head}, "tree": {"stdout": self.tree}, "status": {"stdout": ""}}
        self.write_artifact()
        self.report = {"schema": "QualificationReportV1", "schema_version": 1, "gate": "qualify native-platform", "status": "Blocked",
            "inputs": {"command": ["qualify", "native-platform", "--emit", "ios"], "candidate_sha": self.head, "dirty": False},
            "evidence": {"result": None, "error": self.sentinel(self.path), "commands": [{"exit": 0}]}}

    def sentinel(self, path):
        return f"BLOCKED: collected ios evidence at {path}; complete same-candidate matrix aggregation still required. Production qualification remains Blocked."

    def write_artifact(self):
        self.path.write_text(json.dumps(self.artifact))

    def validate(self, report=None, code=1, existing=()):
        report = report or self.report
        result = subprocess.CompletedProcess([], code, json.dumps(report), "")
        return module.validate_collection(self.root, "ios", self.head, self.tree, result, set(existing))

    def test_timeout_stops_owned_descendant_and_retains_partial_diagnostics(self):
        output = self.root / "timeout-logs"
        output.mkdir()
        heartbeat = self.root / "heartbeat"
        pidfile = self.root / "child.pid"
        child = self.root / "child.py"
        child.write_text("import os, signal, time\nfrom pathlib import Path\nsignal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                         + f"Path({str(pidfile)!r}).write_text(str(os.getpid()))\n"
                         + f"p = Path({str(heartbeat)!r})\n"
                         + "for i in range(3000):\n p.write_text(str(i)); time.sleep(0.01)\n")
        parent = self.root / "parent.py"
        parent.write_text("import subprocess, sys, time\nfrom pathlib import Path\n"
                          + f"subprocess.Popen([sys.executable, {str(child)!r}])\n"
                          + "print('partial stdout', flush=True)\nprint('partial stderr', file=sys.stderr, flush=True)\ntime.sleep(30)\n")
        try:
            with self.assertRaises(TimeoutError):
                module.run_collection([sys.executable, str(parent)], self.root, output, "ios", timeout=0.4, grace=1)
            self.assertTrue(pidfile.exists(), "descendant must actually have launched")
            before = heartbeat.read_text()
            time.sleep(0.15)
            self.assertEqual(heartbeat.read_text(), before, "descendant continued changing artifacts")
            self.assertIn("partial stdout", (output / "ios.stdout.json").read_text())
            self.assertIn("partial stderr", (output / "ios.stderr.log").read_text())
            self.assertEqual((output / "ios.exit").read_text(), "timeout\n")
            receipt = json.loads((output / "ios.timeout.json").read_text())
            self.assertEqual(receipt["status"], "TimedOut")
            self.assertIs(receipt["timed_out"], True)
            self.assertIs(receipt["process_group_gone"], True)
            self.assertEqual(receipt["returncode"], -signal.SIGTERM)
            self.assertEqual(receipt["signal"], signal.SIGTERM)
        finally:
            if pidfile.exists():
                try: os.kill(int(pidfile.read_text()), signal.SIGKILL)
                except ProcessLookupError: pass

    def test_group_with_unreaped_zombie_is_not_gone_until_reaped(self):
        # macOS answers killpg(pgid, 0) with EPERM, not ESRCH, while a group
        # holds only zombies; the probe must keep waiting instead of raising.
        process = subprocess.Popen([sys.executable, "-c", "pass"], start_new_session=True)
        self.addCleanup(process.wait)
        os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT)  # Exited, left unreaped.
        self.assertIs(module.group_gone_within(process.pid, 0.05), False)
        process.wait()
        self.assertIs(module.group_gone_within(process.pid, 0.05), True)

    def test_expected_candidate_is_explicit_immutable_sha(self):
        module.verify_expected_candidate(self.head, self.head)
        for value in [None, "", "main", "c" * 40, "a" * 39, "z" * 40]:
            with self.assertRaises(ValueError): module.verify_expected_candidate(value, self.head)

    def test_accepts_only_fresh_successfully_collected_partial_artifact(self):
        self.assertEqual(self.validate(), self.path)

    def test_rejects_generic_blocked_success_and_failed_command(self):
        for error in ["BLOCKED: missing prerequisite", "failed command", self.sentinel(self.path) + " extra"]:
            report = copy.deepcopy(self.report)
            report["evidence"]["error"] = error
            with self.assertRaises(ValueError): self.validate(report)
        for code in [0, 2, -9]:
            with self.assertRaises(ValueError): self.validate(code=code)
        report = copy.deepcopy(self.report)
        report["evidence"]["commands"][0]["exit"] = 1
        with self.assertRaises(ValueError): self.validate(report)

    def test_rejects_malformed_report_and_symlink_escape(self):
        for stdout in ["{}", json.dumps(self.report) + " {}", "not JSON"]:
            result = subprocess.CompletedProcess([], 1, stdout, "")
            with self.assertRaises(ValueError):
                module.validate_collection(self.root, "ios", self.head, self.tree, result, set())
        external = self.root / "outside.json"
        external.write_text(self.path.read_text())
        self.path.unlink()
        self.path.symlink_to(external)
        with self.assertRaises(ValueError): self.validate()

    def test_rejects_stale_dirty_or_wrong_command_report(self):
        for field, value in [("candidate_sha", "c" * 40), ("dirty", True), ("command", ["qualify", "native-platform", "--emit", "macos-minimum"])]:
            report = copy.deepcopy(self.report)
            report["inputs"][field] = value
            with self.assertRaises(ValueError): self.validate(report)

    def test_rejects_missing_preexisting_or_escaped_artifact(self):
        with self.assertRaises(ValueError): self.validate(existing=[self.path.resolve()])
        report = copy.deepcopy(self.report)
        report["evidence"]["error"] = self.sentinel(self.root / "outside.json")
        with self.assertRaises(ValueError): self.validate(report)
        self.path.unlink()
        with self.assertRaises(ValueError): self.validate()

    def test_rejects_wrong_role_or_stale_artifact_identity(self):
        for phase, field, value in [("before", "head", "c" * 40), ("after", "tree", "d" * 40), ("after", "status", " M x")]:
            original = copy.deepcopy(self.artifact)
            self.artifact[phase][field]["stdout"] = value
            self.write_artifact()
            with self.assertRaises(ValueError): self.validate()
            self.artifact = original
        self.artifact["role"] = "macos-minimum"
        self.write_artifact()
        with self.assertRaises(ValueError): self.validate()

if __name__ == "__main__":
    unittest.main()
