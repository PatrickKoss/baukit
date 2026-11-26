import importlib.util
import re
import sys
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

ROOT = Path(__file__).resolve().parents[1]
UNIT_DIRECTORIES = ("rust/crates", "typescript/packages")
GUIDE_INDEX = "docs/guide/README.md"
DOCS_INDEX = "docs/README.md"
ADR_INDEX = "docs/adr/README.md"
ADR_FILE = re.compile(r"^\d{4}-.+\.md$")
ADR_STATUS = re.compile(
    r"^(Proposed|Accepted|Amended \d{4}-\d{2}-\d{2}|Superseded by \d{4}|Deferred)\b"
)
ADR_SECTIONS = ("## Enforced by", "## History")
UNITS = r"(?:rust |typescript |npm |library )?(?:crates?|packages?)\b"
ALLOWED_COUNT = re.compile(r"one[ -]+(?:crate|package) per\b", re.IGNORECASE)

# Temporary: later docs-overhaul phases fix these, and the last phase deletes the lists.
EXPECTED_INVENTORY_FAILURES = {
    "docs/guide/README.md: missing",
    "CLAUDE.md: layout misses typescript/packages/csp",
    "CLAUDE.md: layout misses typescript/packages/eslint-config",
    "CLAUDE.md: layout misses typescript/packages/purchases-core",
    "CLAUDE.md: layout misses typescript/packages/purchases-revenuecat",
    "CLAUDE.md: layout misses typescript/packages/type-guards",
}
EXPECTED_COUNT_FAILURES = {
    "README.md:10: Twenty-one Rust crates",
    "README.md:10: twenty-four npm packages",
    "README.md:40: one crate",
    "README.md:177: 19 library crates",
    "README.md:178: 21 packages",
    "README.md:194: eighteen library crates",
}
EXPECTED_ADR_FAILURES = {
    "docs/adr/README.md: missing",
    *(
        f"docs/adr/{name}: no {section}"
        for name in (
            "0001-product-experience-package-boundaries.md",
            "0002-integration-connector-contract.md",
            "0003-event-envelope.md",
            "0004-integration-client-contract.md",
            "0005-shared-application-navigation.md",
            "0006-browser-token-handling.md",
        )
        for section in ADR_SECTIONS
    ),
}
EXPECTED_GUIDE_FAILURES = {"docs/README.md: missing", "docs/guide/README.md: missing"}
EXPECTED_HOME_PATH_FAILURES = {
    "deploy/platform/flagger/README.md: /home/ path",
    "docs/quality-security/E3-products.md: /home/ path",
    "docs/quality-security/I-tenancy-ratelimit.md: /home/ path",
}

ONES = [
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
]
TENS = ("twenty", "thirty", "forty")


def number_words() -> list[str]:
    words = list(ONES)
    for ten in TENS:
        words.append(ten)
        words.extend(f"{ten}[- ]{one}" for one in ONES[:9])
    return sorted(words, key=len, reverse=True)


COUNT = re.compile(
    rf"\b(?:\d+|{'|'.join(number_words())})[ -]+{UNITS}",
    re.IGNORECASE,
)


