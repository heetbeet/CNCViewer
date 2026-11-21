"""Verify the packaged native EXE without developer runtimes on PATH."""
from pathlib import Path
import hashlib
import importlib.util
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time

exe = Path(sys.argv[1] if len(sys.argv) > 1 else 'output/desktop/CNC Viewer.exe').resolve()
skill_source = Path(__file__).resolve().parents[1] / 'skills/cnc-viewer'
started = time.perf_counter()
with tempfile.TemporaryDirectory(prefix='CNC native release ') as temporary:
    root = Path(temporary)
    binary = root / 'CNC Viewer.exe'
    shutil.copy2(exe, binary)
    environment = dict(os.environ, PATH='')
    def help_output(*args):
        result = subprocess.run([str(binary), *args], cwd=root, env=environment,
                                capture_output=True, text=True, encoding='utf-8', timeout=10)
        assert result.returncode == 0 and not result.stderr, result
        return result.stdout

    quick_help = help_output('--help')
    assert '--help agents' in quick_help and 'inspect FILE|- [JOB.json]' in quick_help
    for alias in ('-help', '/help', '--h', '-h', '/h', 'help', '-?', '/?'):
        assert help_output(alias) == quick_help, alias
    assert 'standard input' in help_output('inspect', '--help')
    assert 'Python helpers' in help_output('export-skill', '/?')
    guide = help_output('--help', 'agents')
    assert guide == help_output('/help', 'AGENTS.md') == help_output('--help=agents')
    preamble, *sections = re.split(r'^<!-- file: ([^\n]+) -->\n\n', guide, flags=re.MULTILINE)
    assert preamble == '' and sections[0] == 'AGENTS.md'
    embedded = {name: text.removesuffix('\n') for name, text in zip(sections[::2], sections[1::2])}
    expected = {path.relative_to(skill_source).as_posix(): path.read_text(encoding='utf-8')
                for path in skill_source.rglob('*.md')}
    assert embedded == expected

    source = root / 'part.nc'
    source.write_text('#101=0\nN10 G0 X[#101*10]Y0Z5\nG1Z-1F200\nG1X[#101*10+8]F600\nG0Z5\n#101=#101+1\nIF[#101 LT #100]GOTO10\nM30\n', encoding='utf-8-sig')
    counter = 0
    def run(job, code=0, program=source):
        global counter
        counter += 1
        path = root / f'job-{counter}.json'
        path.write_text(json.dumps(job), encoding='utf-8')
        result = subprocess.run([str(binary), 'inspect', str(program), str(path)], cwd=root, env=environment, capture_output=True, text=True, encoding='utf-8', timeout=30)
        report = json.loads(result.stdout)
        assert result.returncode == code, (result.returncode, report, result.stderr)
        return report
    setup = {'variables': {'100': 3}}
    report = run({'setup': setup, 'checks': {'expectedEnd': [28, 0, 5], 'maxFeed': 600}})
    assert report['summary']['moves'] == 12
    assert report['identity']['sourceSha256'] == hashlib.sha256(source.read_bytes()).hexdigest()
    report = run({'setup': setup, 'output': 'evidence', 'artifacts': {'movements': True, 'trace': {'step': 8, 'radius': 2}}, 'cutter': {'diameter': 8}, 'views': [{'name': 'top', 'angle': 'top', 'width': 512, 'height': 384, 'cutter': True, 'labels': [{'step': 4}]}]})
    movements = json.loads((root / 'evidence/movements.json').read_text())
    assert len(movements) == 12 and movements[2]['vertices'][-1] == [8, 0, -1]
    assert movements[2]['source'] == source.name
    assert len((root / 'evidence/trace.jsonl').read_text().splitlines()) == 5
    image = (root / 'evidence/top.png').read_bytes()
    assert image[:8] == b'\x89PNG\r\n\x1a\n'
    assert struct.unpack('>II', image[16:24]) == (512, 384)
    assert json.loads((root / 'evidence/top.json').read_text())['labels'][0]['point'] == [8, 0, -1]
    op = {'id': 'second', 'from': {'line': 2, 'visit': 2}, 'to': {'line': 5, 'visit': 2}}
    selected = {'operation': 'second'}
    visit = {'source': source.name, 'fromLine': 4, 'toLine': 4, 'visit': 2}
    report = run({'setup': setup, 'output': 'selected', 'operations': [op],
                  'motion': [{'id': 'op', 'select': selected}, {'id': 'visit', 'select': visit}],
                  'artifacts': {'movements': {'select': selected, 'format': 'jsonl'}},
                  'views': [{'name': 'visit', 'select': visit, 'labels': True, 'width': 128, 'height': 128}]})
    assert report['execution']['scope'] == 'full' and report['summary']['moves'] == 12
    assert report['motion'][0]['moves'] == 4 and report['motion'][1]['moves'] == 1
    assert report['motion'][1]['bounds'] == {'min': [10, 0, -1], 'max': [18, 0, -1]}
    assert report['artifacts']['movements']['records'] == 4
    selected_moves = [json.loads(line) for line in
                      (root / 'selected' / report['artifacts']['movements']['file']).read_text().splitlines()]
    assert selected_moves == movements[4:8]
    assert selected_moves[2]['visit'] == 2
    selected_view = json.loads((root / 'selected/visit.json').read_text())
    assert selected_view['segments'] == 1
    assert [label['point'] for label in selected_view['labels']] == [[18, 0, -1]]

    repeated = root / 'repeat.nc'
    repeated.write_text('G21 G90\nM98 P9000\nG0 Y4\nM98 P9000\nM30\n')
    sub = root / 'repeat_sub.nc'
    sub.write_text('O9000\nG91\nG1 X2 F120\nG90\nM99\n')
    sub_visit = {'source': sub.name, 'fromLine': 3, 'toLine': 3, 'visit': 2}
    report = run({'output': 'repeat', 'libraries': [sub.name],
                  'motion': [{'id': 'second', 'select': sub_visit}],
                  'artifacts': {'movements': {'select': sub_visit, 'format': 'jsonl'}}},
                 program=repeated)
    assert report['motion'][0]['bounds'] == {'min': [2, 4, 0], 'max': [4, 4, 0]}
    records = (root / 'repeat/movements.jsonl').read_text().splitlines()
    assert len(records) == 1 and json.loads(records[0])['visit'] == 2

    ramp = root / 'ramp.nc'
    ramp.write_text('G0 Z5\nG1 X10 Z-5 F100\nY4\nX8\nM30\n')
    polygon = [[6, 1], [9, 1], [9, 6], [7, 6], [7, 3], [6, 3]]
    band = {'id': 'band', 'polygon': polygon, 'z': [-3, -1],
            'select': {'fromLine': 2, 'toLine': 2}}
    report = run({'cutter': {'diameter': 4}, 'clearance': [
        band, dict(band, id='protected', z=[-5, -1], select=None, minimumClearance=0),
        dict(band, id='outside-band', z=[6, 8]),
    ]}, 2, ramp)
    measured, protected, empty = report['clearance']
    assert measured['status'] == 'measured' and measured['minimumClearance'] == -1
    assert measured['closest']['point'] == [6, 0, -1] and measured['closest']['regionPoint'] == [6, 1]
    assert protected['minimumClearance'] == -2 and protected['closest']['line'] == 4
    assert protected['firstViolation']['line'] == 2 and report['firstFailure']['check'] == 'cutterClearance'
    assert empty['status'] == 'empty' and empty['minimumClearance'] is None
    report = run({'cutter': {'diameter': 4}, 'clearance': [band],
                  'limits': {'clearanceTests': 1}}, 2, ramp)
    assert report['validation'] == 'incomplete' and not report['clearance'][0]['complete']
    assert report['firstFailure']['check'] == 'clearanceIncomplete'
    report = run({'clearance': [band]}, 2, ramp)
    assert 'diameter' in report['clearance'][0]['reason']
    report = run({'cutter': {'diameter': 4}, 'clearance': [
        dict(band, select={'fromLine': 99, 'toLine': 99}, minimumClearance=0)]}, 2, ramp)
    assert report['clearance'][0]['reason'] == 'Selection contains no executed feed segments'
    report = run({'cutter': {'diameter': 4}, 'clearance': [
        dict(band, polygon=[[0, 0], [2, 2], [0, 2], [2, 0]])]}, 1, ramp)
    assert 'polygon' in report['error']['message']
    report = run({'setup': setup, 'output': 'bad-view', 'views': [{'name': 'bad', 'step': 999999}]}, 2)
    assert report['status'] == 'complete' and report['summary']['moves'] == 12
    assert report['validation'] == 'incomplete' and report['artifactError']
    assert report['checks']['passed'] is False
    assert (root / 'bad-view/report.json').is_file()
    report = run({'setup': setup, 'until': {'line': 99}}, 2)
    assert report['firstFailure']['check'] == 'targetNotReached'
    report = run({'setup': setup, 'until': {'line': 4, 'visit': 2}})
    assert report['execution']['scope'] == 'prefix' and report['status'] == 'paused'
    report = run({'setup': setup, 'matrix': {'setup.variables.100': [3, 4]},
                  'checks': {'expectedEnd': [28, 0, 5]}, 'output': 'matrix'}, 2)
    assert [case['summary']['end'][0] for case in report['cases']] == [28, 38]
    assert report['firstFailure'] == 'case-002' and report['cases'][1]['reuse']['compiledSource']
    assert json.loads((root / 'matrix/matrix.json').read_text()) == report
    report = run({'setup': setup, 'limits': {'inputBytes': 32}}, 1)
    assert 'inputBytes' in report['error']['message']
    library = root / 'sub.nc'
    library.write_text('O9000\nM99\n')
    report = run({'setup': setup, 'libraries': [library.name],
                  'limits': {'inputBytes': len(source.read_bytes())}}, 1)
    assert 'inputBytes' in report['error']['message']
    report = run({'setup': setup, 'checks': {'maxFeed': 500}, 'output': 'focused',
                  'views': [{'finding': 'F001', 'fromStep': 3, 'radius': 2, 'width': 512,
                             'height': 384, 'camera': {'direction': [1, -1, 2]}}]}, 2)
    assert report['validation'] == 'failed'
    evidence = json.loads((root / 'focused/F001.json').read_text())
    assert evidence['step'] == 4 and evidence['fromStep'] == 3
    assert evidence['labels'][0]['id'] == 'F001'
    assert evidence['camera']['center'] == [8, 0, -1]
    assert evidence['segments'] == 1
    report = run({'setup': setup, 'checks': {'maxFeed': 500}, 'output': 'compact',
                  'limits': {'findings': 1}}, 2)
    assert report['findingCount'] == 3 and len(report['findings']) == 1
    assert report['findingsTruncated'] and report['findings'][0]['visit'] == 1
    assert json.loads((root / 'compact/report.json').read_text()) == report
    invalid = run({'setup': setup, 'checks': {'requireSpindle': 'true'}}, 1)
    assert 'requireSpindle' in invalid['error']['message']
    invalid = run({'setup': setup, 'checks': False}, 1)
    assert 'checks' in invalid['error']['message']
    arc = root / 'arc.nc'
    arc.write_text('G21 G17 G90\nG0 X1\nG3 X-1 Y0 I-1 J0 F120\nM30')
    report = run({'checks': {'allowedBounds': {'min': [-2, -2, -1], 'max': [2, 0.5, 1]}}}, 2, arc)
    assert report['firstFailure']['point'][1] > 0.5
    stationary = root / 'stationary.nc'
    stationary.write_text('M30\n')
    report = run({'checks': {'allowedBounds': {'min': [1, 0, 0], 'max': [0, 0, 0]}}}, 1, stationary)
    assert 'Bounds must be ordered' in report['error']['message']
    report = run({'setup': setup, 'cutter': {'diameter': 8},
                  'overlap': {'maxComparisons': 1}}, 2)
    assert report['validation'] == 'incomplete' and report['checks']['passed'] is False
    assert report['firstFailure']['check'] == 'overlapLimit'
    original = source.read_bytes()
    linked = root / 'linked'
    linked.mkdir()
    os.link(source, linked / 'top.png')
    report = run({'setup': setup, 'output': 'linked', 'views': [
        {'name': 'top', 'angle': 'top', 'width': 200, 'height': 140}]})
    assert source.read_bytes() == original
    assert (linked / 'top.png').read_bytes().startswith(b'\x89PNG')
    report = run({'setup': setup, 'cutter': {'diameter': 2}, 'output': 'stock',
                  'stock': {'bounds': {'min': [0, -2, -2], 'max': [30, 2, 0]},
                            'sections': [{'id': 'section', 'axis': 'y', 'at': 0},
                                         {'id': 'short', 'axis': 'y', 'at': 0, 'range': [5, 8]}]},
                  'artifacts': {'sections': True, 'stock': True}})
    assert '<polyline' in (root / 'stock/section.svg').read_text()
    assert report['stock']['complete'] is True
    assert 'heights' not in report['stock']
    assert len(json.loads((root / 'stock/stock.json').read_text())['heights']) == 120
    section = json.loads((root / 'stock/short.json').read_text())
    assert section['sampleSpacing'] == 1 and section['sampledAt'] == 0.5
    assert [point['coordinate'] for point in section['points']] == [5.5, 6.5, 7.5]
    report = run({'setup': setup, 'cutter': {'diameter': 2}, 'output': 'stock-target',
                  'stock': {'bounds': {'min': [0, -2, -2], 'max': [30, 2, 0]},
                            'target': {'z': -1}}}, 2)
    assert report['firstFailure']['check'] == 'stockRemaining'
    assert report['firstFailure']['point'][2] == 0
    tilted = root / 'tilted.nc'
    tilted.write_text('G21 G90\nG0 X20 Y10 Z5 B30\nG1 X30 B80 F100\nM30\n')
    tilted_setup = {'axisRoles': {'B': 'rotary'}, 'rotaryMode': 'head'}
    report = run({'setup': tilted_setup, 'output': 'tilt',
                  'views': [{'name': 'tilt', 'angle': 'front', 'cutter': {'diameter': 4},
                             'depthSupport': True}]}, 0, tilted)
    assert report['status'] == 'complete' and report['summary']['end'] == [30, 10, 5]
    assert (root / 'tilt/tilt.png').read_bytes().startswith(b'\x89PNG')
    alarm = root / 'alarm.nc'
    alarm.write_text('#3000=1 (Stop)\nM30\n')
    report = run({'cutter': {'diameter': 2},
                  'stock': {'bounds': {'min': [0, -2, -2], 'max': [30, 2, 0]},
                            'target': {'z': -1}}}, 2, alarm)
    assert report['validation'] == 'incomplete' and report['stock']['target']['status'] == 'failed'
    report = run({'setup': setup, 'artifacts': {'animation': True}}, 1)
    assert 'Unknown artifact: animation' in report['error']['message']
    export = root / 'skill'
    result = subprocess.run([str(binary), 'export-skill', str(export)], cwd=root, env=environment, capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, result.stdout
    for name, text in embedded.items():
        assert (export / name).read_text(encoding='utf-8') == text
    assert Path((export / 'executable.txt').read_text(encoding='utf-8')) == binary
    spec = importlib.util.spec_from_file_location('cnc_native_helper', export / 'scripts/inspect.py')
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    assert helper.inspect(source, {'setup': setup})['validation'] == 'passed'
    assert len(helper.movements(source, setup)) == 12
    assert list(helper.iter_movements(source, setup, select=visit)) == [movements[6]]
    assert helper.inspect_text('G0 X0\nG1 X3 F20\nM30')['summary']['end'] == [3, 0, 0]
    state = helper.state(source, {'line': 4, 'visit': 2}, setup)
    assert state['position'] == [18, 0, -1] and state['variables']['101'] == 1
    assert len(helper.movements(source, setup, until={'line': 4, 'visit': 2}, feed_only=True)) == 4
    result = subprocess.run([str(binary), 'export-skill', str(export)], cwd=root, env=environment, capture_output=True, text=True, timeout=10)
    assert result.returncode == 1, 'Existing skill files must not be overwritten'
print(json.dumps({'passed': True, 'jobs': counter, 'seconds': round(time.perf_counter() - started, 3), 'executableBytes': exe.stat().st_size}))
