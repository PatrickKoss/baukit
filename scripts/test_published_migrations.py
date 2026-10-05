from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]


class PublishedMigrationFormatTest(unittest.TestCase):
    def test_copied_migrations_have_no_whitespace_errors(self):
        migrations = sorted((ROOT / "rust/crates").glob("*/migrations/*.sql"))
        self.assertTrue(migrations)
        for path in migrations:
            with self.subTest(migration=path.relative_to(ROOT)):
                data = path.read_bytes()
                self.assertTrue(data.endswith(b"\n"))
                self.assertFalse(data.endswith(b"\n\n"), "blank line at EOF")
                for line in data.splitlines():
                    self.assertEqual(line, line.rstrip(), "trailing whitespace")
