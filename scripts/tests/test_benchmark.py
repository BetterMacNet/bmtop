"""Synthetic unit tests of the measurement harness, NOT bmtop performance data."""
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("benchmark", Path(__file__).parents[1] / "benchmark.py")
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


def fixture():
    return {"scenario": {"interval_ms": 1000}, "platform": "synthetic", "runtime_version": 1,
            "cpu_seconds_wait4": 1.0, "rss_p95_bytes": 100.0,
            "runtime": {"samples": 10, "sample_errors": 0, "detail_errors": 0, "renders": 12,
                        "input_events": 1, "detail_service": {"total_observations": 1}}}


class BenchmarkContractTests(unittest.TestCase):
    def test_percentiles(self):
        self.assertIsNone(benchmark.percentile([], 95))
        self.assertEqual(benchmark.percentile(list(range(1, 101)), 50), 50)
        self.assertEqual(benchmark.percentile(list(range(1, 101)), 95), 95)
        with self.assertRaises(ValueError):
            benchmark.percentile([float("nan")], 95)

    def test_interval_validation(self):
        for text, millis in [("250ms", 250), ("1s", 1000), ("60s", 60000)]:
            self.assertEqual(benchmark.parse_interval(text), millis)
        for text in ["0s", "1h", "249ms", "61s", "bad"]:
            with self.assertRaises(ValueError):
                benchmark.parse_interval(text)

    def test_idle_redraw_gate(self):
        report = fixture()
        self.assertEqual(benchmark.quality_failures(report), [])
        report["runtime"]["renders"] = 200
        self.assertTrue(benchmark.quality_failures(report))

    def test_no_samples_are_inconclusive(self):
        report = fixture()
        report["runtime"]["samples"] = 0
        self.assertIn("inconclusive", benchmark.quality_failures(report)[0])

    def test_regression_and_incompatible_scenario(self):
        baseline = fixture()
        report = copy.deepcopy(baseline)
        report["cpu_seconds_wait4"] = 1.5
        self.assertTrue(benchmark.compare(report, baseline, 20))
        report["scenario"] = {"interval_ms": 5000}
        with self.assertRaises(ValueError):
            benchmark.compare(report, baseline, 20)

    def test_zero_baseline_does_not_falsely_pass(self):
        baseline = fixture()
        baseline["cpu_seconds_wait4"] = 0
        self.assertIn("inconclusive", benchmark.compare(fixture(), baseline, 20)[0])

    def test_private_exclusive_output(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.json"
            benchmark.save_new(path, {"synthetic": True})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                benchmark.save_new(path, {"replace": True})
            target = Path(directory) / "target.json"
            target.write_text("do not overwrite")
            symlink = Path(directory) / "link.json"
            symlink.symlink_to(target)
            with self.assertRaises(FileExistsError):
                benchmark.save_new(symlink, {})
            self.assertEqual(target.read_text(), "do not overwrite")

if __name__ == "__main__":
    unittest.main()
