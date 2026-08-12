#!/usr/bin/env python3
"""
Convert .pi.json (verbose per-cell frame evidence) to compact .pci.json format.

Usage: python3 scripts/compact-ui-evidence.py [tests/ui-parity/*.pi.json]
       or: python3 scripts/compact-ui-evidence.py --all
       or: python3 scripts/compact-ui-evidence.py --verify <file.pci.json>
"""

import json
import os
import sys
import glob

STYLE_FIELDS = ['foreground', 'background', 'bold', 'dim', 'italic', 'underline', 'inverse']

def style_tuple(cell):
    """Return a canonical tuple representing the cell's style (no text/wide)."""
    return tuple(cell[f] for f in STYLE_FIELDS)

def cell_default(cell):
    """True if cell has empty text and them-default style."""
    if cell['text']:
        return False
    if cell['bold'] or cell['dim'] or cell['italic'] or cell['underline'] or cell['inverse']:
        return False
    if cell['foreground'] != 'default' or cell['background'] != 'default':
        return False
    return True

def style_to_compact(s):
    """Convert style tuple to compact palette entry (short keys)."""
    return {
        'f': s[0],  # foreground
        'b': s[1],  # background  
        'l': 1 if s[2] else 0,  # bold
        'd': 1 if s[3] else 0,  # dim
        'i': 1 if s[4] else 0,  # italic
        'u': 1 if s[5] else 0,  # underline
        'v': 1 if s[6] else 0,  # inverse
    }

def compact_to_style(ps):
    """Convert compact style back to canonical style tuple."""
    return (
        ps.get('f', 'default'),
        ps.get('b', 'default'),
        bool(ps.get('l', 0)),
        bool(ps.get('d', 0)),
        bool(ps.get('i', 0)),
        bool(ps.get('u', 0)),
        bool(ps.get('v', 0)),
    )

def compress_file(pi_path):
    """Convert a single .pi.json file to compact format."""
    with open(pi_path) as f:
        import json
        frames = json.load(f)
    
    # Build palette: collect all unique style tuples
    styles_seen = set()
    for frame in frames:
        for cell in frame['cells']:
            st = style_tuple(cell)
            styles_seen.add(st)
    
    # Sort styles so proximity: default first
    # Default style index 0 = foreground=default, background=default, all false
    palette_entries = [
        ('default', 'default', False, False, False, False, False)
    ]
    palette_set = {palette_entries[0]}
    for st in styles_seen:
        if st not in palette_set:
            palette_set.add(st)
            palette_entries.append(st)
    
    palette = [style_to_compact(s) for s in palette_entries]
    
    # Build per-frame run-length encoded data
    compact_frames = []
    for frame in frames:
        cols = frame['columns']
        rows = frame['rows']
        cells = frame['cells']
        
        # Build runs for each row: runs of same-style cells
        runs = []  # [row, col, text, style_idx]
        
        for row in range(rows):
            col = 0
            while col < cols:
                cell = cells[row * cols + col]
                
                if cell_default(cell):
                    col += 1
                    continue
                
                st = style_tuple(cell)
                style_idx = None
                for idx, ps in enumerate(palette_entries):
                    if st == ps:
                        style_idx = idx
                        break
                
                if style_idx is None:
                    style_idx = 0  # fallback
                
                text = cell['text']
                
                # Handle wide characters: consume continuation cell
                # Wide cells store the char in the first cell, the second is continuation
                if cell['wide'] and not cell['wide_continuation']:
                    # The current cell has the text. Next cell is wide_continuation.
                    wide_text = cells[row * cols + col].get('text', '')
                    text = text or wide_text  # Take whichever cell has text
                    # Skip the continuation cell
                    col += 2
                elif cell['wide_continuation']:
                    # This is a continuation cell. Skip it, already counted.
                    col += 1
                    continue
                else:
                    col += 1
                
                # Merge same-styled adjacent cells into runs
                while col < cols:
                    next_cell = cells[row * cols + col]
                    if cell_default(next_cell):
                        break
                    next_style = style_tuple(next_cell)
                    if next_style != st:
                        break
                    if next_cell['wide'] or next_cell['wide_continuation']:
                        # Wide next makes new cell adds wide char
                        text += next_cell.get('text', '')
                        if next_cell['wide']:
                            col += 1  # skip continuation
                        col += 1
                        continue
                    text += next_cell['text']
                    col += 1
                
                if text or style_idx != 0:
                    runs.append([row, col - 1 - (col - len(text)), next_cell_text, style_idx])
                else:
                    runs.append([row, col, text, style_idx])
        
        compact_frame = {
            'n': frame['name'],
            'g': [frame['columns'], frame['rows']], 
            'c': [frame['cursor_row'], frame['cursor_column'], 1 if frame['cursor_visible'] else 0],
            'r': runs
        }
        compact_frames.append(compact_frame)
    
    return {
        'v': 1,
        'p': palette,
        'f': compact_frames
    }