def docs_roots() -> tuple[str, ...]:
    path = ROOT / "scripts/check_docs_links.py"
    spec = importlib.util.spec_from_file_location("check_docs_links", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.DOCS_ROOTS


def tracked_markdown(root: Path, pathspecs: tuple[str, ...]) -> list[str]:
    output = run_process(
        ["git", "-C", str(root), "ls-files", "-z", "--", *pathspecs],
        capture_output=True,
        check=True,
    ).stdout
    return sorted(name.decode() for name in output.split(b"\0") if name.lower().endswith(b".md"))


def read(root: Path, relative: str) -> str | None:
    path = root / relative
    return path.read_text(encoding="utf-8") if path.is_file() else None


def expand_braces(word: str) -> list[str]:
    match = re.search(r"\{([^{}]*)\}", word)
    if match is None:
        return [word]
    head, tail = word[: match.start()], word[match.end() :]
    return [
        expanded
        for option in match.group(1).split(",")
        for expanded in expand_braces(head + option + tail)
    ]


def layout_names(claude: str) -> set[str]:
    section = claude.partition("## Layout")[2]
    block = section.partition("```text")[2].partition("```")[0]
    names: set[str] = set()
    for word in re.findall(r"(?:[^\s,{]|\{[^}]*\})+", block):
        for name in expand_braces(word.strip("`")):
            names.add(f"baukit{name}" if name.startswith("-") else name)
    return names


def unit_directories(root: Path) -> list[str]:
    return sorted(
        f"{parent}/{child.name}"
        for parent in UNIT_DIRECTORIES
        if (root / parent).is_dir()
        for child in (root / parent).iterdir()
        if child.is_dir() and (child / "README.md").is_file()
    )


def mentions(text: str, name: str) -> bool:
    return re.search(rf"(?<![\w-]){re.escape(name)}(?![\w-])", text) is not None


def inventory_failures(root: Path) -> set[str]:
    failures: set[str] = set()
    guide = read(root, GUIDE_INDEX)
    claude = read(root, "CLAUDE.md") or ""
    layout = layout_names(claude)
    if guide is None:
        failures.add(f"{GUIDE_INDEX}: missing")
    for unit in unit_directories(root):
        name = unit.rsplit("/", 1)[1]
        if guide is not None and not mentions(guide, name):
            failures.add(f"{GUIDE_INDEX}: misses {unit}")
        if name not in layout:
            failures.add(f"CLAUDE.md: layout misses {unit}")
    return failures


def count_failures(root: Path) -> set[str]:
    failures: set[str] = set()
    pages = [
        "README.md",
        *(p.relative_to(root).as_posix() for p in root.glob("docs/guide/**/*.md")),
    ]
    for page in pages:
        for number, line in enumerate((read(root, page) or "").splitlines(), start=1):
            for match in COUNT.finditer(line):
                if not ALLOWED_COUNT.match(line, match.start()):
                    failures.add(f"{page}:{number}: {match.group()}")
    return failures


def adr_status(text: str) -> str:
    after = text.partition("\n## Status")[2]
    lines = [line.strip() for line in after.splitlines()[1:]]
    return next((line for line in lines if line), "")


def adr_failures(root: Path) -> set[str]:
    failures: set[str] = set()
    index = read(root, ADR_INDEX)
    if index is None:
        failures.add(f"{ADR_INDEX}: missing")
    adr_directory = root / "docs/adr"
    adrs = sorted(p for p in adr_directory.glob("*.md") if ADR_FILE.match(p.name))
    for path in adrs:
        relative = f"docs/adr/{path.name}"
        text = path.read_text(encoding="utf-8")
        if index is not None and path.name not in index:
            failures.add(f"{ADR_INDEX}: misses {path.name}")
        if not ADR_STATUS.match(adr_status(text)):
            failures.add(f"{relative}: bad status")
        for section in ADR_SECTIONS:
            if not re.search(rf"^{re.escape(section)}\s*$", text, re.MULTILINE):
                failures.add(f"{relative}: no {section}")
    return failures


def guide_failures(root: Path) -> set[str]:
    failures: set[str] = set()
    pages = sorted(p.name for p in (root / "docs/guide").glob("*.md") if p.name != "README.md")
    for index_path, prefix in ((GUIDE_INDEX, ""), (DOCS_INDEX, "guide/")):
        index = read(root, index_path)
        if index is None:
            if pages:
                failures.add(f"{index_path}: missing")
            continue
        for page in pages:
            if f"({prefix}{page}" not in index and f"(./{prefix}{page}" not in index:
                failures.add(f"{index_path}: misses guide/{page}")
    return failures


def home_path_failures(root: Path, roots: tuple[str, ...]) -> set[str]:
    return {
        f"{page}: /home/ path"
        for page in tracked_markdown(root, roots)
        if "/home/" in (read(root, page) or "")
    }


class ExpectedFailures(unittest.TestCase):
    def assert_only_expected(self, actual: set[str], expected: set[str]) -> None:
        unexpected = sorted(actual - expected)
        fixed = sorted(expected - actual)
        self.assertEqual(unexpected, [], "new docs inventory failures")
        self.assertEqual(fixed, [], "these pass now, remove it from the expected-failure list")


class DocsInventoryTests(ExpectedFailures):
    def test_every_crate_and_package_is_indexed(self):
        self.assert_only_expected(inventory_failures(ROOT), EXPECTED_INVENTORY_FAILURES)

    def test_readme_and_guide_state_no_unit_counts(self):
        self.assert_only_expected(count_failures(ROOT), EXPECTED_COUNT_FAILURES)

    def test_adrs_are_indexed_with_status_and_sections(self):
        self.assert_only_expected(adr_failures(ROOT), EXPECTED_ADR_FAILURES)

    def test_guide_pages_are_indexed(self):
        self.assert_only_expected(guide_failures(ROOT), EXPECTED_GUIDE_FAILURES)

    def test_public_docs_hold_no_home_paths(self):
        self.assert_only_expected(
            home_path_failures(ROOT, docs_roots()), EXPECTED_HOME_PATH_FAILURES
        )


class DocsInventoryCheckTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def write(self, relative: str, text: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def test_counts_match_digits_and_words_but_not_articles(self):
        self.write(
            "README.md",
            "Use 21 crates.\nThirty-seven npm packages.\nforty packages\n"
            "Add a crate. Each crate stands alone. One crate per concern.\n",
        )
        self.assertEqual(
            count_failures(self.root),
            {
                "README.md:1: 21 crates",
                "README.md:2: Thirty-seven npm packages",
                "README.md:3: forty packages",
            },
        )

    def test_counts_cover_nested_guide_pages(self):
        self.write("docs/guide/deep/page.md", "the two packages\n")
        self.assertEqual(count_failures(self.root), {"docs/guide/deep/page.md:1: two packages"})

    def test_layout_names_expand_braces_and_crate_prefixes(self):
        names = layout_names(
            "## Layout\n\n```text\nrust/crates/ baukit-auth, -config\n"
            "typescript/packages/ data-contracts{,-dexie}, auth-{web,node}\n```\n"
        )
        self.assertTrue(
            {"baukit-auth", "baukit-config", "data-contracts", "data-contracts-dexie", "auth-web"}
            <= names
        )

    def test_unindexed_units_fail(self):
        self.write("rust/crates/baukit-x/README.md", "# x\n")
        self.write("CLAUDE.md", "## Layout\n\n```text\nrust/crates/ baukit-y\n```\n")
        self.write(GUIDE_INDEX, "baukit-x-extra\n")
        self.assertEqual(
            inventory_failures(self.root),
            {
                "docs/guide/README.md: misses rust/crates/baukit-x",
                "CLAUDE.md: layout misses rust/crates/baukit-x",
            },
        )

    def test_adr_needs_index_status_and_sections(self):
        self.write(ADR_INDEX, "- [0001](0001-good.md)\n")
        self.write(
            "docs/adr/0001-good.md",
            "# ADR\n\n## Status\nAmended 2026-12-01.\n\n## Enforced by\nx\n\n## History\ny\n",
        )
        self.write("docs/adr/0002-bad.md", "# ADR\n\n## Status\n\nDone.\n\n## History\n")
        self.assertEqual(
            adr_failures(self.root),
            {
                "docs/adr/README.md: misses 0002-bad.md",
                "docs/adr/0002-bad.md: bad status",
                "docs/adr/0002-bad.md: no ## Enforced by",
            },
        )

    def test_adr_status_vocabulary(self):
        for status in ("Proposed", "Accepted, 2026-12-01.", "Superseded by 0007", "Deferred"):
            self.assertRegex(status, ADR_STATUS)
        for status in ("Amended", "Superseded", "accepted", "Rejected"):
            self.assertNotRegex(status, ADR_STATUS)

    def test_guide_pages_need_both_indexes(self):
        self.write("docs/guide/cli.md", "# CLI\n")
        self.write(GUIDE_INDEX, "- [CLI](cli.md)\n")
        self.assertEqual(guide_failures(self.root), {"docs/README.md: missing"})
        self.write(DOCS_INDEX, "- [Guide](guide/README.md)\n")
        self.assertEqual(guide_failures(self.root), {"docs/README.md: misses guide/cli.md"})

    def test_home_paths_fail_in_tracked_markdown(self):
        run_process(["git", "init", "-q", str(self.root)], check=True)
        self.write("docs/a.md", "see /home/someone/x\n")
        self.write("docs/b.md", "fine\n")
        run_process(["git", "-C", str(self.root), "add", "."], check=True)
        self.assertEqual(home_path_failures(self.root, ("docs",)), {"docs/a.md: /home/ path"})

    def test_fixed_expected_failure_asks_to_shrink_the_list(self):
        with self.assertRaisesRegex(AssertionError, "remove it from the expected-failure list"):
            ExpectedFailures().assert_only_expected(set(), {"README.md: gone"})


if __name__ == "__main__":
    sys.exit(unittest.main())
