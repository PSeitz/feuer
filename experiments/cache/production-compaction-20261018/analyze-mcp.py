"""Compute inclusive CPU shares from saved MCP responses, without API calls."""
import csv
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent
settings = json.loads((ROOT / 'settings.json').read_text())['profiling_mcp']
calls = json.loads((ROOT / 'raw/mcp-measured-windows-calls.json').read_text())
units = {'nanoseconds': 1e-9, 'microseconds': 1e-6, 'milliseconds': 1e-3, 'seconds': 1, 'minutes': 60}
rows = []
for version in [20, 21]:
    for category in ['all', 'query', 'memory', 'insert', 'storage']:
        filename = f'mcp-measured-{version}-{category}.json'
        call = next(c['arguments'] for c in calls if Path(c['output']).name == filename)
        assert [call['fromString'], call['toString']] == settings['windows'][str(version)]
        assert call['queryString'] == settings['query']
        assert call['profileType'] == settings['profile_type']
        response = json.loads((ROOT / 'raw' / filename).read_text())
        text = '\n'.join(c['text'] for c in response['content'] if c['type'] == 'text')
        data = json.loads(re.search(r'<profiling_data>\s*(.*?)\s*</profiling_data>', text, re.S)[1])
        matching, total = data['totalMatchingValue'], data['totalValue']
        assert matching['isPerMinute'] and total['isPerMinute']
        matching_seconds = matching['value'] * units[matching['unit']]
        total_seconds = total['value'] * units[total['unit']]
        rows.append(dict(version=version, stack_filter=call.get('frameRegexFilter', ''),
                         matching_cpu_seconds_per_minute=matching_seconds,
                         total_cpu_seconds_per_minute=total_seconds,
                         percent=100 * matching_seconds / total_seconds))
with (ROOT / 'results/profile-shares.csv').open('w') as output:
    writer = csv.DictWriter(output, fieldnames=list(rows[0]))
    writer.writeheader()
    writer.writerows(rows)
for row in rows:
    print(row)
