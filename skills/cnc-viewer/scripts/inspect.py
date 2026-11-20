"""Small Python entry points for the native CNC Viewer executable."""
import copy
import json
import os
from pathlib import Path
import subprocess
import tempfile


def executable():
    override = os.environ.get('CNC_VIEWER_EXE')
    if override:
        return Path(override).resolve()
    skill = Path(__file__).resolve().parents[1]
    hint = skill / 'executable.txt'
    if hint.is_file():
        return Path(hint.read_text(encoding='utf-8').strip())
    root = skill.parents[1]
    for path in [
        root / 'output/desktop/CNC Viewer.exe',
        root / 'target/x86_64-pc-windows-msvc/release/cnc-viewer.exe',
        root / 'target/release/cnc-viewer.exe',
    ]:
        if path.is_file():
            return path
    raise FileNotFoundError('Build CNC Viewer with tools/build.ps1 or set CNC_VIEWER_EXE')


def inspect(path, job=None):
    """Return the report, including failed/incomplete checks. Invalid inputs raise."""
    return _inspect(str(Path(path).resolve()), job)


def inspect_text(text, job=None, *, filename='stdin.nc'):
    """Inspect source supplied in memory. Keep the filename for source selectors."""
    return _inspect('-', {**(job or {}), 'filename': filename}, text)


def _inspect(path, job, text=None):
    job = copy.deepcopy(job or {})
    def paths(config):
        if 'output' in config:
            config['output'] = str(Path(config['output']).resolve())
        if 'libraries' in config:
            config['libraries'] = [str(Path(p).resolve()) for p in config['libraries']]
    if "output" not in job and (job.get("artifacts") or job.get("views")):
        job["output"] = str(Path(".cnc/inspection").resolve())
    paths(job)
    for case in job.get('cases', []):
        paths(case)
    with tempfile.TemporaryDirectory(prefix='cnc-job-') as temporary:
        config = Path(temporary) / 'job.json'
        config.write_text(json.dumps(job), encoding='utf-8')
        process = subprocess.run(
            [str(executable()), 'inspect', path, str(config)],
            input=text, capture_output=True, text=True, encoding='utf-8', check=False,
        )
        if process.returncode not in (0, 2):
            raise RuntimeError(process.stdout.strip() or process.stderr.strip())
        return json.loads(process.stdout)


def movements(path, setup=None, *, select=None, operations=(), until=None, libraries=(), feed_only=False):
    """Return selected movements as a list. Use iter_movements for long selections."""
    return list(iter_movements(path, setup, select=select, operations=operations,
                               until=until, libraries=libraries, feed_only=feed_only))


def iter_movements(path, setup=None, *, select=None, operations=(), until=None, libraries=(), feed_only=False):
    """Yield selected moves with source visits and sampled motion vertices."""
    with tempfile.TemporaryDirectory(prefix='cnc-moves-') as temporary:
        report = inspect(path, {
            'setup': setup or {}, 'output': temporary, 'until': until,
            'libraries': list(libraries), 'operations': list(operations),
            'artifacts': {'movements': {'format': 'jsonl', 'select': select, 'feedOnly': feed_only}},
        })
        status = 'paused' if until is not None else 'complete'
        if report['status'] != status or report.get('artifactError'):
            raise RuntimeError(json.dumps(report))
        artifact = report['artifacts']['movements']
        with (Path(report['artifacts']['directory']) / artifact['file']).open(encoding='utf-8') as records:
            for line in records:
                yield json.loads(line)


def state(path, selector, setup=None, *, libraries=()):
    """Run through the selected block and return its machine and variable state."""
    with tempfile.TemporaryDirectory(prefix='cnc-state-') as temporary:
        report = inspect(path, {
            'setup': setup or {}, 'output': temporary, 'until': selector,
            'libraries': list(libraries), 'artifacts': {'trace': {'end': True, 'radius': 0}},
        })
        if report['status'] != 'paused' or report.get('artifactError'):
            raise RuntimeError(json.dumps(report))
        return json.loads((Path(temporary) / 'trace.jsonl').read_text(encoding='utf-8'))


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('file')
    parser.add_argument('job', nargs='?')
    args = parser.parse_args()
    job = json.loads(Path(args.job).read_text(encoding='utf-8-sig')) if args.job else None
    input_path = Path(args.file).resolve()
    if args.job:
        base = Path(args.job).resolve().parent
        os.chdir(base)
    print(json.dumps(inspect(input_path, job), indent=2))
