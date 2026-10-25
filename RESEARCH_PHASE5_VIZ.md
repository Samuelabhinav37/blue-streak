# Blue Streak — Phase 5: Visualization & UX Architecture

> Research on treemap rendering, interaction patterns, and performance.
> Covers what WizTree does that we don't — specifically for DiskMap.

---

## Part 1 — Current DiskMap.tsx State

### What Works ✅
- SVG treemap with drill-down (click → view children)
- Stack-based breadcrumb history + "Back" button
- Right-side info panel (size, file count, depth, path)
- Size-based 4-color scheme (green → amber → red → violet)
- Text labels hidden below 60×30 px threshold

### What's Missing vs WizTree ❌

| Feature | Current | WizTree | Effort |
|---|---|---|---|
| Hover tooltips | None | Show name + size on hover | 1 hr |
| Clickable breadcrumb | Read-only | Jump to any level | 1 hr |
| File list panel | Summary only | Sortable virtualized table | 3 hrs |
| Right-click context menu | None | Open in Explorer, Copy path, Delete | 2 hrs |
| Canvas rendering (50K+ rects) | SVG only | Auto-switch at threshold | 4 hrs |
| Keyboard navigation | None | Arrows, Enter, Backspace | 2 hrs |
| "Other" bucket (tiny files) | None | Merge <2% into grey bucket | 1 hr |
| Continuous color gradient | 4 discrete | HSL hue by size | 1 hr |
| Search/filter by name | None | Real-time filter | 1 hr |
| Multi-select | None | Ctrl+Click, Shift+Click | 2 hrs |
| Export treemap as PNG | None | Save screenshot | 1 hr |
| Smooth zoom animation | Discrete drill | Animated transition | 3 hrs |

---

## Part 2 — SVG vs Canvas vs WebGL

| Metric | SVG (current) | Canvas 2D | WebGL |
|---|---|---|---|
| DOM nodes | 1 per rect | 1 total | 1 total |
| Render cost (50K rects) | 800 ms+ | 50 ms | 10 ms |
| Memory (idle) | ~100 MB | ~20 MB | ~20 MB |
| Hover/Click | Native events | Manual hit-test | Shader hit-test |
| Zoom/pan | SVG re-render | Redraw only | Matrix transform |

**Crossover point:** Canvas beats SVG at ~3,000 DOM nodes.

**Recommendation:**
- ≤ 5K rectangles: keep SVG (simpler, native events)
- > 5K rectangles: Canvas 2D (auto-switch)
- > 50K rectangles: WebGL or merge into "Other" buckets

**Canvas hit-testing** (replaces native SVG events):
```typescript
const hitTest = (mouseX: number, mouseY: number): TreemapNode | null => {
  for (const n of nodes) {
    if (mouseX >= n.x && mouseX <= n.x + n.width &&
        mouseY >= n.y && mouseY <= n.y + n.height) {
      return n;
    }
  }
  return null;
};
```

---

## Part 3 — Text Label Visibility

**Current thresholds:** `width > 60 && height > 30` — too conservative.

**WizTree observed:** Name hidden if `width < 40 || height < 25`.

**Recommended thresholds:**
```typescript
const showName = n.width > 40 && n.height > 25;
const showSize = n.width > 50 && n.height > 40;
const showCount = n.width > 30 && n.height > 20 && !showSize;

// Abbreviate long names
const abbrevMap: Record<string, string> = {
  'Documents': 'Docs', 'Downloads': 'DLs', 'Program Files': 'Prog',
  'System32': 'Sys32', 'AppData': 'App', 'Windows': 'Win',
};
```

---

## Part 4 — Color Schemes

**Current:** 4 discrete size buckets (loses info within ranges).

**Recommended:** Continuous HSL gradient + depth-based lightness:
```typescript
// Hue 0° (red/large) → 120° (green/small)
const hue = Math.max(0, 120 - (sizeBytes / maxSize) * 120);
const lightness = 50 + (depth * 5); // Deeper = slightly lighter
return `hsl(${hue}, 70%, ${lightness}%)`;
```

