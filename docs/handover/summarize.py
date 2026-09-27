# Condense `lab` run-mode JSON lines into one readable line per report.
# Usage: cargo run --release --bin lab -- ... | python3 docs/handover/summarize.py [tick,tick,...]
import sys, json
ticks = set(int(x) for x in sys.argv[1].split(',')) if len(sys.argv) > 1 else None
for l in sys.stdin:
    d = json.loads(l)
    if 'extinct_at' in d:
        print('  EXTINCT at', d['extinct_at'])
        continue
    if ticks is not None and d['tick'] not in ticks:
        continue
    e = d.get('energy_per_cell_tick', {})
    pt = d.get('per_tick', {})
    cl = ' '.join('%s=%d' % (k, v[0]) for k, v in d['class'].items() if v[0])
    print(' t=%-5d pop=%-6d lin=%-4d rows90=%-4d E=%-6.1f | %s | in: ph=%.2f th=%.2f sc=%.2f pr=%.2f metab=%.2f waste=%.2f | b=%.1f d=%.1f kills=%.2f atk=%.1f | phase=%s' % (
        d['tick'], d['pop'], d['lineages'], d['rows_90'], d['mean_energy'], cl,
        e.get('photo', 0), e.get('thermo', 0), e.get('scav', 0), e.get('predation', 0),
        e.get('metab', 0), e.get('cap_waste', 0), pt.get('births', 0), pt.get('deaths', 0),
        pt.get('kills', 0), pt.get('attacks', 0), d['phase']))
