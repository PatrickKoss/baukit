import importlib.util
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

SCRIPT = Path(__file__).resolve().parent / "check_docs_links.py"
SPEC = importlib.util.spec_from_file_location("check_docs_links", SCRIPT)
links = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(links)


class CheckDocsLinksTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        run_process(["git", "init", "-q", str(self.root)], check=True)

    def commit(self, files: dict[str, str]) -> None:
        for relative, text in files.items():
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        run_process(["git", "-C", str(self.root), "add", "."], check=True)
        run_process(
            [
                "git",
                "-C",
                str(self.root),
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "t",
            ],
            check=True,
        )

    def test_planted_broken_link_fails(self):
        self.commit({"docs/a.md": "[x](missing.md)\n", "README.md": "[a](docs/a.md)\n"})
        self.assertEqual(
            links.check(self.root, ["README.md", "docs"]), (2, ["docs/a.md:1 -> missing.md"])
        )

    def test_empty_root_list_checks_no_files(self):
        self.commit({"README.md": "# r\n"})
        self.assertEqual(links.check(self.root, ["nothing"]), (0, []))

    def test_links_to_pending_guide_and_adr_pages_pass(self):
        self.commit(
            {
                "README.md": "[g](docs/guide/cli.md) [a](docs/adr/0007-toolkit.md)\n",
                "docs/platform/x.md": "[g](../guide/auth.md) [o](../other.md)\n",
            }
        )
        self.assertEqual(
            links.check(self.root, ["README.md", "docs"]),
            (2, ["docs/platform/x.md:1 -> ../other.md"]),
        )

    def test_expected_broken_links_may_only_shrink(self):
        self.commit({"README.md": "[x](gone.md)\n"})
        self.assertEqual(links.check(self.root, ["README.md"], {"README.md -> gone.md"}), (1, []))
        _, missing = links.check(self.root, ["README.md"], {"README.md -> gone.md", "a.md -> b.md"})
        self.assertEqual(
            missing, ["a.md -> b.md resolves now, remove it from the expected-failure list"]
        )

    def test_repository_docs_pass_with_the_expected_list(self):
        checked, missing = links.check(
            links.ROOT, list(links.DOCS_ROOTS), links.EXPECTED_BROKEN_LINKS
        )
        self.assertGreater(checked, 0)
        self.assertEqual(missing, [])


if __name__ == "__main__":
    unittest.main()
