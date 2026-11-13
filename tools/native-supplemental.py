#!/usr/bin/env python3
"""Hosted, unsigned observations only. Never emits aggregate-compatible evidence."""
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def version_tuple(value, major):
    require(isinstance(value, str) and re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*))?", value), "malformed runtime version")
    parts = tuple(int(part) for part in value.split("."))
    require(parts[0] == major and all(part <= 4294967295 for part in parts), "wrong runtime major")
    return parts + (0,) * (3 - len(parts))


def select_destination(runtimes, devices):
    candidates = []
    for runtime in runtimes.get("runtimes", []):
        identifier = runtime.get("identifier", "")
        if not identifier.startswith("com.apple.CoreSimulator.SimRuntime.iOS-") or runtime.get("isAvailable") is not True:
            continue
        try:
            version = version_tuple(runtime.get("version"), 18)
        except ValueError:
            continue
        for device in devices.get("devices", {}).get(identifier, []):
            udid = device.get("udid", "")
            if device.get("isAvailable") is True and re.fullmatch(r"[A-Za-z0-9-]+", udid):
                candidates.append((version, udid, identifier, runtime["version"]))
    require(candidates, "no available preinstalled iOS 18.x simulator; no setup fallback")
    # Latest available 18.x version, then lexical ID. Never create a device/runtime.
    candidates.sort(key=lambda row: (tuple(-n for n in row[0]), row[1], row[2]))
    _, udid, identifier, version = candidates[0]
    return {"id": udid, "runtime_identifier": identifier, "version": version, "platform": "iOS Simulator"}


def validate_host(kernel, arch, os_version, xcode, swift):
    require(kernel == "Darwin" and arch == "arm64", "requires actual arm64 macOS host")
    version_tuple(os_version, 15)
    require(re.search(r"^Xcode (?:1[6-9]|[2-9][0-9])(?:\.[0-9]+)*$", xcode, re.M)
            and re.search(r"^Build version \S+$", xcode, re.M), "full Swift 6-capable Xcode required")
    match = re.search(r"Swift version ([0-9]+)\.", swift)
    require(match and int(match[1]) >= 6, "Swift 6 or newer required")


def validate_results(tests, summary, expected):
    names = set()

    def visit(nodes):
        require(isinstance(nodes, list), "missing test nodes")
        for node in nodes:
            if node.get("nodeType") == "Test Case":
                name = node.get("name")
                require(node.get("result") == "Passed" and isinstance(name, str) and name not in names,
                        "failed/skipped/duplicate assertion")
                names.add(name)
            if "children" in node:
                visit(node["children"])

    visit(tests.get("testNodes"))
    required = {"cur_004_4()", "cur_004_actor_c_roundtrip()"}
    if expected["platform"] == "macOS":
        required.add("cur_004_1()")
    require(names == required, "zero, missing or unexpected assertions")
    require(summary.get("result") == "Passed" and summary.get("totalTestCount") == len(names), "unsuccessful or inconsistent summary")
    require(summary.get("testFailures") == [], "missing or nonempty failures")
    devices = summary.get("devicesAndConfigurations")
    require(isinstance(devices, list) and len(devices) == 1, "missing or ambiguous destination")
    for counts in [summary, devices[0]]:
        require(counts.get("passedTests") == len(names), "wrong passed count")
        for field in ["failedTests", "skippedTests", "expectedFailures"]:
            require(type(counts.get(field)) is int and counts[field] == 0, "unsuccessful test count")
    device = devices[0].get("device", {})
    require(device.get("architecture") == "arm64", "wrong runtime architecture")
    for field, value in expected.items():
        require(device.get(field) == value, f"actual runtime {field} differs from measured/selected value")
    require(device.get("deviceId") and device.get("osBuildNumber"), "missing actual runtime ID/build")
    return sorted(names)


def test_command(role, configuration, destination, output):
    require(role in ["macos", "ios"] and configuration in ["Debug", "Release"], "invalid test role/configuration")
    scheme, plan = ("CurrentsCapture", "CurrentsCaptureSynthetic") if role == "ios" else ("CurrentsPlatformTests", "CurrentsSynthetic")
    target = f"platform=iOS Simulator,id={destination}" if role == "ios" else "platform=macOS,arch=arm64"
    return ["/usr/bin/xcodebuild", "test", "-project", "native/Currents.xcodeproj", "-scheme", scheme,
            "-testPlan", plan, "-destination", target, "-configuration", configuration,
            "-derivedDataPath", str(output / "DerivedData"), "-resultBundlePath", str(output / f"{role}-{configuration}.xcresult"),
            "-parallel-testing-enabled", "NO", "CODE_SIGNING_ALLOWED=NO"]


def new_report():
    return {"schema": "SupplementalNativeObservationV1", "schema_version": 1,
            "scope": "unsigned hosted patched-OS observations only; not CUR-004 matrix evidence",
            "native_qualification": "Blocked", "exact_macos_15_0": "Pending", "production_qualification": "Blocked",
            "collection_status": "Blocked", "commands": [], "runs": []}


