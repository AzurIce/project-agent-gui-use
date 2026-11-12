"""Opt-in end-to-end tests on one explicitly selected OPPO device.

Only the application's local test surface and local RemoteInput fixture are used.
Pairing material and screenshots stay in a private temporary directory. Services,
forwarding, preferences and the test host are restored/stopped even on failure.
"""

import argparse
import base64
import concurrent.futures
import json
import os
import re
from pathlib import Path
import signal
import socket
import struct
import subprocess
import tempfile
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "dev.azurice.guiuse"
COMPONENT = f"{PACKAGE}/{PACKAGE}.DeviceService"
NOTIFICATIONS = f"{PACKAGE}/{PACKAGE}.NotificationService"


class Smoke:
    def __init__(self, serial, directory, skip_install=False, ptc_agent=False):
        self.serial = serial
        self.directory = Path(directory)
        self.host = ROOT / "target/debug/gui-use-host"
        self.process = None
        self.log = None
        self.results = []
        self.skip_install = skip_install
        self.ptc_agent = ptc_agent

    def adb(self, *arguments, check=True, data=None, status_only=False):
        try:
            result = subprocess.run(["adb", "-s", self.serial, *arguments], input=data, capture_output=True, timeout=90)
        except subprocess.TimeoutExpired:
            raise RuntimeError("ADB operation timed out (arguments redacted)") from None
        if check and result.returncode:
            # Never echo arguments: configuration commands contain pairing credentials.
            raise RuntimeError(f"ADB operation failed ({result.returncode}): {result.stderr.decode(errors='replace')[:500]}")
        return result.returncode if status_only else result.stdout

    def shell(self, *arguments, **kwargs):
        return self.adb("shell", *arguments, **kwargs).decode().strip()

    def debug(self, command, **extras):
        arguments = ["am", "broadcast", "-n", f"{PACKAGE}/.DebugControlReceiver", "--es", "command", command]
        for key, value in extras.items():
            arguments += ["--ez" if isinstance(value, bool) else "--es", key, str(value).lower() if isinstance(value, bool) else value]
        self.shell(*arguments)

    def dismiss_own_accessibility_guide(self):
        state = self.shell("dumpsys", "window", check=False)
        foreground = next((line for line in state.splitlines() if line.strip().startswith("mCurrentFocus=")), "")
        if "com.oplus.safecenter" not in foreground or "AccessibilityGuideCloseActivity" not in foreground:
            return False
        path = "/data/local/tmp/gui-use-accessibility.xml"
        self.shell("uiautomator", "dump", path)
        nodes = list(ET.fromstring(self.shell("cat", path)).iter("node"))
        if not any("GUI Use" in node.get("text", "") and node.get("package") == "com.oplus.safecenter" for node in nodes):
            return False  # Never approve a prompt for another application.
        for node in nodes:
            if node.get("package") != "com.oplus.safecenter" or node.get("text") not in ("保持开启", "Keep enabled") or node.get("enabled") != "true":
                continue
            bounds = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
            if bounds:
                left, top, right, bottom = map(int, bounds.groups())
                self.shell("input", "tap", str((left + right) // 2), str((top + bottom) // 2))
                return True
        return False

    def control(self, method, params=None, timeout=10000):
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(timeout / 1000 + 5)
            connection.connect(str(self.directory / "control.sock"))
            connection.sendall((json.dumps({"method": method, "params": params or {}, "timeout_ms": timeout}) + "\n").encode())
            with connection.makefile("rb") as reader:
                return json.loads(reader.readline())

    def call(self, method, params=None, expected_error=None, timeout=10000):
        response = self.control(method, params, timeout)
        if expected_error is not None:
            assert response.get("error", {}).get("code") == expected_error, (method, response)
            return response
        assert "error" not in response, (method, response)
        return response["result"]

    def wait(self, condition, timeout=45, recover=None):
        deadline = time.monotonic() + timeout
        last = None
        def retry_action():
            if recover is not None:
                try:
                    recover()
                except (OSError, AssertionError, StopIteration):
                    pass
        while time.monotonic() < deadline:
            try:
                value = condition()
                if value:
                    return value
            except (OSError, AssertionError, json.JSONDecodeError, StopIteration) as error:
                last = str(error)
                retry_action()
            if self.dismiss_own_accessibility_guide():
                time.sleep(0.5)  # UiAutomation may briefly rebind accessibility services.
                retry_action()
            time.sleep(0.2)
        raise AssertionError(f"Timed out waiting for test condition: {last}")

    def start_host(self, port):
        self.log = open(self.directory / "host.log", "ab", buffering=0)
        self.process = subprocess.Popen([str(self.host), "--state-dir", str(self.directory), "serve", "--listen", f"127.0.0.1:{port}"], stdout=self.log, stderr=self.log)
        self.wait(lambda: self.control("host.devices"), 15)

    def stop_host(self):
        if self.process is not None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(5)
            self.process = None
        if self.log is not None:
            self.log.close()
            self.log = None

    def snapshot(self):
        return self.call("ui.snapshot", {"filter": "all"})

    @staticmethod
    def node(snapshot, suffix):
        return next(node for node in snapshot["nodes"] if (node.get("view_id") or "").endswith("/" + suffix))

    def record(self, name):
        self.results.append(name)
        print(f"PASS {name}", flush=True)

    def check_ptc_agent(self, device_id):
        agent = ROOT / "target/debug/gui-use-agent"
        command = [str(agent), "--state-dir", str(self.directory), "--device", device_id]
        # The reference program operates ONLY the already-owned fixture. A single
        # execution batches launch, observation, three clicks, waits and verification.
        def execute():
            process = subprocess.run(command + ["--allow-actions", "exec", "--file", str(ROOT / "examples/ptc/diagnostics.js")],
                capture_output=True, text=True, timeout=45)
            result = json.loads(process.stdout)
            assert process.returncode == 0 and result["ok"], result.get("error")
            return result
        result = self.wait(execute)
        assert result["result"]["verified"] and result["result"]["clicks"] == 3
        assert result["result"]["after"] == f'Count: {result["result"]["before"] + 3}'
        assert result["stats"]["rpc_calls"] == 13
        assert result["stats"]["rpc_response_bytes"] > 5 * result["stats"]["model_visible_bytes"]
        readonly = subprocess.run(command + ["exec", "--file", str(ROOT / "examples/ptc/diagnostics.js")],
            capture_output=True, text=True, timeout=10)
        denied = json.loads(readonly.stdout)
        assert readonly.returncode != 0 and not denied["ok"] and "Actions are disabled" in denied["error"]
        print("PTC byte counts:", json.dumps(result["stats"]), flush=True)
        self.record("isolated PTC runtime, batched real-device actions, compact evidence and read-only enforcement")

    def check_service_lifecycle(self):
        self.shell("input", "keyevent", "KEYCODE_HOME")
        assert "apps" in self.call("app.list")
        services = self.shell("dumpsys", "activity", "services", PACKAGE)
        assert "ConnectionService" in services and "isForeground=true" in services
        self.record("foreground service remains connected with the control activity closed")
        old_pid = self.shell("pidof", PACKAGE)
        assert old_pid.isdigit(), "Only the owned application process may be killed"
        self.shell("run-as", PACKAGE, "kill", "-9", old_pid)
        self.wait(lambda: self.shell("pidof", PACKAGE, check=False) not in ("", old_pid), timeout=60)
        self.wait(lambda: self.control("host.devices").get("result"), timeout=60)
        assert "apps" in self.call("app.list")
        self.record("sticky foreground service and connection recover after owned process death")
        self.debug("stop-action")
        self.wait(lambda: not self.control("host.devices")["result"])
        prefs = self.adb("exec-out", "run-as", PACKAGE, "cat", "shared_prefs/connection.xml")
        assert b'name="service_enabled" value="false"' in prefs
        time.sleep(1)
        assert not self.control("host.devices")["result"]
        self.debug("start")
        self.wait(lambda: self.control("host.devices").get("result"))
        self.record("explicit service stop persists and explicit start reconnects")

    def check_usb_bootstrap(self):
        self.debug("disconnect")
        self.stop_host()
        directory = self.directory / "automatic-host"
        command = [str(ROOT / "target/debug/gui-use-agent"), "--state-dir", str(directory),
            "--serial", self.serial, "--allow-actions", "exec", "--file", str(ROOT / "examples/ptc/diagnostics.js")]
        try:
            def execute():
                process = subprocess.run(command, capture_output=True, text=True, timeout=50)
                assert process.returncode == 0, process.stderr
                value = json.loads(process.stdout)
                assert value["ok"], value.get("error")
                return value
            result = self.wait(execute, timeout=75)
            assert result["result"]["verified"] and result["result"]["clicks"] == 3
            assert (directory / "pairing.json").is_file()
            self.wait(lambda: not (directory / "control.sock").exists())
            services = self.shell("dumpsys", "activity", "services", PACKAGE)
            assert "ConnectionService" in services and "isForeground=true" in services
            self.record("single-command USB identity, embedded host, pairing import, device selection and PTC")
            def inspect_foreground():
                inspect = subprocess.run(command[:-1] + [str(ROOT / "examples/ptc/inspect.js")],
                    capture_output=True, text=True, timeout=50)
                assert inspect.returncode == 0, inspect.stderr
                observed = json.loads(inspect.stdout)
                assert observed["ok"] and any((node.get("view_id") or "").endswith("/test_increment") for node in observed["result"]["controls"])
                return observed
            # The deliberate process-death test may produce a delayed, GUI Use-only
            # OEM accessibility guide. Dismiss it using the existing scoped helper
            # and restore our fixture before retrying the READ-ONLY inspection.
            self.wait(inspect_foreground, timeout=75, recover=lambda: self.debug("fixture"))
            self.record("repeated USB tasks preserve the foreground application instead of reopening settings")
            self.debug("clear")
            self.debug("start")
            services = self.shell("dumpsys", "activity", "services", PACKAGE)
            self.wait(lambda: "isForeground=true" in self.shell("dumpsys", "activity", "services", PACKAGE))
            self.record("unconfigured service can run in waiting-for-computer state")
        finally:
            if (directory / "pairing.json").exists():
                from urllib.parse import urlparse
                port = urlparse(json.loads((directory / "pairing.json").read_text())["endpoint"]).port
                self.adb("reverse", "--remove", f"tcp:{port}", check=False)

    def run(self):
        assert self.shell("getprop", "ro.product.manufacturer").upper() == "OPPO", "Refusing to test a non-OPPO device"
        originals = {key: self.shell("settings", "get", "secure", key) for key in ("enabled_accessibility_services", "accessibility_enabled")}
        previous_listeners = self.shell("settings", "get", "secure", "enabled_notification_listeners")
        existed = bool(self.shell("pm", "path", PACKAGE, check=False))
        notification_permission = "android.permission.POST_NOTIFICATIONS: granted=true" in self.shell("dumpsys", "package", PACKAGE, check=False) if existed else True
        preferences_exist = existed and self.adb("shell", "run-as", PACKAGE, "test", "-f", "shared_prefs/connection.xml", check=False, status_only=True) == 0
        saved_preferences = self.adb("exec-out", "run-as", PACKAGE, "cat", "shared_prefs/connection.xml") if preferences_exist else b""
        existing_reverse = self.adb("reverse", "--list").decode().split()
        while True:
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                port = listener.getsockname()[1]
            if f"tcp:{port}" not in existing_reverse:
                break
        try:
            if not self.skip_install:
                self.adb("install", "-r", "-g", str(ROOT / "android/app/build/outputs/apk/debug/app-debug.apk"))
            self.shell("pm", "grant", PACKAGE, "android.permission.POST_NOTIFICATIONS", check=False)
            self.shell("input", "keyevent", "KEYCODE_WAKEUP")
            self.shell("am", "start", "-n", f"{PACKAGE}/.MainActivity")
            existing = originals["enabled_accessibility_services"]
            entries = [] if existing in ("", "null") else existing.split(":")
            if COMPONENT not in entries:
                entries.append(COMPONENT)
            self.shell("settings", "put", "secure", "enabled_accessibility_services", ":".join(entries))
            self.shell("settings", "put", "secure", "accessibility_enabled", "1")
            self.shell("cmd", "notification", "allow_listener", NOTIFICATIONS)
            subprocess.run([str(self.host), "--state-dir", str(self.directory), "init", "--endpoint", f"wss://localhost:{port}"], check=True, capture_output=True)
            pairing = json.loads((self.directory / "pairing.json").read_text())
            self.adb("reverse", f"tcp:{port}", f"tcp:{port}")
            self.start_host(port)
            self.debug("configure", endpoint=pairing["endpoint"], token=pairing["token"], pin=pairing["certificate_sha256"])
            devices = self.wait(lambda: self.control("host.devices").get("result"))
            assert len(devices) == 1
            self.record("pinned TLS, authenticated hello and capability negotiation")
            apps = self.call("app.list", {"include_system": True})["apps"]
            assert any(app["package_name"] == PACKAGE for app in apps)
            self.record("application enumeration")
            self.dismiss_own_accessibility_guide()
            # Permission/service activation can bring an OEM system page forward.
            # Give the app foreground status before testing its API-driven launch.
            self.shell("am", "start", "-n", f"{PACKAGE}/.MainActivity")
            assert self.call("app.launch", {"package_name": PACKAGE, "activity": ".DiagnosticsActivity"})["accepted"]
            self.wait(lambda: self.call("ui.wait", {"until": "window", "value": PACKAGE, "timeout_ms": 2000}),
                recover=lambda: self.call("app.launch", {"package_name": PACKAGE, "activity": ".DiagnosticsActivity"}))
            snapshot = self.wait(lambda: (s if any((n.get("view_id") or "").endswith("/test_input") for n in s["nodes"]) else None) if (s := self.snapshot()) else None,
                recover=lambda: self.call("app.launch", {"package_name": PACKAGE, "activity": ".DiagnosticsActivity"}))
            self.record("launch and live accessibility snapshot")
            initial_input = self.node(snapshot, "test_input")
            old_input = initial_input["id"]
            if not initial_input["flags"]["focused"]:
                assert self.call("ui.act", {"node": old_input, "action": "focus"})["changed"]
            input_value = "autonomous-local-test-" + str(time.monotonic_ns())
            assert self.call("ui.act", {"node": old_input, "action": "set_text", "text": input_value})["changed"]
            snapshot = self.wait(lambda: (s if self.node(s, "test_input")["text"] == input_value else None) if (s := self.snapshot()) else None)
            assert self.node(snapshot, "test_input")["text"] == input_value
            self.call("ui.act", {"node": old_input, "action": "click"}, expected_error=2001)
            marker = "NativeTextEvent" + str(time.monotonic_ns())
            def repeat_text_change():
                node = self.node(self.snapshot(), "test_input")
                if not node["flags"]["focused"]:
                    self.call("ui.act", {"node": node["id"], "action": "focus"})
                state = self.shell("dumpsys", "activity", "activities", check=False)
                foreground = next((line for line in state.splitlines() if line.strip().startswith("ResumedActivity:")), "")
                assert f"{PACKAGE}/.DiagnosticsActivity" in foreground, "Typing is restricted to our local fixture"
                # External, human-like input tests the listener independently of
                # ACTION_SET_TEXT, which Android can report as content-only events.
                self.shell("input", "text", marker)
            repeat_text_change()
            changed = self.wait(lambda: [record for record in self.control("host.events")["result"]
                if record["event"]["params"]["type"] == "ui.text_changed" and
                marker in record["event"]["params"]["data"]["text"]], recover=repeat_text_change)
            assert changed[-1]["event"]["params"]["data"]["node_hint"]
            self.record("set_text and stale-node rejection")
            snapshot = self.snapshot()
            increment = self.node(snapshot, "test_increment")
            assert self.call("ui.act", {"node": increment["id"], "action": "click"})["changed"]
            assert self.call("ui.wait", {"until": "text", "value": "Count: 1", "timeout_ms": 3000})["matched"]
            snapshot = self.snapshot()
            bounds = self.node(snapshot, "test_increment")["bounds"]
            point = {"x": (bounds["left"] + bounds["right"]) // 2, "y": (bounds["top"] + bounds["bottom"]) // 2}
            assert self.call("ui.gesture", {"strokes": [{"points": [point], "duration_ms": 80}]})["performed"]
            assert self.call("ui.wait", {"until": "text", "value": "Count: 2", "timeout_ms": 3000})["matched"]
            assert self.call("ui.wait", {"until": "idle", "timeout_ms": 5000})["matched"]
            self.record("semantic click, physical gesture and wait conditions")
            snapshot = self.snapshot()
            scroll = self.node(snapshot, "test_scroll")
            assert self.call("ui.act", {"node": scroll["id"], "action": "scroll_forward"})["changed"]
            self.call("ui.wait", {"until": "idle", "timeout_ms": 5000})
            scroll = self.node(self.snapshot(), "test_scroll")
            assert self.call("ui.act", {"node": scroll["id"], "action": "scroll_backward"})["changed"]
            self.call("ui.wait", {"until": "idle", "timeout_ms": 5000})
            checkbox = self.node(self.snapshot(), "test_checkbox")
            assert self.call("ui.act", {"node": checkbox["id"], "action": "click"})["changed"]
            self.wait(lambda: self.node(self.snapshot(), "test_checkbox")["flags"]["checked"] is True)
            snapshot = self.snapshot()
            input_node = self.node(snapshot, "test_input")
            assert self.call("ui.act", {"node": input_node["id"], "action": "clear_text"})["changed"]
            self.call("ui.act", {"node": input_node["id"], "action": "unsupported"}, expected_error=2002)
            assert self.call("ui.snapshot", {"filter": "all", "max_nodes": 1})["truncated"]
            self.call("ui.snapshot", {"max_nodes": 0}, expected_error=-32602)
            self.call("ui.wait", {"until": "text", "value": "does-not-exist-local", "timeout_ms": 150}, expected_error=2004)
            self.call("app.launch", {"package_name": "com.android.settings"}, expected_error=2005)
            self.call("app.launch", {"package_name": PACKAGE, "force_stop": True}, expected_error=2002)
            self.call("unknown.method", expected_error=-32601)
            self.record("parameter validation, permissions, unsupported actions and timeouts")
            response = self.control("ui.screenshot", {"max_width": 320})
            assert "error" not in response, response
            png = base64.b64decode(response["binary"])
            assert png.startswith(b"\x89PNG\r\n\x1a\n")
            width, height = struct.unpack(">II", png[16:24])
            assert width == 320 and height == response["result"]["height"] and len(png) == response["result"]["bytes"]
            self.record("screenshot metadata and correlated PNG binary frame")
            self.debug("post")
            notifications = self.wait(lambda: [item for item in self.call("notification.list")["notifications"] if item["title"] == "GUI Use test notification"])
            key = notifications[0]["key"]
            assert notifications[0]["actions"][0]["remote_input"]
            reply_text = "local-test-reply-" + str(time.monotonic_ns())
            assert self.call("notification.reply", {"key": key, "text": reply_text})["sent"]
            self.wait(lambda: self.call("ui.wait", {"until": "text", "value": "Reply: " + reply_text, "timeout_ms": 2000}))
            self.debug("cancel")
            def notification_events():
                kinds = {record["event"]["params"]["type"] for record in self.control("host.events")["result"]}
                return {"notification.posted", "notification.removed", "ui.content_changed", "ui.idle"}.issubset(kinds)
            self.wait(notification_events)
            self.record("notification list, local RemoteInput reply and posted/removed UI events")
            self.debug("fixture", sensitive=True)
            self.wait(lambda: self.control("ui.snapshot").get("error", {}).get("code") == 2003)
            self.call("ui.snapshot", expected_error=2003)
            self.call("ui.screenshot", expected_error=2003)
            self.debug("fixture")
            self.wait(lambda: self.snapshot())
            self.record("password-screen blocking without real credentials")
            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
                values = list(pool.map(lambda _: self.call("app.list"), range(8)))
                assert all("apps" in value for value in values)
            self.record("concurrent correlated requests")
            self.stop_host()
            self.start_host(port)
            self.wait(lambda: self.control("host.devices").get("result"), 45)
            assert self.call("app.list")["apps"]
            self.record("automatic reconnect after host restart")
            self.debug("disconnect")
            self.wait(lambda: not self.control("host.devices")["result"])
            self.debug("configure", endpoint=pairing["endpoint"], token="0" * 64, pin=pairing["certificate_sha256"])
            time.sleep(3)
            assert not self.control("host.devices")["result"]
            self.debug("disconnect")
            self.debug("configure", endpoint=pairing["endpoint"], token=pairing["token"], pin="0" * 64)
            time.sleep(3)
            assert not self.control("host.devices")["result"]
            self.debug("disconnect")
            self.debug("configure", endpoint=pairing["endpoint"], token=pairing["token"], pin=pairing["certificate_sha256"])
            self.wait(lambda: self.control("host.devices").get("result"))
            self.record("wrong-token and wrong-certificate rejection, then successful recovery")
            cli = subprocess.run([str(self.host), "--state-dir", str(self.directory), "call", "app.list"], capture_output=True, check=True, text=True)
            assert "apps" in json.loads(cli.stdout)["result"]
            native = subprocess.run([str(self.host), "--state-dir", str(self.directory), "apps", "--include-system"], capture_output=True, check=True, text=True)
            assert any(app["package_name"] == PACKAGE for app in json.loads(native.stdout)["result"]["apps"])
            output = self.directory / "fixture.png"
            subprocess.run([str(self.host), "--state-dir", str(self.directory), "call", "ui.screenshot", "--params", '{"max_width":160}', "--output", str(output)], capture_output=True, check=True)
            assert output.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
            assert output.stat().st_mode & 0o777 == 0o600
            self.record("host CLI")
            if self.ptc_agent:
                self.check_ptc_agent(devices[0]["id"])
                self.check_service_lifecycle()
                self.check_usb_bootstrap()
        except Exception:
            state = self.shell("dumpsys", "activity", "activities", check=False)
            print("Foreground at failure:", next((line.strip() for line in state.splitlines() if line.strip().startswith("ResumedActivity:")), "unavailable"), flush=True)
            window = self.shell("dumpsys", "window", check=False)
            print("Focused window at failure:", [line.strip() for line in window.splitlines() if line.strip().startswith("mCurrentFocus=")], flush=True)
            try:
                events = self.control("host.events")["result"]
                kinds = {}
                for record in events:
                    name = record["event"]["params"]["type"]
                    kinds[name] = kinds.get(name, 0) + 1
                text_stats = [{"length": len(record["event"]["params"]["data"]["text"]), "has_hint": bool(record["event"]["params"]["data"]["node_hint"])} for record in events if record["event"]["params"]["type"] == "ui.text_changed"]
                print("Event diagnostics (no text payloads):", kinds, text_stats, flush=True)
            except Exception:
                pass
            raise
        finally:
            cleanup_errors = []
            def clean(action):
                try:
                    action()
                except Exception as error:
                    cleanup_errors.append(type(error).__name__)
            clean(lambda: self.debug("cancel"))
            clean(lambda: self.debug("clear"))
            clean(lambda: self.shell("am", "force-stop", PACKAGE, check=False))
            clean(lambda: self.shell("run-as", PACKAGE, "rm", "-f", "shared_prefs/connection.xml.bak", check=False))
            if saved_preferences:
                clean(lambda: self.adb("shell", "run-as", PACKAGE, "dd", "of=shared_prefs/connection.xml", data=saved_preferences, check=False))
            else:
                clean(lambda: self.shell("run-as", PACKAGE, "rm", "-f", "shared_prefs/connection.xml", check=False))
            for key, value in originals.items():
                if value in ("", "null"):
                    clean(lambda key=key: self.shell("settings", "delete", "secure", key, check=False))
                else:
                    clean(lambda key=key, value=value: self.shell("settings", "put", "secure", key, value, check=False))
            if NOTIFICATIONS not in previous_listeners:
                clean(lambda: self.shell("cmd", "notification", "disallow_listener", NOTIFICATIONS, check=False))
            if existed and not notification_permission:
                clean(lambda: self.shell("pm", "revoke", PACKAGE, "android.permission.POST_NOTIFICATIONS", check=False))
            clean(lambda: self.adb("reverse", "--remove", f"tcp:{port}", check=False))
            clean(lambda: self.shell("rm", "-f", "/data/local/tmp/gui-use-accessibility.xml", check=False))
            self.stop_host()
            if cleanup_errors:
                raise RuntimeError(f"Device cleanup needs attention: {cleanup_errors}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True, help="Explicit OPPO adb serial; no automatic device selection")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--skip-install", action="store_true", help="Use an already installed build (e.g. after approving an OEM installer prompt)")
    parser.add_argument("--ptc-agent", action="store_true", help="Also test the isolated PTC executor on the owned DiagnosticsActivity")
    arguments = parser.parse_args()
    if not arguments.skip_build:
        subprocess.run(["cargo", "build", "--locked", "-p", "gui-use-host", "-j8"], cwd=ROOT, check=True)
        subprocess.run(["./android/gradlew", "-p", "android", ":app:testDebugUnitTest", ":app:assembleDebug", "--max-workers=4", "--no-daemon"], cwd=ROOT, check=True)
    if arguments.ptc_agent and not arguments.skip_build:
        subprocess.run(["cargo", "build", "--locked", "-p", "gui-use-agent", "-j8"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="gui-use-e2e-", dir=os.environ.get("GUI_USE_TEST_TMP")) as directory:
        smoke = Smoke(arguments.serial, directory, arguments.skip_install, arguments.ptc_agent)
        smoke.run()
        print(json.dumps({"passed": len(smoke.results), "checks": smoke.results, "device": "OPPO", "cleanup": "complete"}, indent=2))


if __name__ == "__main__":
    main()