**Alternative — file-type colors (WinDirStat style):**
```typescript
const colorByExt: Record<string, string> = {
  '.exe': '#3b82f6', '.dll': '#8b5cf6', '.jpg': '#ec4899',
  '.mp4': '#ef4444', '.zip': '#f59e0b',
};
```

---

## Part 5 — Interaction Patterns

### Clickable Breadcrumb
```typescript
const jumpToLevel = (index: number) => {
  setTree(index < 0 ? rootNode : stack[index]);
  setStack(index < 0 ? [] : stack.slice(0, index));
  setSelected(null);
};
```

### Hover Tooltip
```typescript
const [hovered, setHovered] = useState<TreemapNode | null>(null);
const [mousePos, setMousePos] = useState({ x: 0, y: 0 });

// On SVG <g> elements:
onMouseEnter={(e) => { setHovered(n); setMousePos({x: e.clientX, y: e.clientY}); }}
onMouseLeave={() => setHovered(null)}
```

### Right-Click Context Menu
```typescript
const handleContextMenu = (e: React.MouseEvent, node: TreemapNode) => {
  e.preventDefault();
  setContextMenu({ x: e.clientX, y: e.clientY, node });
};
// Actions: Open in Explorer | Copy Path | Delete | Properties
```

### Keyboard Navigation
```typescript
if (e.key === 'Enter') handleSelect(selected);
if (e.key === 'Backspace') drillUp();
if (e.key === 'ArrowRight') setSelected(findNeighbor(selected, 'right'));
```

---

## Part 6 — File List Panel (Missing Feature)

**Why needed:** Clicking a rectangle should show sortable files inside it.

**Architecture:**
```typescript
function FileListPanel({ node, scanResults }: ...) {
  const [sortBy, setSortBy] = useState<'size' | 'name' | 'age'>('size');

  const files = useMemo(() =>
    scanResults
      .filter(f => f.path.startsWith(node.path))
      .sort((a, b) => sortBy === 'size' ? b.sizeBytes - a.sizeBytes : ...)
  , [node.path, scanResults, sortBy]);

  return (
    <List                           // react-window (already installed)
      rowCount={files.length}
      rowHeight={35}
      rowComponent={FileResultRow}
      rowProps={{ files }}
    />
  );
}
```

---

## Part 7 — "Other" Bucket for Tiny Files

**Problem:** System32 has 100K files — treemap becomes 100K unreadable dots.

**Solution:** Merge items < 2% of parent into an "Other" bucket:
```typescript
function mergeOthers(items: TreemapNode[], threshold = 0.02): TreemapNode[] {
  const parentSize = items.reduce((s, i) => s + i.sizeBytes, 0);
  const main = items.filter(i => i.sizeBytes > threshold * parentSize);
  const other = items.filter(i => i.sizeBytes <= threshold * parentSize);
  if (other.length === 0) return main;
  return [...main, {
    label: `Other (${other.length} items)`,
    sizeBytes: other.reduce((s, i) => s + i.sizeBytes, 0),
    color: '#475569', // neutral gray
    children: other,
  }];
}
```

---

## Part 8 — Progressive Rendering During Scan

**SpaceSniffer pattern:** Start rendering immediately, update every 200 ms.

```typescript
// In DiskMap.tsx
useEffect(() => {
  if (!isScanning || !scanProgress) return;
  const timer = setInterval(() => {
    // Re-query Tauri for partial treemap
    computePartialTreemap(scanProgress.scanned);
  }, 200);
  return () => clearInterval(timer);
}, [isScanning, scanProgress]);
```

---

## References

- Squarified Treemap Paper: Bruls et al. (2000) https://www.win.tue.nl/~vanwijk/stm.pdf
- d3-hierarchy squarify: https://github.com/d3/d3-hierarchy/blob/main/src/treemap/squarify.js
- WizTree UI Guide: https://diskanalyzer.com/guide
- WinDirStat Source: https://sourceforge.net/projects/windirstat/
- react-window docs: https://github.com/bvaughn/react-window
