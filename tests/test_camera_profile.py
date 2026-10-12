"""Keep performance reports isolated from captures and setup transients."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("camera_profile", ROOT / "scripts/profile-camera.py")
PROFILE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROFILE)


class CameraProfileTests(unittest.TestCase):
    def test_process_accounting_handles_spaces_and_parentheses_in_command(self):
        # utime/stime are fields 14/15; do not split the parenthesized name.
        fields = ["S"] + ["0"] * 21
        fields[11], fields[12] = "120", "30"
        stat = "123 (a worker (camera)) " + " ".join(fields)
        with patch.object(Path, "read_text", side_effect=[stat, "Name:\tworker\nVmRSS:\t8192 kB\n"]), patch.object(PROFILE.os, "sysconf", return_value=100):
            self.assertEqual(PROFILE.read_process(123), (1.5, 8192))

    def test_phase_statistics_exclude_setup_and_other_phases(self):
        samples = [
            {"phase": "preview", "elapsed_s": second, "cpu_percent": 100 if second < 4 else 10, "rss_kib": 1000 + second, "decode_us": 3000, "dropped_preview": 0}
            for second in range(1, 7)
        ]
        samples.append({"phase": "recording", "elapsed_s": 7, "cpu_percent": 500, "rss_kib": 9000})
        summary = PROFILE.phase_summary(samples, "preview")
        self.assertEqual(summary["samples"], 3)
        self.assertEqual(summary["cpu_percent_mean"], 10)
        self.assertEqual(summary["rss_kib_first"], 1004)
        self.assertEqual(summary["rss_kib_last"], 1006)
        self.assertEqual(PROFILE.phase_summary([], "preview"), {"samples": 0})

    def test_existing_captures_are_preserved_without_launching_application(self):
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            binary = folder / "app"
            binary.write_bytes(b"fixture")
            output = folder / "captures"
            output.mkdir()
            existing = output / "original.jpg"
            existing.write_bytes(b"preserve")
            with patch("sys.argv", ["profile-camera.py", str(binary), str(output)]), patch.object(PROFILE.platform, "system", return_value="Linux"), patch.object(PROFILE.subprocess, "Popen") as launch, self.assertRaises(SystemExit):
                PROFILE.main()
            launch.assert_not_called()
            self.assertEqual(existing.read_bytes(), b"preserve")


if __name__ == "__main__":
    unittest.main()