import json

def decompress_to_frames(compact):
    """Reverse-compression compact → list of FrameSnapshot dicts."""
    palette = compact['p']
    # Build a cache from style index → style tuple for fast lookup
    style_cache = {}
    for idx, ps in enumerate(palette):
        style_cache[idx] = compact_style_to_cell(ps)
    
    def cell_default_filled(style_idx):
        s = style_cache.get(style_idx)
        if s is None:
            return True
        return (s['foreground'] == 'default' and s['background'] == 'default' and
                not s['bold'] and not s['dim'] and not s['italic'] and not s['underline'] and not s['inverse'])
    
    result = []
    for frame_data in compact['f']:
        n = frame_data['n']
        g = frame_data['g']
        cursor = frame_data['c']
        cols, rows = g[0], g[1]
        
        # Initialize all cells to default
        cells = [None] * (cols * rows)
        for idx in range(cols * rows):
            cells[idx] = default_cell()
        
        runs = frame_data.get('r', [])
        for run in runs:
            row, col, text, style_idx = run
            
            if row >= rows:
                continue
            
            style = style_cache.get(style_idx, default_style())
            
            for i, char in enumerate(text):
                cell_idx = row * cols + col + i
                if cell_idx >= len(cells):
                    break
                cells[cell_idx] = style.copy()
                cells[cell_idx]['text'] = char
        
        result.append({
            'name': n,
            'columns': cols,
            'rows': rows,
            'cursor_row': cursor[0],
            'cursor_column': cursor[1],
            'cursor_visible': bool(cursor[2]),
            'cells': cells
        })
    
    return result

def compact_style_to_cell(ps):
    return {
        'text': '',
        'wide': False,
        'wide_continuation': False,
        'foreground': ps.get('f', 'default'),
        'background': ps.get('b', 'default'),
        'bold': bool(ps.get('l', 0)),
        'dim': bool(ps.get('d', 0)),
        'italic': bool(ps.get('i', 0)),
        'underline': bool(ps.get('u', 0)),
        'inverse': bool(ps.get('v', 0)),
    }

def default_cell():
    return {
        'text': '',
        'wide': False,
        'wide_continuation': False,
        'foreground': 'default',
        'background': 'default',
        'bold': False,
        'dim': False,
        'italic': False,
        'underline': False,
        'inverse': False,
    }

