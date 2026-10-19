"""Partition every CPU sample once, using explicit caller-stack precedence."""
import collections
import hashlib
import json
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
locations = {}
samples = []
section = ''
for line in path.read_text().splitlines():
    if line in ['Samples:', 'Locations', 'Mappings']:
        section = line
        continue
    if section == 'Samples:':
        match = re.fullmatch(r'\s*(\d+)\s+(\d+):\s*([\d ]+)', line)
        if match:
            samples.append({'cpu_ns':int(match[1]), 'count':int(match[2]), 'locations':[int(x) for x in match[3].split()]})
        elif 'process name:[' in line:
            samples[-1]['process'] = re.search(r'process name:\[([^]]*)\]', line)[1]
            samples[-1]['thread'] = re.search(r'thread name:\[([^]]*)\]', line)[1]
    elif section == 'Locations':
        match = re.fullmatch(r'\s*(\d+): 0x[\da-f]+ (?:M=\d+ )?(.*?) (\S*):\d+:\d+ s=\d+(?:\(.*\))?', line)
        if match is not None:
            locations[int(match[1])] = {'function':match[2], 'file':match[3]}
        else:
            address = re.fullmatch(r'\s*(\d+): (0x[\da-f]+)(?: M=\d+)?\s*', line)
            assert address is not None, ('Unparsed location', line)
            locations[int(address[1])] = {'function':address[2], 'file':''}

assert samples
assert {s['process'] for s in samples} == {'quickwit'}
assert all(i in locations for s in samples for i in s['locations'])

rules = [
    ('Memory insertion / eviction / compaction', 'feuer_memory::store::MemoryCache::insert'),
    ('Other memory-cache paths', 'feuer_memory'),
    ('Storage paths outside memory-cache paths', 'feuer_storage'),
    ('Other Feuer / historian paths', 'feuer'),
    ('Tantivy paths outside cache paths', 'tantivy'),
    ('Other Quickwit query / search paths', r'quickwit_(query|search)'),
]
compiled = [(name, re.compile(pattern)) for name, pattern in rules]
counts = collections.Counter()
threads = collections.Counter()
inclusive = collections.Counter()
leaf_functions = collections.Counter()
examples = collections.defaultdict(list)
unknown_leaf_ns = 0
for sample in samples:
    functions = [locations[i]['function'] for i in sample['locations']]
    joined = '\n'.join(functions)
    unknown_leaf = functions[0] in ['Unknown Function', 'native', ''] or functions[0].startswith('0x')
    if unknown_leaf:
        unknown_leaf_ns += sample['cpu_ns']
    for name, pattern in compiled:
        if pattern.search(joined):
            category = name
            break
    else:
        category = 'Unresolved leaf without recognized subsystem' if unknown_leaf else 'Other named runtime / system paths'
    counts[category] += sample['cpu_ns']
    threads[sample['thread']] += sample['cpu_ns']
    leaf_functions[functions[0]] += sample['cpu_ns']
    for function in set(functions):
        inclusive[function] += sample['cpu_ns']
    if len(examples[category]) < 3:
        examples[category].append(functions)

total = sum(s['cpu_ns'] for s in samples)
assert sum(counts.values()) == total == sum(threads.values())
result = {
    'raw_text_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
    'sample_rows':len(samples), 'sample_count':sum(s['count'] for s in samples),
    'cpu_seconds':total / 1e9,
    'rule_order':rules,
    'method':'First matching stack rule wins. Samples are leaf-to-root. Unknown leaves retain caller-based attribution when a recognized subsystem is present. Categories and thread groups are separate disjoint partitions.',
    'unknown_leaf_cpu_share':unknown_leaf_ns / total,
    'categories':[{'category':name, 'cpu_seconds':value/1e9, 'percent':100*value/total} for name, value in counts.most_common()],
    'threads':[{'thread':name, 'cpu_seconds':value/1e9, 'percent':100*value/total} for name, value in threads.most_common()],
    'top_leaf_functions':[{'function':name, 'cpu_seconds':value/1e9, 'percent':100*value/total} for name, value in leaf_functions.most_common(30)],
    'inclusive_cache_functions':[{'function':name, 'cpu_seconds':value/1e9, 'percent':100*value/total} for name, value in inclusive.most_common() if 'feuer' in name][:30],
    'example_stacks':dict(examples),
}
output = path.with_name(path.stem + '-attribution.json')
output.write_text(json.dumps(result, indent=2))
print('Samples', result['sample_count'], 'CPU seconds', result['cpu_seconds'])
print('Unresolved leaf share', result['unknown_leaf_cpu_share'])
for group in ['categories', 'threads']:
    print(group)
    for row in result[group]: print(row)
print(output)
