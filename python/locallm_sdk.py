"""LocalLM Python SDK (Phase 7): drive schedules + config over the loopback API.

The desktop app serves 127.0.0.1:<port> when the webhook listener is enabled
(Settings -> webhook, token rotated there). Every call is bearer-authed and
body-bounded server-side; nothing here bypasses approval or audit.

Example:
    from locallm_sdk import Client
    client = Client(token="...")
    print(client.config())
    job = client.run_schedule("sched-123")
"""

from __future__ import annotations

import json
import time
import urllib.request


class Error(Exception):
    pass


class Client:
    def __init__(self, token: str, host: str = "127.0.0.1", port: int = 4317, timeout: int = 30):
        if not token or len(token) > 512:
            raise Error("A webhook bearer token is required (rotate one in the desktop app).")
        self._base = f"http://{host}:{port}"
        self._token = token
        self._timeout = timeout

    def _call(self, method: str, path: str, body: dict | None = None) -> dict | list:
        data = json.dumps(body or {}).encode() if body is not None or method == "POST" else None
        request = urllib.request.Request(
            self._base + path, data=data, method=method,
            headers={"Authorization": f"Bearer {self._token}", "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=self._timeout) as response:
                payload = response.read()
        except OSError as error:
            raise Error(f"Is the desktop app running with the webhook listener enabled? ({error})")
        if len(payload) > 1_048_576:
            raise Error("API response exceeded 1 MiB; refusing to parse.")
        try:
            return json.loads(payload)
        except ValueError:
            raise Error(f"API returned non-JSON: {payload[:200]!r}")

    def config(self) -> dict:
        result = self._call("GET", "/api/config")
        assert isinstance(result, dict)
        return result

    def schedules(self) -> list:
        result = self._call("GET", "/api/schedules")
        assert isinstance(result, list)
        return result

    def run_schedule(self, schedule_id: str) -> dict:
        if not schedule_id or len(schedule_id) > 64:
            raise Error("Schedule id must be 1-64 characters.")
        result = self._call("POST", "/api/schedules/run", {"id": schedule_id})
        assert isinstance(result, dict)
        return result

    def webhook(self, schedule_id: str) -> dict:
        if not schedule_id or len(schedule_id) > 64:
            raise Error("Schedule id must be 1-64 characters.")
        result = self._call("POST", f"/webhook/{schedule_id}", {})
        assert isinstance(result, dict)
        return result

    def wait_for_result(self, schedule_id: str, timeout: int = 600, poll: int = 5) -> str:
        deadline = time.time() + timeout
        while time.time() < deadline:
            for item in self.schedules():
                if item.get("id") == schedule_id and item.get("lastRunAt") is not None:
                    return str(item.get("lastResult") or "")
            time.sleep(poll)
        raise Error(f"No result for {schedule_id} within {timeout}s.")