class Collector:
    def __init__(self, root, output):
        self.root, self.output = root, output
        self.report = new_report()
        self.deadline = time.monotonic() + 25 * 60

    def save(self):
        path = self.output / "report.json"
        temporary = path.with_suffix(".tmp")
        temporary.write_text(json.dumps(self.report, indent=2) + "\n")
        temporary.replace(path)

    def command(self, args):
        require(time.monotonic() < self.deadline, "collection time budget exhausted")
        number = len(self.report["commands"])
        code, failure = None, None
        try:
            result = subprocess.run(args, cwd=self.root, capture_output=True, timeout=min(480, self.deadline - time.monotonic()), check=False)
            stdout, stderr, code = result.stdout, result.stderr, result.returncode
        except subprocess.TimeoutExpired as error:
            stdout, stderr = error.stdout or b"", error.stderr or b""
            failure = "command timed out"
        except OSError as error:
            stdout, stderr = b"", str(error).encode()
            failure = "command could not start"
        receipt = {"args": args, "exit": code}
        for stream, data in [("stdout", stdout), ("stderr", stderr)]:
            path = self.output / f"command-{number:03d}-{stream}.log"
            path.write_bytes(data)
            receipt[stream] = path.name
            receipt[f"{stream}_sha256"] = hashlib.sha256(data).hexdigest()
        self.report["commands"].append(receipt)
        self.save()
        require(code == 0 and failure is None, f"{failure or 'command failed'}: {args}; exit={code}")
        return stdout.decode("utf-8")

    def snapshot(self):
        return {"candidate_sha": self.command(["git", "rev-parse", "HEAD"]).strip(),
                "tree": self.command(["git", "rev-parse", "HEAD^{tree}"]).strip(),
                "status": self.command(["git", "status", "--porcelain", "--untracked-files=all"])}

    def collect(self):
        require(os.environ.get("GITHUB_ACTIONS") == "true", "hosted GitHub Actions runner required; do not run native tests locally")
        before = self.snapshot()
        self.report["before"] = before
        require(not before["status"], "dirty candidate before collection")
        require(before["candidate_sha"] == os.environ.get("GITHUB_SHA"), "checkout differs from exact workflow candidate SHA")
        self.report["workflow_sha"] = os.environ["GITHUB_SHA"]
        self.report["checkout_action"] = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
        host = {
            "kernel": self.command(["uname", "-s"]).strip(),
            "arch": self.command(["uname", "-m"]).strip(),
            "version": self.command(["/usr/bin/sw_vers", "-productVersion"]).strip(),
            "build": self.command(["/usr/bin/sw_vers", "-buildVersion"]).strip(),
            "xcode": self.command(["/usr/bin/xcodebuild", "-version"]),
            "swift": self.command(["/usr/bin/xcrun", "swift", "--version"]),
            "macos_sdk": self.command(["/usr/bin/xcrun", "--sdk", "macosx", "--show-sdk-version"]).strip(),
            "ios_sdk": self.command(["/usr/bin/xcrun", "--sdk", "iphonesimulator", "--show-sdk-version"]).strip(),
            "developer_dir": os.environ.get("DEVELOPER_DIR"),
        }
        self.report["host"] = host
        validate_host(host["kernel"], host["arch"], host["version"], host["xcode"], host["swift"])
        require(host["build"] and host["macos_sdk"] and host["ios_sdk"], "missing host build or SDK versions")
        runtimes = json.loads(self.command(["/usr/bin/xcrun", "simctl", "list", "runtimes", "--json"]))
        devices = json.loads(self.command(["/usr/bin/xcrun", "simctl", "list", "devices", "--json"]))
        destination = select_destination(runtimes, devices)
        self.report["supplemental_destination"] = destination
        self.save()
        for role in ["macos", "ios"]:
            expected = {"platform": "macOS", "osVersion": host["version"], "osBuildNumber": host["build"]} if role == "macos" else {
                "platform": "iOS Simulator", "osVersion": destination["version"], "deviceId": destination["id"]}
            for configuration in ["Debug", "Release"]:
                self.command(test_command(role, configuration, destination["id"], self.output))
                bundle = self.output / f"{role}-{configuration}.xcresult"
                parsed = {}
                for section in ["tests", "summary"]:
                    raw = self.command(["/usr/bin/xcrun", "xcresulttool", "get", "test-results", section, "--path", str(bundle), "--compact"])
                    (self.output / f"{role}-{configuration}-{section}.json").write_text(raw)
                    parsed[section] = json.loads(raw)
                assertions = validate_results(parsed["tests"], parsed["summary"], expected)
                self.report["runs"].append({"role": role, "configuration": configuration, "assertions": assertions,
                    "actual_runtime": parsed["summary"]["devicesAndConfigurations"][0]["device"], "result_bundle": bundle.name})
                self.save()
        after = self.snapshot()
        self.report["after"] = after
        require(before == after and not after["status"], "candidate/tree/clean state changed during collection")
        self.report["collection_status"] = "Observed"
        self.save()


def main():
    root = Path(__file__).resolve().parent.parent
    output = root / "target/native-supplemental"
    # Fresh directory only: previous artifacts must never be reused as new observations.
    output.parent.mkdir(exist_ok=True)
    output.mkdir(exist_ok=False)
    collector = Collector(root, output)
    collector.save()
    try:
        collector.collect()
    except Exception as error:  # Preserve partial evidence even for an unexpected tool format.
        collector.report["error"] = str(error)
        collector.save()
        print(f"BLOCKED: {error}", file=sys.stderr)
        return 1
    print("Supplemental observations collected. Native qualification remains Blocked; exact macOS 15.0 remains pending.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
