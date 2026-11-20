"""Offline synthetic tests; never invoke Xcode or qualify an actual runtime."""
import hashlib
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("native_supplemental", Path(__file__).with_name("native-supplemental.py"))
sut = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sut)


class SupplementalTests(unittest.TestCase):
    def inventory(self):
        runtimes = {"runtimes": [
            {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-18-5", "version": "18.5", "isAvailable": True},
            {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-18-6", "version": "18.6", "isAvailable": True},
            {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-27-0", "version": "27.0", "isAvailable": True},
        ]}
        devices = {"devices": {
            runtime["identifier"]: [{"udid": f"TEST-{i}", "isAvailable": True}]
            for i, runtime in enumerate(runtimes["runtimes"])
        }}
        return runtimes, devices

    def test_selects_available_18_minor_deterministically(self):
        runtimes, devices = self.inventory()
        selected = sut.select_destination(runtimes, devices)
        self.assertEqual(selected["version"], "18.6")
        self.assertEqual(selected["id"], "TEST-1")
        runtimes["runtimes"].reverse()
        self.assertEqual(selected, sut.select_destination(runtimes, devices))
        devices["devices"]["com.apple.CoreSimulator.SimRuntime.iOS-18-6"][0]["isAvailable"] = False
        self.assertEqual(sut.select_destination(runtimes, devices)["version"], "18.5")

    def test_missing_unavailable_or_wrong_runtime_is_blocked(self):
        for version in ["17.6", "27.0", "18", "18.6beta", "18..6", "018.6", "18.06"]:
            runtimes, devices = self.inventory()
            runtimes["runtimes"] = [runtimes["runtimes"][0]]
            runtimes["runtimes"][0]["version"] = version
            with self.assertRaises(ValueError):
                sut.select_destination(runtimes, devices)
        runtimes, devices = self.inventory()
        for runtime in runtimes["runtimes"]:
            runtime["isAvailable"] = False
        with self.assertRaises(ValueError):
            sut.select_destination(runtimes, devices)

    def test_requires_actual_arm64_macos15_and_full_xcode(self):
        sut.validate_host("Darwin", "arm64", "15.8.2", "Xcode 16.4\nBuild version 16F6\n", "Apple Swift version 6.1\n")
        for kernel, arch, os, xcode, swift in [
            ("Linux", "arm64", "15.8.2", "Xcode 16.4\nBuild version 16F6", "Swift version 6.1"),
            ("Darwin", "x86_64", "15.8.2", "Xcode 16.4\nBuild version 16F6", "Swift version 6.1"),
            ("Darwin", "arm64", "26.0", "Xcode 16.4\nBuild version 16F6", "Swift version 6.1"),
            ("Darwin", "arm64", "15.8.2", "CLT", "Swift version 6.1"),
            ("Darwin", "arm64", "15.8.2", "Xcode 16.4\nBuild version 16F6", "Swift version 5.9"),
        ]:
            with self.assertRaises(ValueError):
                sut.validate_host(kernel, arch, os, xcode, swift)

    def results(self, ios=False):
        names = ["cur_004_4()", "cur_004_actor_c_roundtrip()"]
        if not ios:
            names.append("cur_004_1()")
        tests = {"testNodes": [{"nodeType": "Test Case", "name": n, "result": "Passed"} for n in names]}
        device = {"platform": "iOS Simulator" if ios else "macOS", "architecture": "arm64", "osVersion": "18.6" if ios else "15.8.2", "deviceId": "TEST-1", "osBuildNumber": "TEST-BUILD"}
        run = {"device": device, "passedTests": len(names), "failedTests": 0, "skippedTests": 0, "expectedFailures": 0}
        summary = {"result": "Passed", "totalTestCount": len(names), "passedTests": len(names), "failedTests": 0, "skippedTests": 0, "expectedFailures": 0, "testFailures": [], "devicesAndConfigurations": [run]}
        return tests, summary, device.copy()

    def test_validates_actual_assertions_and_runtime(self):
        for ios in [False, True]:
            tests, summary, expected = self.results(ios)
            self.assertEqual(set(sut.validate_results(tests, summary, expected)), {n["name"] for n in tests["testNodes"]})
        for field, value in [("osVersion", "18.5"), ("deviceId", "OTHER"), ("architecture", "x86_64"), ("platform", "macOS")]:
            tests, summary, expected = self.results(True)
            summary["devicesAndConfigurations"][0]["device"][field] = value
            with self.assertRaises(ValueError):
                sut.validate_results(tests, summary, expected)
        for field, value in [("totalTestCount", 0), ("skippedTests", 1), ("failedTests", 1), ("testFailures", None)]:
            tests, summary, expected = self.results()
            summary[field] = value
            with self.assertRaises(ValueError):
                sut.validate_results(tests, summary, expected)
        for outcome in ["Skipped", "Failed", "Expected Failure"]:
            tests, summary, expected = self.results()
            tests["testNodes"][0]["result"] = outcome
            with self.assertRaises(ValueError):
                sut.validate_results(tests, summary, expected)
        tests, summary, expected = self.results()
        tests["testNodes"].append(tests["testNodes"][0])
        with self.assertRaises(ValueError):
            sut.validate_results(tests, summary, expected)

    def test_commands_and_report_cannot_claim_full_qualification(self):
        command = sut.test_command("ios", "Release", "TEST-1", Path("target/test-only"))
        self.assertIn("CurrentsCaptureSynthetic", command)
        self.assertIn("platform=iOS Simulator,id=TEST-1", command)
        self.assertIn("CODE_SIGNING_ALLOWED=NO", command)
        report = sut.new_report()
        self.assertEqual(report["schema"], "SupplementalNativeObservationV1")
        self.assertEqual(report["native_qualification"], "Blocked")
        self.assertEqual(report["minimum_runtime_qualification"], "Pending")
        self.assertNotIn("qualified_ios_destination", report)

    def test_failed_command_retains_raw_logs_without_promoting_qualification(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            collector = sut.Collector(output, output)
            with self.assertRaises(ValueError):
                collector.command([sys.executable, "-c", "import sys; print('synthetic stdout'); print('synthetic stderr', file=sys.stderr); sys.exit(17)"])
            report = json.loads((output / "report.json").read_text())
            receipt = report["commands"][0]
            self.assertEqual(receipt["exit"], 17)
            for stream in ["stdout", "stderr"]:
                raw = (output / receipt[stream]).read_bytes()
                self.assertEqual(receipt[stream + "_sha256"], hashlib.sha256(raw).hexdigest())
                self.assertIn(b"synthetic", raw)
            self.assertEqual(report["collection_status"], "Blocked")
            self.assertEqual(report["native_qualification"], "Blocked")

    def test_workflow_is_bounded_read_only_and_keeps_failure_artifacts(self):
        workflow = json.loads((Path(__file__).resolve().parent.parent / ".github/workflows/native-supplemental.yml").read_text())
        self.assertEqual(workflow["permissions"], {"contents": "read"})
        self.assertEqual(set(workflow["on"]), {"workflow_dispatch", "pull_request"})
        job = workflow["jobs"]["observe"]
        self.assertEqual(job["runs-on"], "macos-15")
        self.assertLessEqual(job["timeout-minutes"], 30)
        checkout, upload = job["steps"][0], job["steps"][-1]
        self.assertFalse(checkout["with"]["persist-credentials"])
        self.assertEqual(checkout["with"]["ref"], "${{ github.event.pull_request.head.sha || github.sha }}")
        self.assertEqual(job["env"]["CURRENTS_CANDIDATE_SHA"], checkout["with"]["ref"])
        self.assertNotIn("DerivedData", upload["with"]["path"])
        runs = [step.get("run", "") for step in job["steps"]]
        self.assertIn("python3 tools/native-canonical.py", runs)
        self.assertNotIn("python3 tools/native-supplemental.py", runs)
        self.assertEqual(checkout["uses"], "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1")
        self.assertEqual(upload["uses"], "actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9")
        self.assertEqual(upload["if"], "${{ always() }}")
        self.assertNotIn("secrets", json.dumps(workflow))


if __name__ == "__main__":
    unittest.main()
