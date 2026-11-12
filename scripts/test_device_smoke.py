import subprocess
import unittest
from unittest.mock import patch

from device_smoke import Smoke


class DeviceSmokeSafetyTests(unittest.TestCase):
    def test_oem_guide_for_another_app_is_never_approved(self):
        smoke = Smoke("explicit-device", "/unused")
        calls = []
        def shell(*args, **kwargs):
            calls.append(args)
            if args[0] == "dumpsys":
                return "mCurrentFocus=Window{Sys2039:com.oplus.safecenter/com.oplus.safecenter.accessibility.AccessibilityGuideCloseActivity}"
            if args[0] == "cat":
                return '<hierarchy><node package="com.oplus.safecenter" text="Another app"/><node package="com.oplus.safecenter" text="保持开启" enabled="true" bounds="[10,30][30,70]"/></hierarchy>'
            return ""
        with patch.object(smoke, "shell", side_effect=shell):
            self.assertFalse(smoke.dismiss_own_accessibility_guide())
        self.assertFalse(any(call[0] == "input" for call in calls))

    def test_only_the_named_apps_keep_enabled_button_is_tapped(self):
        smoke = Smoke("explicit-device", "/unused")
        calls = []
        def shell(*args, **kwargs):
            calls.append(args)
            if args[0] == "dumpsys":
                return "mCurrentFocus=Window{Sys2039:com.oplus.safecenter/com.oplus.safecenter.accessibility.AccessibilityGuideCloseActivity}"
            if args[0] == "cat":
                return '<hierarchy><node package="com.oplus.safecenter" text="GUI Use"/><node package="com.oplus.safecenter" text="保持开启" enabled="true" bounds="[10,30][30,70]"/></hierarchy>'
            return ""
        with patch.object(smoke, "shell", side_effect=shell):
            self.assertTrue(smoke.dismiss_own_accessibility_guide())
        self.assertIn(("input", "tap", "20", "50"), calls)

    def test_file_existence_uses_exit_status_not_remote_error_text(self):
        smoke = Smoke("explicit-device", "/unused")
        result = subprocess.CompletedProcess(["adb"], 1, stdout=b"remote diagnostic", stderr=b"")
        with patch("device_smoke.subprocess.run", return_value=result):
            self.assertEqual(smoke.adb("shell", "test", "-f", "missing", check=False, status_only=True), 1)

    def test_adb_timeouts_do_not_expose_pairing_arguments(self):
        smoke = Smoke("explicit-device", "/unused")
        with patch("device_smoke.subprocess.run", side_effect=subprocess.TimeoutExpired(["adb", "private-pairing-token"], 1)):
            with self.assertRaises(RuntimeError) as failure:
                smoke.adb("shell", "private-pairing-token")
        self.assertNotIn("private-pairing-token", str(failure.exception))
        self.assertIn("redacted", str(failure.exception))

    def test_non_oppo_devices_are_refused_before_installation(self):
        smoke = Smoke("explicit-device", "/unused")
        with patch.object(smoke, "shell", return_value="Another vendor"), patch.object(smoke, "adb") as adb:
            with self.assertRaises(AssertionError):
                smoke.run()
        adb.assert_not_called()


if __name__ == "__main__":
    unittest.main()
