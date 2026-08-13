import json, sys
o = json.load(open('tests/extension-context-parity/oracle.json'))
print('type:', type(o).__name__)
if isinstance(o, dict):
    print('keys:', list(o.keys()))
    print('count:', len(o))
elif isinstance(o, list):
    print('count:', len(o))
    print('first elem type:', type(o[0]).__name__)
    if isinstance(o[0], dict):
        print('first keys:', list(o[0].keys()))