def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        return
    
    if args[0] == '--all':
        pi_dir = os.path.dirname(os.path.abspath(__file__)).replace('/scripts', '')
        pi_dir = os.path.join(pi_dir, 'tests/ui-parity')
        files = sorted(glob.glob(os.path.join(pi_dir, '*.pi.json')))
    elif args[0] == '--verify':
        for path in args[0:]:
            verify_file(path)
        return
    else:
        files = args
    
    total_orig = 0
    total_compact = 0
    
    for pi_path in files:
        if not pi_path.endswith('.pi.json'):
            continue
        
        pci_path = pi_path.replace('.pi.json', '.pci.json')
        
        compact = compress_file(pi_path)
        
        with open(pci_path, 'w') as f:
            json.dump(compact, f, separators=(',', ':'))
        
        orig_size = os.path.getsize(pi_path)
        compact_size = os.path.getsize(pci_path)
        total_orig += orig_size
        total_compact += compact_size
        
        ratio = compact_size / orig_size
        reduction = (1 - ratio) * 100
        print(f"{os.path.basename(pi_path):45s} {orig_size/1024/1024:5.1f}M → {compact_size/1024/1024:5.1f}M  ({reduction:.0f}% reduction)")
    
    if len(files) > 1:
        print(f"{'TOTAL':45s} {total_orig/1024/1024:5.1f}M → {total_compact/1024/1024:5.2f}M  ({100-100*total_compact/total_orig:.0f}% reduction)")

def verify_file(path):
    """Verify that compact format round-trips to the original."""
    import json
    
    if not path.endswith('.pci.json'):
        return
    
    pi_path = path.replace('.pci.json', '.pi.json')
    if not os.path.exists(pi_path):
        print(f"Cannot verify: no .pi.json for {path}")
        return
    
    with open(pi_path) as f:
        original = json.load(f)
    with open(path) as f:
        compact = json.load(f)
    
    decompressed = decompress_to_frame_dicts(compact)
    
    # Compare frame by frame
    for i, (o, d) in enumerate(zip(original, decompressed)):
        if o['name'] != d['name']:
            print(f"MISMATCH frame {i}: name {o['name']} vs {d['name']}")
            return
        if o['columns'] != d['columns'] or o['rows'] != d['rows']:
            print(f"MISMATCH frame {i} ({o['name']}): geometry")
            return
        if o['cursor_row'] != d['cursor_row'] or o['cursor_column'] != d['cursor_column']:
            print(f"MISMATCH frame {i}: cursor")
            return
        
        cols = o['columns']
        for idx, (oc, dc) in enumerate(zip(o['cells'], d['cells'])):
            row, col = divmod(idx, cols)
            for key in ['text', 'bold', 'dim', 'italic', 'underline', 'inverse']:
                if oc.get(key) != dc.get(key):
                    print(f"  MISMATCH frame {i} ({o['name']}) cell ({row},{col}) {key}: {oc.get(key)} vs {dc.get(key)}")
                    return
            for key in ['foreground', 'background']:
                if oc.get(key) != dc.get(key):
                    print(f"CLEAN frame {i} ({o['name']}) cell ({row},{col}) {key}: {oc.get(key)} vs {dc.get(key)}")
                    return
            for key in ['wide', 'wide_continuation']:
                if oc.get(key, False) != dc.get(key, False):
                    print(f"  NOT OK frame {i} ({o['name']}) cell ({row},{col}) {key}: {oc.get(key)} vs {dc.get(key)}")
                    return
    
    orig_bytes = os.path.getsize(pi_path)
    compact_bytes = os.path.getsize(path)
    print(f"OK ({orig_bytes/1024/1024:.1f}M → {compact_bytes/1024/1024:.2f}M, {100-100*compact_bytes/orig_bytes:.0f}% reduction)")

if __name__ == '__main__':
    import glob
    if '--verify' in sys.argv[1:]:
        idx = sys.argv.index('--verify')
        files = sys.argv[idx+1:]
        for f in files:
            verify_file(f)
    elif '--all' in sys.argv:
        pi_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'tests')
        files = sorted(glob.glob(os.path.join(pi_dir, '**/*.pi.json'), recursive=True))
        main_args = [f for f in files if not f.endswith('.pci.json')]
        main()
    else:
        # Code properly so we can call from the command line
        if args[0] in ['--verify']:
            pass  # handled above
        else:
            main()