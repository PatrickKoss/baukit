import shutil
import subprocess


def run_process(
    command: list[str], *, check: bool = False, **options
) -> subprocess.CompletedProcess:
    executable = shutil.which(command[0])
    if executable is None:
        raise SystemExit(f"{command[0]} is not on PATH")
    argv = [executable, *command[1:]]
    return subprocess.run(argv, check=check, **options)  # noqa: S603 -- fixed program resolved by shutil.which, argv list, no shell
