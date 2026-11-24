import re
import shutil
import unittest
from pathlib import Path

import helm_yaml

FOUNDATION = Path(__file__).resolve().parent.parent / "overlays/local/components/foundation"
KUSTOMIZE = shutil.which("kustomize")
MIN_OPEN_FILES = 65536


@unittest.skipUnless(KUSTOMIZE, "kustomize is not installed")
class RustfsOpenFilesTest(unittest.TestCase):
    def test_rustfs_raises_its_soft_open_files_limit_before_exec(self):
        output = helm_yaml.process_runner.run_process(
            [KUSTOMIZE, "build", str(FOUNDATION)], check=True, capture_output=True, text=True
        ).stdout
        documents = helm_yaml.parse_documents(helm_yaml.split_documents(output))
        deployment = next(
            doc
            for doc in documents
            if doc and doc["kind"] == "Deployment" and doc["metadata"]["name"] == "rustfs"
        )
        container = deployment["spec"]["template"]["spec"]["containers"][0]
        self.assertEqual(container["command"][:2], ["/bin/sh", "-c"])
        script = container["command"][2]
        match = re.fullmatch(r"ulimit -Sn (\d+) && exec /etc/rustfs/entrypoint\.sh rustfs", script)
        self.assertIsNotNone(match, script)
        self.assertGreaterEqual(int(match.group(1)), MIN_OPEN_FILES)
        self.assertNotIn("args", container)


if __name__ == "__main__":
    unittest.main()
