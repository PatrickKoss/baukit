import shutil
import subprocess


def start_process(command: list[str], **options) -> subprocess.Popen:
    executable = shutil.which(command[0])
    if executable is None:
        raise SystemExit(f"{command[0]} is not on PATH")
    argv = [executable, *command[1:]]
    return subprocess.Popen(argv, **options)  # noqa: S603 -- fixed program resolved by shutil.which, argv list, no shell


def run_process(
    command: list[str], *, check: bool = False, **options
) -> subprocess.CompletedProcess:
    stdin_data = options.pop("input", None)
    timeout = options.pop("timeout", None)
    if stdin_data is not None:
        options["stdin"] = subprocess.PIPE
    if options.pop("capture_output", False):
        options["stdout"] = subprocess.PIPE
        options["stderr"] = subprocess.PIPE
    with start_process(command, **options) as process:
        try:
            stdout, stderr = process.communicate(stdin_data, timeout=timeout)
        except BaseException:
            process.kill()
            raise
        returncode = process.poll()
    if check and returncode:
        raise subprocess.CalledProcessError(returncode, process.args, output=stdout, stderr=stderr)
    return subprocess.CompletedProcess(process.args, returncode, stdout, stderr)
