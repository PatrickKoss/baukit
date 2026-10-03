from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "release-train.sh"


class ReleaseTrainCorepackTest(unittest.TestCase):
    def test_release_train_uses_corepack_in_the_typescript_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "typescript").mkdir()
            (root / "typescript/.changeset").mkdir()
            (root / "rust").mkdir()
            (root / "rust/Cargo.toml").write_text(
                '[workspace.package]\nversion = "0.6.0"\n'
            )
            (root / "scripts").mkdir()
            self.executable(root / "scripts/check-version-coherence.py", "#!/bin/sh\nexit 0\n")
            binaries = root / "bin"
            binaries.mkdir()
            self.executable(
                binaries / "git",
                '#!/bin/sh\ncase "$1" in\n'
                'rev-parse) printf "%s\\n" "$TEST_REPO_ROOT";;\n'
                'status) exit 0;;\nesac\n',
            )
            self.executable(
                binaries / "corepack",
                '#!/bin/sh\nprintf "%s\\n" "$PWD" "$@" > "$TEST_COREPACK_LOG"\nexit 17\n',
            )
            self.executable(binaries / "pnpm", "#!/bin/sh\nexit 99\n")
            log = root / "corepack.log"
            environment = {
                **os.environ,
                "PATH": f"{binaries}:{os.environ['PATH']}",
                "TEST_REPO_ROOT": str(root),
                "TEST_COREPACK_LOG": str(log),
            }
            result = subprocess.run(
                ["bash", str(SCRIPT), "minor"],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 17, result.stderr)
            self.assertEqual(
                log.read_text().splitlines(),
                [str(root / "typescript"), "pnpm", "version-packages"],
            )
            self.assertIn(
                "Release the coordinated baukit 0.7.0 train.",
                (root / "typescript/.changeset/release-train.md").read_text(),
            )

    @staticmethod
    def executable(path: Path, contents: str) -> None:
        path.write_text(contents)
        path.chmod(0o755)


if __name__ == "__main__":
    unittest.main()
