from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"


class McpChartTest(unittest.TestCase):
    def render(self, *settings: str) -> subprocess.CompletedProcess:
        command = ["helm", "template", "mcp-test", str(CHART)]
        for setting in settings:
            command.extend(["--set", setting])
        return subprocess.run(command, capture_output=True, text=True, check=False)

    def test_mcp_routes_and_configuration_are_opt_in(self) -> None:
        disabled = self.render("ingress.enabled=true")
        self.assertEqual(disabled.returncode, 0, disabled.stderr)
        self.assertNotIn("MCP__ENABLED", disabled.stdout)
        self.assertNotIn("oauth-protected-resource", disabled.stdout)
        enabled = self.render("ingress.enabled=true", "mcp.enabled=true", "mcp.resourceUrl=https://mcp.example/mcp", "mcp.issuer=https://identity.example/realms/product", "mcp.allowedHosts[0]=mcp.example")
        self.assertEqual(enabled.returncode, 0, enabled.stderr)
        for path in ["/mcp", "/.well-known/oauth-protected-resource", "/.well-known/oauth-protected-resource/mcp"]:
            self.assertIn(f"- path: {path}\n            pathType: Exact", enabled.stdout)
        self.assertIn('value: "https://mcp.example/mcp"', enabled.stdout)
        self.assertIn('value: "https://identity.example/realms/product"', enabled.stdout)
        self.assertIn('MCP__ALLOWED_HOSTS', enabled.stdout)
        self.assertIn('value: "[\\"mcp.example\\"]"', enabled.stdout)

    def test_enabled_mcp_requires_a_host_allowlist(self) -> None:
        result = self.render("mcp.enabled=true", "mcp.resourceUrl=https://mcp.example/mcp", "mcp.issuer=https://identity.example")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("mcp.allowedHosts must not be empty", result.stderr)


if __name__ == "__main__":
    unittest.main()
