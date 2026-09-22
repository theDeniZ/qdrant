"""Environment check for the Mac side of the pipeline.

Run before a real ``sopack pack`` — especially before the first one on a new
machine — so a broken environment fails in a second, with a readable table,
instead of twenty minutes into a run (R7) or after a 19-minute model download
(failure #14, the onnxruntime external-data trap).

Every check is best-effort and independent: one failing check does not stop
the rest from running, so the table always shows the full picture.
"""

from __future__ import annotations

import argparse
import os
import platform
import sys
import tempfile
from pathlib import Path

from . import contract

__all__ = ["run", "main"]


class _Result:
    def __init__(self, name: str, ok: bool, detail: str):
        self.name = name
        self.ok = ok
        self.detail = detail


def _check_python() -> _Result:
    ok = sys.version_info[:2] >= (3, 11)
    return _Result("python", ok,
                    f"{platform.python_version()} at {sys.executable}"
                    + ("" if ok else " (need >= 3.11)"))


def _check_fastembed() -> _Result:
    try:
        import fastembed
    except ImportError as exc:
        return _Result("fastembed", False, f"not importable: {exc}")
    got = getattr(fastembed, "__version__", None)
    want = contract.EMBEDDING["library_version"]
    if got != want:
        return _Result("fastembed", False,
                        f"{got} installed, contract requires {want}")
    return _Result("fastembed", True, f"{got} (matches contract)")


def _check_numpy() -> _Result:
    try:
        import numpy
    except ImportError as exc:
        return _Result("numpy", False, f"not importable: {exc}")
    return _Result("numpy", True, numpy.__version__)


def _cache_root() -> Path:
    env = os.environ.get("FASTEMBED_CACHE_PATH")
    if env:
        return Path(env)
    return Path.home() / ".cache" / "fastembed"


def _resolve_hf_repo(model_id: str) -> str | None:
    """The HF repo that actually serves *model_id*'s ONNX export, per
    fastembed's own registry — NOT a guess. fastembed maps a friendly model
    id like ``intfloat/multilingual-e5-large`` onto whatever repo ships the
    ONNX weights, which is commonly a *different* org and name (here:
    ``qdrant/multilingual-e5-large-onnx``). Returns ``None`` if fastembed
    isn't importable or the registry shape doesn't match what we expect —
    callers must have a weaker fallback for that case, never a guess dressed
    up as a lookup."""
    try:
        from fastembed import TextEmbedding
    except ImportError:
        return None
    try:
        for entry in TextEmbedding.list_supported_models():
            if entry.get("model") == model_id:
                return (entry.get("sources") or {}).get("hf")
    except Exception:  # noqa: BLE001 - registry internals are not our contract
        return None
    return None


def _check_model_cache() -> _Result:
    root = _cache_root()
    if not root.is_dir():
        return _Result("model cache", False, f"{root} does not exist")

    model_id = contract.EMBEDDING["model"]
    hf_repo = _resolve_hf_repo(model_id)
    if hf_repo is not None:
        expected = root / ("models--" + hf_repo.replace("/", "--"))
        if not expected.is_dir():
            present = ", ".join(p.name for p in root.glob("models--*")) or "none"
            return _Result("model cache", False,
                            f"{expected.name} not found under {root} (resolved "
                            f"via fastembed registry: {model_id!r} -> {hf_repo!r}); "
                            f"present: {present}")
        onnx_files = list(expected.rglob("model.onnx"))
        if not onnx_files:
            return _Result("model cache", False,
                            f"{expected} present but no model.onnx under it "
                            f"(resolved via registry: {model_id!r} -> {hf_repo!r})")
        return _Result("model cache", True,
                        f"{expected.name} ({len(onnx_files)} model.onnx file(s), "
                        f"resolved via registry: {model_id!r} -> {hf_repo!r})")

    # Registry lookup failed — fall back to "is anything at all cached",
    # and say plainly that this is the weaker check, not silently degrade.
    any_models = list(root.glob("models--*"))
    onnx_files = list(root.rglob("model.onnx"))
    if not any_models or not onnx_files:
        return _Result("model cache", False,
                        f"{root}: no cached model.onnx found (weak check — "
                        "could not resolve the HF repo via fastembed's registry)")
    return _Result("model cache", True,
                    f"weak check only (registry lookup failed): "
                    f"{len(any_models)} model dir(s), {len(onnx_files)} model.onnx file(s)")


