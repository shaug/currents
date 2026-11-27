"""Fail-closed composition of exact-checkout PR workflow gates."""
import json
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
GATES = ("core", "quality-policy", "msrv", "docs", "privacy", "doubleentry")
SHA_CHECK = 'test "$(git rev-parse HEAD)" = "$GITHUB_SHA"'


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
        # Linux runners are slower, so Linux core takes three shards and macOS two.
        self.assertEqual(core["strategy"]["matrix"], {"include": [
            {"os": os, "shard": shard, "shards": shards}
            for os, shards in (("ubuntu-24.04", 3), ("macos-15", 2))
            for shard in range(1, shards + 1)
        ]})
        self.assertEqual(core["runs-on"], "${{ matrix.os }}")
        self.assertEqual(core["name"],
                         "core (${{ matrix.os }}, ${{ matrix.shard }}/${{ matrix.shards }})")
        msrv = self.jobs["msrv"]
        self.assertEqual(msrv["strategy"]["matrix"], {"shard": [1, 2, 3]})
        self.assertEqual(msrv["name"], "msrv (${{ matrix.shard }}/3)")
        shard = {"core": "${{ matrix.shard }}/${{ matrix.shards }}",
                 "msrv": "${{ matrix.shard }}/3"}
        for gate in ("core", "msrv", "docs", "privacy"):
            step = {"name": f"Run {gate} gate", "run": f"cargo xtask check {gate}"}
            if gate in ("core", "msrv"):
                # Each runner takes one shard; together they run every test once.
                step["env"] = {"CURRENTS_CI_CORE_PHASES": "1",
                               "CURRENTS_TEST_SHARD": shard[gate]}
            # About twice each job's slowest time in recent runs.
            self.assertEqual(self.jobs[gate]["timeout-minutes"],
                             {"core": 25, "msrv": 25, "docs": 10, "privacy": 5}[gate])
            self.assertEqual(self.jobs[gate]["steps"][-1], step)
        # The core job runs the workspace build and tests on every event, so the
        # quality job never repeats them, including on push.
        quality_policy = self.jobs["quality-policy"]
        self.assertEqual(quality_policy["timeout-minutes"], 10)
        tests, quality, policy = quality_policy["steps"][-3:]
        self.assertEqual(tests, {
            "name": "Test quality orchestration",
            "run": "PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools -p test_pr_quality.py",
        })
        self.assertEqual(quality, {"name": "Run quality checks",
                                   "run": "cargo xtask check quality-ci"})
        self.assertEqual(policy, {"name": "Qualify policy counterexamples",
                                  "run": "cargo xtask ticket CUR-003"})
        for step in quality_policy["steps"]:
            self.assertNotIn("if", step)
            self.assertNotEqual(step.get("run"), "cargo xtask check quality")

    def test_only_the_dependency_policy_job_builds_cargo_deny(self):
        install = ('python3 tools/install-ci-tools.py{}\n'
                   'echo "CURRENTS_TOOLS_BIN=$PWD/target/ci-tools/bin" >> "$GITHUB_ENV"')
        for name in GATES:
            if name == "doubleentry":
                continue  # Needs no policy tools.
            with self.subTest(job=name):
                step = next(s for s in self.jobs[name]["steps"]
                            if s.get("name") == "Install checksum-pinned policy tools")
                flag = "" if name == "quality-policy" else " --without-cargo-deny"
                self.assertEqual(step["run"], install.format(flag))

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


class DoubleentryJob(unittest.TestCase):
    """The vendored suite runs once for each new set of inputs."""

    def test_runs_the_vendored_suite_only_without_an_earlier_pass(self):
        workflow = json.loads((ROOT / ".github/workflows/ci.yml").read_text())
        job = workflow["jobs"]["doubleentry"]
        self.assertEqual(job["strategy"]["matrix"], {"include": [
            {"os": "ubuntu-24.04", "toolchain": "1.99.0"},
            {"os": "macos-15", "toolchain": "1.99.0"},
            {"os": "ubuntu-24.04", "toolchain": "1.94.0"},
        ]})
        _, _, inputs, earlier, install, suite, record = job["steps"]
        # The key names every input of the suite's result, plus OS and compiler.
        self.assertIn("git rev-parse HEAD:vendor HEAD:patches HEAD:rust-toolchain.toml "
                      "HEAD:.cargo HEAD:crates/currents-storage/tests/cur_005.rs", inputs["run"])
        self.assertIn("$RUNNER_OS-$RUNNER_ARCH-$TOOLCHAIN-$fingerprint", inputs["run"])
        key = "${{ steps.inputs.outputs.key }}"
        self.assertRegex(earlier["uses"], r"^actions/cache/restore@[a-f0-9]{40}$")
        self.assertEqual(earlier["with"], {"key": key, "path": "target/doubleentry-passed",
                                           "lookup-only": True})
        self.assertRegex(record["uses"], r"^actions/cache/save@[a-f0-9]{40}$")
        self.assertEqual(record["with"], {"key": key, "path": "target/doubleentry-passed"})
        miss = "steps.earlier.outputs.cache-hit != 'true'"
        for step in (install, suite, record):
            self.assertEqual(step["if"], miss)
        # The pass is recorded only after the whole suite passed.
        self.assertTrue(suite["run"].startswith(
            'rustup run "$TOOLCHAIN" cargo xtask ticket CUR-005\n'))
        self.assertNotIn("continue-on-error", suite)
        # The core pool leaves exactly this test binary to this job.
        pool = (ROOT / "xtask/src/test_pool.rs").read_text()
        self.assertIn('const SEPARATE: &[&str] = &["currents-storage/cur_005"];', pool)


if __name__ == "__main__":
    unittest.main()
