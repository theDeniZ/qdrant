"""Unit tests for sopack.doctor's model-cache check.

Runnable standalone:
    /workspaces/sdarm/.venv/bin/python3.11 -m sopack.tests.test_doctor
"""

from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from sopack import doctor  # noqa: E402

_REPO = "qdrant/multilingual-e5-large-onnx"


class CacheRootTest(unittest.TestCase):
    def test_env_var_wins(self):
        with mock.patch.dict(os.environ, {"FASTEMBED_CACHE_PATH": "/x/y"}):
            self.assertEqual(doctor._cache_root(), Path("/x/y"))

    def test_default_is_fastembeds_temp_dir_not_home_cache(self):
        env = {k: v for k, v in os.environ.items() if k != "FASTEMBED_CACHE_PATH"}
        with mock.patch.dict(os.environ, env, clear=True):
            self.assertEqual(doctor._cache_root(),
                             Path(tempfile.gettempdir()) / "fastembed_cache")


class ModelCacheCheckTest(unittest.TestCase):
    def _run(self, root: Path) -> doctor._Result:
        with mock.patch.dict(os.environ, {"FASTEMBED_CACHE_PATH": str(root)}), \
                mock.patch.object(doctor, "_resolve_hf_repo", return_value=_REPO):
            return doctor._check_model_cache()

    def test_missing_root_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            r = self._run(Path(tmp) / "absent")
        self.assertFalse(r.ok)
        self.assertIn("does not exist", r.detail)

    def test_cached_model_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            snap = Path(tmp) / "models--qdrant--multilingual-e5-large-onnx" / "snapshots" / "abc"
            snap.mkdir(parents=True)
            (snap / "model.onnx").write_bytes(b"")
            r = self._run(Path(tmp))
        self.assertTrue(r.ok, r.detail)

    def test_successful_load_rechecks_a_stale_cache_fail(self):
        stale = doctor._Result("model cache", False, "does not exist")
        fresh = doctor._Result("model cache", True, "downloaded by the load")
        loads = doctor._Result("model loads", True, "dim 1024")
        with mock.patch.object(doctor, "CHECKS", (lambda: stale,)), \
                mock.patch.object(doctor, "_check_model_loads", return_value=loads), \
                mock.patch.object(doctor, "_check_model_cache", return_value=fresh):
            results = doctor.run()
        self.assertEqual([r.ok for r in results], [True, True])


if __name__ == "__main__":
    unittest.main()
