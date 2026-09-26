"""Tests for MCP auth (app/auth.py + the OAuth half of app/keystore.py).

Standalone ``unittest`` — run with::

    cd /workspaces/sdarm/qdrant && ../.venv/bin/python3.11 -m app.tests.test_auth

The MCP endpoints are stood in for by a trivial route, so no Qdrant, model or
FastMCP is needed; only the auth layer is under test.
"""

from __future__ import annotations

import base64
import hashlib
import os
import sys
import tempfile
import unittest
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

_TMP = tempfile.mkdtemp(prefix="qdrant-auth-test-")
os.environ["KEYS_DB"] = str(Path(_TMP) / "keys.db")
os.environ.pop("PUBLIC_URL", None)
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from starlette.applications import Starlette  # noqa: E402
from starlette.responses import PlainTextResponse  # noqa: E402
from starlette.routing import Route  # noqa: E402
from starlette.testclient import TestClient  # noqa: E402

from app import auth, keystore  # noqa: E402

CLAUDE_CB = "https://claude.ai/api/mcp/auth_callback"
PATHS = ["/mcp", "/sop/mcp", "/bible/mcp"]


async def _tool(request):
    return PlainTextResponse("tool:" + request.scope["state"]["api_key_name"])


def _client() -> TestClient:
    routes = auth.routes(PATHS) + [Route(p, _tool, methods=["GET", "POST"]) for p in PATHS]
    return TestClient(auth.ApiKeyMiddleware(Starlette(routes=routes)), follow_redirects=False)


def _pkce() -> tuple[str, str]:
    verifier = base64.urlsafe_b64encode(os.urandom(40)).rstrip(b"=").decode()
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    return verifier, challenge


def _authorize_params(client_id: str, challenge: str, redirect_uri: str = CLAUDE_CB) -> dict:
    return {"response_type": "code", "client_id": client_id, "redirect_uri": redirect_uri,
            "state": "st4te", "code_challenge": challenge, "code_challenge_method": "S256",
            "resource": "http://testserver/mcp"}


def _code_from(resp) -> str:
    assert resp.status_code == 302, (resp.status_code, resp.text)
    q = parse_qs(urlsplit(resp.headers["location"]).query)
    assert q["state"] == ["st4te"]
    return q["code"][0]


class AuthTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        keystore.init()
        cls.key = keystore.create("tester")
        cls.c = _client()

    # ── header path (unchanged behaviour) ──────────────────────────────────
    def test_header_key_still_works(self):
        r = self.c.post("/mcp", headers={"Authorization": f"Bearer {self.key}"})
        self.assertEqual(r.text, "tool:tester")

    def test_401_points_at_resource_metadata(self):
        r = self.c.post("/sop/mcp")
        self.assertEqual(r.status_code, 401)
        self.assertIn('resource_metadata="http://testserver/.well-known/oauth-protected-resource/sop/mcp"',
                      r.headers["www-authenticate"])

    # ── discovery ──────────────────────────────────────────────────────────
    def test_discovery(self):
        prm = self.c.get("/.well-known/oauth-protected-resource/bible/mcp").json()
        self.assertEqual(prm["resource"], "http://testserver/bible/mcp")
        self.assertEqual(prm["authorization_servers"], ["http://testserver"])
        self.assertEqual(self.c.get("/.well-known/oauth-protected-resource").json()["resource"],
                         "http://testserver/mcp")
        asm = self.c.get("/.well-known/oauth-authorization-server").json()
        self.assertEqual(asm["code_challenge_methods_supported"], ["S256"])
        self.assertIn("none", asm["token_endpoint_auth_methods_supported"])
        self.assertTrue(asm["client_id_metadata_document_supported"])

    # ── client secret = API key ────────────────────────────────────────────
    def test_secret_flow_post_and_refresh(self):
        verifier, challenge = _pkce()
        code = _code_from(self.c.get("/oauth/authorize",
                                     params=_authorize_params("anything", challenge)))
        r = self.c.post("/oauth/token", data={
            "grant_type": "authorization_code", "code": code, "redirect_uri": CLAUDE_CB,
            "code_verifier": verifier, "client_id": "anything", "client_secret": self.key})
        self.assertEqual(r.status_code, 200, r.text)
        tok = r.json()
        self.assertTrue(tok["access_token"].startswith("qda_"))
        self.assertEqual(self.c.post("/mcp", headers={
            "Authorization": f"Bearer {tok['access_token']}"}).text, "tool:tester")

        # refresh rotates: the old refresh token is single use
        r2 = self.c.post("/oauth/token", data={"grant_type": "refresh_token",
                                               "refresh_token": tok["refresh_token"]})
        self.assertEqual(r2.status_code, 200)
        self.assertNotEqual(r2.json()["refresh_token"], tok["refresh_token"])
        r3 = self.c.post("/oauth/token", data={"grant_type": "refresh_token",
                                               "refresh_token": tok["refresh_token"]})
        self.assertEqual(r3.json()["error"], "invalid_grant")

    def test_secret_flow_basic_auth(self):
        verifier, challenge = _pkce()
        code = _code_from(self.c.get("/oauth/authorize", params=_authorize_params("bible-sop", challenge)))
        basic = base64.b64encode(f"bible-sop:{self.key}".encode()).decode()
        r = self.c.post("/oauth/token", headers={"Authorization": f"Basic {basic}"}, data={
            "grant_type": "authorization_code", "code": code, "redirect_uri": CLAUDE_CB,
            "code_verifier": verifier})
        self.assertEqual(r.status_code, 200, r.text)

    def test_wrong_secret_and_code_reuse(self):
        verifier, challenge = _pkce()
        code = _code_from(self.c.get("/oauth/authorize", params=_authorize_params("x", challenge)))
        r = self.c.post("/oauth/token", data={
            "grant_type": "authorization_code", "code": code, "redirect_uri": CLAUDE_CB,
            "code_verifier": verifier, "client_id": "x", "client_secret": "qd_wrong"})
        self.assertEqual(r.status_code, 401)
        self.assertEqual(r.json()["error"], "invalid_client")
        # the code was consumed by the failed attempt
        r = self.c.post("/oauth/token", data={
            "grant_type": "authorization_code", "code": code, "redirect_uri": CLAUDE_CB,
            "code_verifier": verifier, "client_id": "x", "client_secret": self.key})
        self.assertEqual(r.json()["error"], "invalid_grant")

    def test_bad_pkce(self):
        _, challenge = _pkce()
        code = _code_from(self.c.get("/oauth/authorize", params=_authorize_params("x", challenge)))
        r = self.c.post("/oauth/token", data={
            "grant_type": "authorization_code", "code": code, "redirect_uri": CLAUDE_CB,
            "code_verifier": "not-it", "client_id": "x", "client_secret": self.key})
        self.assertEqual(r.json()["error"], "invalid_grant")

    # ── DCR / CIMD + paste page ────────────────────────────────────────────
    def test_dcr_paste_flow(self):
        reg = self.c.post("/oauth/register", json={"redirect_uris": [CLAUDE_CB], "client_name": "Claude"})
        self.assertEqual(reg.status_code, 201)
        cid = reg.json()["client_id"]
        verifier, challenge = _pkce()
        params = _authorize_params(cid, challenge)
        page = self.c.get("/oauth/authorize", params=params)
        self.assertEqual(page.status_code, 200)
        self.assertIn("claude.ai", page.text)
        self.assertEqual(page.headers["x-frame-options"], "DENY")

        bad = self.c.post("/oauth/authorize", data={**params, "api_key": "qd_nope"})
        self.assertEqual(bad.status_code, 200)
        self.assertIn("not valid", bad.text)

        code = _code_from(self.c.post("/oauth/authorize", data={**params, "api_key": self.key}))
        r = self.c.post("/oauth/token", data={
            "grant_type": "authorization_code", "code": code, "redirect_uri": CLAUDE_CB,
            "code_verifier": verifier, "client_id": cid})
        self.assertEqual(r.status_code, 200, r.text)

    def test_cimd_loopback_gets_page_with_warning(self):
        _, challenge = _pkce()
        params = _authorize_params("https://claude.ai/oauth/claude-code-client-metadata", challenge,
                                   redirect_uri="http://localhost:41234/callback")
        page = self.c.get("/oauth/authorize", params=params)
        self.assertEqual(page.status_code, 200)
        self.assertIn("this computer", page.text)

    def test_redirect_allowlist(self):
        _, challenge = _pkce()
        r = self.c.get("/oauth/authorize",
                       params=_authorize_params("x", challenge, redirect_uri="https://evil.example/cb"))
        self.assertEqual(r.status_code, 400)
        self.assertEqual(self.c.post("/oauth/register",
                                     json={"redirect_uris": ["https://evil.example/cb"]}).status_code, 400)

    def test_pkce_required(self):
        params = _authorize_params("x", "c")
        params.pop("code_challenge")
        r = self.c.get("/oauth/authorize", params=params)
        self.assertEqual(r.status_code, 302)
        self.assertIn("error=invalid_request", r.headers["location"])

    # ── revocation reaches OAuth tokens ────────────────────────────────────
    def test_revoking_key_ends_oauth_session(self):
        key = keystore.create("to-revoke")
        verifier, challenge = _pkce()
        code = _code_from(self.c.get("/oauth/authorize", params=_authorize_params("x", challenge)))
        tok = self.c.post("/oauth/token", data={
            "grant_type": "authorization_code", "code": code, "redirect_uri": CLAUDE_CB,
            "code_verifier": verifier, "client_id": "x", "client_secret": key}).json()
        kid = keystore.key_id(key)
        self.assertEqual(keystore.oauth_sessions().get(kid), 1)
        keystore.revoke(kid)
        self.assertEqual(self.c.post("/mcp", headers={
            "Authorization": f"Bearer {tok['access_token']}"}).status_code, 401)
        r = self.c.post("/oauth/token", data={"grant_type": "refresh_token",
                                              "refresh_token": tok["refresh_token"]})
        self.assertEqual(r.json()["error"], "invalid_grant")


if __name__ == "__main__":
    unittest.main(verbosity=2)
