#!/usr/bin/env python3

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
SCRIPT = Path(__file__).resolve().with_name("gitops-pins.py")
SPEC = importlib.util.spec_from_file_location("gitops_pins", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
pins = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(pins)

TAG = "0123456789ab"
DIGEST = "a" * 64
OTHER_DIGEST = "b" * 64
RELEASE = """spec:
  values:
    api:
      image:
        tag: "000000000000" # api image
    migration:
      image:
        tag: "000000000000" # migrate image
"""


class GitopsPinsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.release = self.root / "release.yaml"
        self.release.write_text(RELEASE, encoding="utf-8")

    def tearDown(self) -> None:
        self.directory.cleanup()

    def test_set_replaces_every_named_pin(self) -> None:
        pins.main(
            [
                "set",
                str(self.release),
                f"api={TAG}@sha256:{DIGEST}",
                f"migrate={TAG}@sha256:{OTHER_DIGEST}",
            ]
        )
        self.assertEqual(
            pins.read_pins(self.release),
            {"api": f"{TAG}@sha256:{DIGEST}", "migrate": f"{TAG}@sha256:{OTHER_DIGEST}"},
        )
        self.assertEqual(pins.shared_tag(self.release), TAG)

    def test_set_rejects_a_missing_or_unknown_image(self) -> None:
        with self.assertRaises(SystemExit):
            pins.main(["set", str(self.release), f"api={TAG}@sha256:{DIGEST}"])
        with self.assertRaises(SystemExit):
            pins.main(
                [
                    "set",
                    str(self.release),
                    f"api={TAG}@sha256:{DIGEST}",
                    f"worker={TAG}@sha256:{DIGEST}",
                ]
            )

    def test_set_rejects_a_pin_without_digest(self) -> None:
        with self.assertRaises(SystemExit):
            pins.main(["set", str(self.release), f"api={TAG}", f"migrate={TAG}"])

    def test_copy_refuses_placeholders_and_copies_pins(self) -> None:
        target = self.root / "production.yaml"
        target.write_text(RELEASE, encoding="utf-8")
        with self.assertRaises(SystemExit):
            pins.main(["copy", str(self.release), str(target)])
        pins.main(
            [
                "set",
                str(self.release),
                f"api={TAG}@sha256:{DIGEST}",
                f"migrate={TAG}@sha256:{DIGEST}",
            ]
        )
        pins.main(["copy", str(self.release), str(target)])
        self.assertEqual(
            target.read_text(encoding="utf-8"), self.release.read_text(encoding="utf-8")
        )

    def test_tag_rejects_mixed_tags(self) -> None:
        pins.main(
            [
                "set",
                str(self.release),
                f"api={TAG}@sha256:{DIGEST}",
                f"migrate=ba9876543210@sha256:{DIGEST}",
            ]
        )
        with self.assertRaises(SystemExit):
            pins.shared_tag(self.release)

    def test_check_encrypted_lists_plain_secret_files(self) -> None:
        (self.root / "plain.enc.yaml").write_text(
            "stringData:\n  KEY: REPLACE_ME\n", encoding="utf-8"
        )
        (self.root / "sealed.enc.yaml").write_text(
            "stringData: {}\nsops:\n  version: 3\n", encoding="utf-8"
        )
        self.assertEqual(pins.unencrypted_files(self.root), [self.root / "plain.enc.yaml"])
        with self.assertRaises(SystemExit):
            pins.main(["check-encrypted", str(self.root)])


if __name__ == "__main__":
    unittest.main()
