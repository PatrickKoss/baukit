import shutil
import unittest
from pathlib import Path

import helm_yaml

FOUNDATION = Path(__file__).resolve().parent.parent / "overlays/local/components/foundation"
KUSTOMIZE = shutil.which("kustomize")
MIB = 1024 * 1024
UNITS = {"Mi": MIB, "Gi": 1024 * MIB}
COPY_CONCURRENCY = 16
COPY_PART_BYTES = 32 * MIB
BASELINE_BYTES = 128 * MIB
MIN_LIMIT_BYTES = COPY_CONCURRENCY * COPY_PART_BYTES + BASELINE_BYTES


def parse_quantity(quantity):
    for suffix, factor in UNITS.items():
        if quantity.endswith(suffix):
            return int(quantity[: -len(suffix)]) * factor
    raise ValueError(f"unsupported memory quantity {quantity}")


@unittest.skipUnless(KUSTOMIZE, "kustomize is not installed")
class RustfsMemoryTest(unittest.TestCase):
    def test_rustfs_limit_fits_the_documented_copy_fan_out(self):
        output = helm_yaml.process_runner.run_process(
            [KUSTOMIZE, "build", str(FOUNDATION)], check=True, capture_output=True, text=True
        ).stdout
        documents = helm_yaml.parse_documents(helm_yaml.split_documents(output))
        deployment = next(
            doc
            for doc in documents
            if doc and doc["kind"] == "Deployment" and doc["metadata"]["name"] == "rustfs"
        )
        limit = deployment["spec"]["template"]["spec"]["containers"][0]["resources"]["limits"]["memory"]
        self.assertGreaterEqual(parse_quantity(limit), MIN_LIMIT_BYTES, limit)

    def test_parse_quantity(self):
        self.assertEqual(parse_quantity("768Mi"), 768 * MIB)
        self.assertEqual(parse_quantity("1Gi"), 1024 * MIB)
        with self.assertRaises(ValueError):
            parse_quantity("1G")


if __name__ == "__main__":
    unittest.main()
