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

    def test_link_to_a_missing_guide_page_fails(self):
        self.commit({"README.md": "[g](docs/guide/nope.md)\n"})
        self.assertEqual(
            links.check(self.root, ["README.md"]), (1, ["README.md:1 -> docs/guide/nope.md"])
        )

    def test_repository_docs_links_resolve(self):
        checked, missing = links.check(links.ROOT, list(links.DOCS_ROOTS))
        self.assertGreater(checked, 0)
        self.assertEqual(missing, [])


if __name__ == "__main__":
    unittest.main()