def _check_model_loads(quick: bool) -> _Result:
    """Failure #14 (onnxruntime >=1.23 rejecting ``model.onnx_data`` whose
    symlink resolves outside the directory it treats as the model root) is
    **not** reliably detectable by inspecting the cache layout: HF's normal,
    healthy cache always symlinks ``snapshots/<rev>/model.onnx_data`` to a
    content hash under a sibling ``blobs/`` directory, so a path heuristic
    cannot tell a working cache from a broken one — only onnxruntime's own
    loader can. So this check is empirical: actually construct the model and
    embed one string, exactly what ``sopack pack`` is about to do for real.

    Costs ~30s (model load) — that is what makes it worth running: skipping
    it is exactly how failure #14 surfaced only after a 19-minute download in
    the original incident. ``quick=True`` skips it anyway (for a fast
    sanity pass) and is honest that doing so proves nothing."""
    if quick:
        return _Result("model loads", True, "skipped (--quick) — NOT VERIFIED")
    try:
        from fastembed import TextEmbedding
    except ImportError as exc:
        return _Result("model loads", False, f"fastembed not importable: {exc}")
    try:
        model = TextEmbedding(model_name=contract.EMBEDDING["model"])
        vectors = list(model.embed(
            [contract.EMBEDDING["passage_prefix"] + "sopack doctor check"]))
    except Exception as exc:  # noqa: BLE001 - report whatever onnxruntime raises
        msg = str(exc)
        hint = ""
        if "onnx_data" in msg or "external data" in msg.lower():
            hint = " — this is failure #14, the onnxruntime external-data trap"
        return _Result("model loads", False, f"{type(exc).__name__}: {msg}{hint}")
    vec = vectors[0]
    dim = len(vec.tolist() if hasattr(vec, "tolist") else vec)
    if dim != contract.VECTOR_SIZE:
        return _Result("model loads", False,
                        f"loaded and embedded, but produced dim {dim}, "
                        f"contract requires {contract.VECTOR_SIZE}")
    return _Result("model loads", True,
                    f"loaded {contract.EMBEDDING['model']}, embedded 1 string, "
                    f"dim {dim} matches contract")


def _check_cpu_count() -> _Result:
    n = os.cpu_count()
    return _Result("cpu_count", n is not None and n > 0, str(n))


def _check_tmp_writable() -> _Result:
    try:
        with tempfile.NamedTemporaryFile(prefix="sopack-doctor-") as fh:
            fh.write(b"x")
            path = fh.name
        return _Result("writable temp", True, str(Path(path).parent))
    except OSError as exc:
        return _Result("writable temp", False, str(exc))


CHECKS = (
    _check_python,
    _check_fastembed,
    _check_numpy,
    _check_model_cache,
    _check_cpu_count,
    _check_tmp_writable,
)


def run(quick: bool = False) -> list[_Result]:
    results = [check() for check in CHECKS]
    results.append(_check_model_loads(quick))
    return results


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="sopack doctor", description=__doc__)
    ap.add_argument("--quick", action="store_true",
                     help="skip the ~30s real model load/embed check "
                          "(that check is what actually verifies failure #14 "
                          "is not present — skipping it proves nothing about it)")
    args = ap.parse_args(argv)

    results = run(quick=args.quick)
    width = max(len(r.name) for r in results)
    for r in results:
        mark = "PASS" if r.ok else "FAIL"
        print(f"[{mark}] {r.name.ljust(width)}  {r.detail}")
    ok = all(r.ok for r in results)
    print("all checks passed" if ok else "one or more checks FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
