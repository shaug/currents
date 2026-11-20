"""Fail-closed composition of exact-checkout PR workflow gates."""
import json
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
GATES = ("core", "quality-policy", "msrv", "docs", "privacy")
SHA_CHECK = 'test "$(git rev-parse HEAD)" = "$GITHUB_SHA"'
PR = "${{ github.event_name == 'pull_request' }}"
NON_PR = "${{ github.event_name != 'pull_request' }}"


class WorkflowContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = json.loads((ROOT / ".github/workflows/ci.yml").read_text())
        cls.jobs = cls.workflow["jobs"]

    def test_parallel_gates_and_exact_checkouts(self):
        self.assertEqual(self.workflow["permissions"], {"contents": "read"})
        self.assertEqual(set(self.workflow["on"]), {"pull_request", "push", "workflow_dispatch"})
        for name in (*GATES, "quality"):
            with self.subTest(job=name):
                job = self.jobs[name]
                if name != "quality":
                    self.assertNotIn("needs", job)
                    self.assertNotIn("if", job)
                self.assertNotIn("continue-on-error", job)
                checkout, exact, *_ = job["steps"]
                self.assertRegex(checkout["uses"], r"^actions/checkout@[a-f0-9]{40}$")
                self.assertEqual(checkout["with"], {"persist-credentials": False})
                self.assertEqual(exact, {"name": "Verify exact checkout", "run": SHA_CHECK})

    def test_platforms_and_gate_commands(self):
        core = self.jobs["core"]
        self.assertEqual(core["strategy"]["matrix"], {"os": ["ubuntu-24.04", "macos-15"]})
        self.assertEqual(core["runs-on"], "${{ matrix.os }}")
        for gate in ("core", "msrv", "docs", "privacy"):
            step = {"name": f"Run {gate} gate", "run": f"cargo xtask check {gate}"}
            if gate in ("core", "msrv"):
                step["env"] = {"CURRENTS_CI_CORE_PHASES": "1"}
                self.assertEqual(self.jobs[gate]["timeout-minutes"], 85 if gate == "msrv" else 65)
            else:
                self.assertEqual(self.jobs[gate]["timeout-minutes"], 40)
            self.assertEqual(self.jobs[gate]["steps"][-1], step)
        quality_policy = self.jobs["quality-policy"]
        self.assertEqual(quality_policy["timeout-minutes"],
                         "${{ github.event_name == 'pull_request' && 40 || 75 }}")
        tests, pr, other, policy = quality_policy["steps"][-4:]
        self.assertEqual(tests, {
            "name": "Test PR quality orchestration",
            "if": PR,
            "run": "PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools -p test_pr_quality.py",
        })
        self.assertEqual(pr, {"name": "Run PR quality checks", "if": PR,
                              "run": "cargo xtask check quality-ci"})
        self.assertEqual(other, {"name": "Run full quality gate", "if": NON_PR,
                                 "run": "cargo xtask check quality",
                                 "env": {"CURRENTS_CI_CORE_PHASES": "1"}})
        self.assertEqual(policy, {"name": "Qualify policy counterexamples",
                                  "run": "cargo xtask ticket CUR-003"})

    def test_reporter_requires_every_result_to_be_explicit_success(self):
        job = self.jobs["quality"]
        self.assertEqual(job["name"], "quality")
        self.assertEqual(set(job.get("needs", [])), set(GATES))
        self.assertEqual(job["if"], "${{ !cancelled() }}")
        guard = job["steps"][-1]
        variables = {gate.upper().replace("-", "_") + "_RESULT": gate for gate in GATES}
        self.assertEqual(guard["env"], {
            key: "${{ needs." + gate + ".result }}" for key, gate in variables.items()
        })
        self.assertNotIn("if", guard)
        self.assertNotIn("continue-on-error", guard)

        def gate_passed(results):
            env = {key: value for key, value in os.environ.items() if key not in variables}
            env.update(results)
            return subprocess.run(["bash", "-e", "-c", guard["run"]], env=env,
                                  capture_output=True, check=False).returncode == 0

        good = dict.fromkeys(variables, "success")
        self.assertTrue(gate_passed(good))
        for key in good:
            for bad in ("failure", "cancelled", "skipped", "", None):
                with self.subTest(result=key, state=bad):
                    results = dict(good)
                    if bad is None:
                        del results[key]
                    else:
                        results[key] = bad
                    self.assertFalse(gate_passed(results))


if __name__ == "__main__":
    unittest.main()
