#!/usr/bin/env python3
"""Convert .pi.json → .pci.json (compact format) and verify round-trip."""

import json, os, sys, glob

FIELDS = ['foreground','background','bold','dim','italic','underline','inverse']
DEFAULT = ('default','default',False,False,False,False,False)

def ckey(c):
    return tuple(c[f] for f in FIELDS)

def is_default(c):
    return not c['text'] and ckey(c) == DEFAULT

def palette(frames):
    seen = {DEFAULT}
    pal = [DEFAULT]
    for fr in frames:
        for c in fr['cells']:
            k = ckey(c)
            if k not in seen:
                seen.add(k)
                pal.append(k)
    return pal

def pal_to_json(pal):
    """Convert palette list to JSON-friendly compact styles."""
    return [{'f':s[0],'b':s[1],
             'l':1 if s[2] else 0, 'd':1 if s[3] else 0,
             'i':1 if s[4] else 0, 'u':1 if s[5] else 0,
             'v':1 if s[6] else 0} for s in pal]

def convert_one(pi_path):
    with open(pi_path) as f:
        frames = json.load(f)
    pal = make_pairs(frames)
    pal_json = pal_to_compact(pal)

    out = {'v':1, 'p':pal_json, 'f':[]}

    for fr in frames:
        cols, rows = fr['columns'], fr['rows']
        cells = fr['cells']
        runs = []
        for r in range(rows):
            c = 0
            while c < cols:
                cell = cells[r * cols + c]
                if is_default(cell):
                    c += 1
                    continue
                k = ckey(cell)
                si = pal.index(k)
                txt = cell['text']
                start = c

                # skip continuation, handled by parent
                if cell.get('wide_continuation'):
                    c += 1; continue

                # handle wide char
                if cell.get('wide'):
                    c += 1
                    if c < cols:
                        txt += cells[r * cols + c].get('text','')
                        c += 1
                    continue

                c += 1

                # extend run: same style, non-empty, same row
                while c < cols:
                    nxt = cells[r * cols + c]
                    if nxt.get('wide_continuation'):
                        c += 1; continue
                    if nxt.get('wide'):
                        txt += nxt.get('text','')
                        c += 1
                        if c < cols:
                            txt += cells[r * cols + c].get('text','')
                            c += 1
                        continue
                    if is_default(nxt) or ckey(nxt) != k:
                        break
                    txt += nxt['text']
                    c += 1

                runs.append([r, start, txt, si])

        out.append({
            'n': fr['name'],
            'g': [fr['columns'], fr['rows']],
            'c': [fr['cursor_row'], fr['cursor_column'], 1 if fr['cursor_visible'] else 0],
            'r': runs
        })

    return {'v':1, 'p':pal_json, 'f':out}

def decompress(comp):
    pal = comp['p']
    # Build cell templates per style index
    temps = []
    for p in pal:
        temps.append({'text':'','wide':False,'wide_continuation':False,
                       'foreground':p['f'],'background':p['b'],
                       'bold':bool(p['l']),'dim':bool(p['d']),
                       'italic':bool(p['i']),'underline':bool(p['u']),
                       'inverse':bool(p['v'])})
    out = []
    for cfr in comp['f']:
        cols, rows = cfr['s']
        n = cfr['n']
        total = cols * rows
        cells = [dict(temps[0]) for _ in range(total)]

        for run in cfr['r']:
            row, col, txt, si = run
            if si >= len(temps): si = 0
            t = temps[si]
            for i, ch in enumerate(txt):
                idx = row * cols + col + i
                if idx >= total: break
                c = dict(t)
                c['text'] = ch
                cells[idx] = c

        out.append({
            'name': n,
            'columns': cols,
            'rows': rows,
            'cursor_row': cfr['c'][0],
            'cursor_column': cfr['c'][1],
            'cursor_visible': bool(cfr['c'][2]),
            'cells': cells
        })
    return out

def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        return

    is_verify = args[0] == '--verify'
    targets = args[1:] if is_verify or len(args) > 1 else None

    if is_verify:
        if not targets:
            targets = glob.glob('tests/ui-parity/*.pci.json')
        ok_all = True
        for pci in targets:
            pi = pci.replace('.pci.json','.pi.json')
            if not os.path.exists(pi):
                print(f"SKIP {pci}: no .pi.json")
                continue
            with open(pci) as f: comp = json.load(f)
            with open(pi) as f: orig = json.load(f)
            dec = decompress(comp)
            for i, (o, d) in enumerate(zip(orig, dec)):
                for k in ['name','columns','rows','cursor_row','cursor_column','cursor_visible']:
                    if o[k] != d[k]:
                        print(f"FAIL {pci}: frame {i} {k}")
                        ok_all = False
                for ci, (oc, dc) in enumerate(zip(o['cells'], d['cells'])):
                    for k in FIELDS:
                        if oc[k] != dc[k]:
                            r, cc = divmod(ci, o['columns'])
                            print(f"FAIL {pci}: frame {i} cell({r},{cc}) {k}: {oc[k]} vs {dc[k]}")
                            ok_all = False
            osz = os.path.getsize(pi)
            csz = os.path.getsize(pci)
            pct = 100 - (100*csz//osz) if osz else 0
            print(f"OK {os.path.basename(pci):45s} {osz//1024:>4}K → {csz//1024:>4}K ({pct}%)")
        sys.exit(0 if ok_all else 1)

    # Convert mode
    files = []
    for t in targets:
        if os.path.isdir(t):
            files.extend(glob.glob(os.path.join(t, '*.pi.json')))
        else:
            files.append(t)

    if not files:
        print("No .pi.json files found")
        return

    to = tc = 0
    for pi_path in files:
        pci_path = pi_path.replace('.pi.json','.pci.json')
        try:
            comp = convert_one(pi_path)
            with open(pci_path, 'w') as f:
                json.dump(comp, f, separators=(',',':'))
        except Exception as e:
            print(f"FAIL: {os.path.basename(pi_path)}: {e}")
            continue

        osz = os.path.getsize(pi_path)
        csz = os.path.getsize(pci_path)
        to += osz; tc += csz
        pct = 100 - (100*csz/osz) if osz else 0
        print(f"  {os.path.basename(pi_path):45s} {osz:>4}K → {csz:>4}K ({pct:.0f}%)")
)

    if len(sys.argv) > 1:
        print(f"  {'':45s} {to:>5}K → {tc:>5}K ({100-100*tc/to:.0f}%)")

if __name__ == '__main__':
    main()