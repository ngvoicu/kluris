
// ============================================================
// Module state — path-based navigation
// ============================================================
//
// `currentPath` is the array of folder names from the brain root.
// `[]` means "show the brain's direct children" (top-level lobes plus
// glossary.md when present). Each element is the
// next folder name as drilling proceeds. There is no depth ceiling.

let currentPath = [];

// Lobe palette — first 3 anchored to mockup colors so the demo brain
// (projects/infrastructure/knowledge) renders identically to the static HTML.
const lobePalette = ['#7bf7ff', '#ff8bd8', '#f8c76d', '#7df7b4', '#9ea9ff', '#ffa06f', '#b8f0c1', '#f2a8ff'];
const uniqueLobes = [...new Set(graph.nodes.map(n => n.lobe))]
  .filter(l => l && l !== 'root')
  .sort((a, b) => a.localeCompare(b));

function lobeColor(lobe) {
  const idx = uniqueLobes.indexOf(lobe);
  return lobePalette[(idx >= 0 ? idx : 0) % lobePalette.length];
}

// Top-level folder name for any node — first segment of node.path. Used
// for color inheritance on nested folders/leaves so a sublobe inside
// `projects/...` stays in the projects color family.
function topLobeOf(parts) {
  if (!parts.length) return null;
  const first = parts[0];
  // Files at the brain root are not under any lobe.
  if (parts.length === 1) return null;
  return uniqueLobes.includes(first) ? first : null;
}

const canvas = document.getElementById('mri-canvas');
const ctx = canvas.getContext('2d');
const searchInput = document.getElementById('search-input');
const resultsEl = document.getElementById('search-results');
const resultCountEl = document.getElementById('result-count');
const recentListEl = document.getElementById('recent-list');
const breadcrumbEl = document.getElementById('breadcrumb');
const stage = document.getElementById('stage');
const backBtn = document.getElementById('btn-back');
const forwardBtn = document.getElementById('btn-forward');

const neighbors = new Map();
for (const node of graph.nodes) neighbors.set(node.id, new Set());
for (const edge of graph.edges) {
  neighbors.get(edge.source)?.add(edge.target);
  neighbors.get(edge.target)?.add(edge.source);
}

let W = 0;
let H = 0;
const camera = { x: 0, y: 0, scale: 1 };

let needsDraw = true;
let pointer = { x: 0, y: 0 };
let selectedId = null;
let hoveredId = null;
let isPanning = false;
let dragMoved = false;
let lastPointer = { x: 0, y: 0 };

// Cached layout for the current path — boxes + sibling edges.
let currentBoxes = [];
let currentEdges = [];
let pathHistory = [[]];
let pathHistoryIndex = 0;

function requestDraw() { needsDraw = true; }

// ============================================================
// childrenOf — direct children of a path
// ============================================================
//
// Returns an array of:
//   - {kind: 'folder', name, path: [...path, name], topLobe, sampleNode?}
//   - {kind: 'leaf', name, path: [...path, name], node}
//
// Folders are sorted before leaves; both sorted by name. A folder's
// `topLobe` is the first segment of its path (used for color
// inheritance when nested two or more levels deep).
function childrenOf(path) {
  const folders = new Map();
  const leaves = [];
  for (const node of graph.nodes) {
    const parts = node.path.split('/');
    if (parts.length <= path.length) continue;
    let prefixOk = true;
    for (let i = 0; i < path.length; i++) {
      if (parts[i] !== path[i]) { prefixOk = false; break; }
    }
    if (!prefixOk) continue;
    if (parts.length === path.length + 1) {
      // Direct file child
      const name = parts[path.length];
      // Structural files are indexed for links/edge counts, but they stay off
      // the canvas so folders do not look like INDEX cards.
      const structural = node.type === 'map' || node.type === 'index' || node.type === 'brain';
      if (!structural) {
        leaves.push({
          kind: 'leaf',
          name,
          path: parts,
          node,
        });
      }
    } else {
      // Folder child (one or more levels deeper)
      const name = parts[path.length];
      if (!folders.has(name)) {
        folders.set(name, {
          kind: 'folder',
          name,
          path: [...path, name],
          topLobe: path.length === 0 && uniqueLobes.includes(name) ? name : (path[0] || null),
        });
      }
    }
  }
  const folderList = [...folders.values()].sort((a, b) => a.name.localeCompare(b.name));
  leaves.sort((a, b) => a.name.localeCompare(b.name));
  if (!folderList.length && !leaves.length && path.length > 0) {
    // Custom brains sometimes put useful neurons deeper without a neat
    // "sublobe" layer. If a folder has no direct visual children, flatten
    // its descendant neurons instead of showing an empty level.
    const prefix = path.join('/') + '/';
    const fallback = graph.nodes
      .filter(n => n.type === 'neuron' && n.path.startsWith(prefix))
      .sort((a, b) => a.path.localeCompare(b.path))
      .map(node => ({
        kind: 'leaf',
        name: node.path.slice(prefix.length),
        path: node.path.split('/'),
        node,
      }));
    if (fallback.length) return fallback;
  }
  return [...folderList, ...leaves];
}

// Count direct + descendant neurons inside a folder path so we can show
// a useful stats line on folder boxes ("12 neurons · 3 sublobes").
function folderStats(path) {
  let neurons = 0;
  const directSubs = new Set();
  for (const node of graph.nodes) {
    const parts = node.path.split('/');
    if (parts.length <= path.length) continue;
    let prefixOk = true;
    for (let i = 0; i < path.length; i++) {
      if (parts[i] !== path[i]) { prefixOk = false; break; }
    }
    if (!prefixOk) continue;
    if (node.type === 'neuron') neurons += 1;
    if (parts.length > path.length + 1) directSubs.add(parts[path.length]);
  }
  return { neurons, sublobes: directSubs.size };
}

// Human label for a folder — prefer the H1 of its map.md when present.
function folderTitle(path) {
  const target = path.join('/') + '/map.md';
  const mapNode = graph.nodes.find(n => n.path === target);
  if (mapNode && mapNode.title) return mapNode.title;
  const last = path[path.length - 1] || '';
  return last.replace(/-/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
}

// Kicker for a child item.
function kickerFor(item, depth) {
  if (item.kind === 'folder') return depth === 0 ? 'LOBE' : 'SUBLOBE';
  // Leaf
  const node = item.node;
  if (node.type === 'glossary') return 'GLOSSARY';
  if (node.type === 'brain') return 'INDEX';
  if (node.type === 'index') return 'INDEX';
  if (node.type === 'map') return 'INDEX';
  return 'NEURON';
}

// ============================================================
// aggregateEdgesAt — edges between siblings of the current path
// ============================================================
//
// For each non-parent edge, classify both endpoints as either "child X
// of currentPath" (folder OR leaf) or "outside" — pair-count for
// inside/inside. Edges that leave the current path are intentionally hidden:
// the canvas is a navigator first, and bottom "external relationship" pills
// quickly become noise in large customizable brains.
function aggregateEdgesAt(path, items) {
  // Build a lookup: nodeId -> child key (folder name or leaf name) or null
  // when the node lives outside this container entirely.
  const childOfNode = new Map();
  // Map child name -> the canonical item it corresponds to (so we can
  // place edges).
  const itemByName = new Map(items.map(it => [it.name, it]));
  for (const node of graph.nodes) {
    const parts = node.path.split('/');
    if (parts.length <= path.length) { childOfNode.set(node.id, null); continue; }
    let prefixOk = true;
    for (let i = 0; i < path.length; i++) {
      if (parts[i] !== path[i]) { prefixOk = false; break; }
    }
    if (!prefixOk) { childOfNode.set(node.id, null); continue; }
    const name = parts[path.length];
    childOfNode.set(node.id, itemByName.has(name) ? name : null);
  }
  const inside = new Map();
  for (const edge of graph.edges) {
    if (edge.type === 'parent') continue;
    const sk = childOfNode.get(edge.source);
    const tk = childOfNode.get(edge.target);
    const sIn = sk != null;
    const tIn = tk != null;
    if (sIn && tIn) {
      if (sk === tk) continue;  // self-edge inside same child
      const [a, b] = sk.localeCompare(tk) <= 0 ? [sk, tk] : [tk, sk];
      const cellKey = a + '||' + b;
      if (!inside.has(cellKey)) inside.set(cellKey, { a, b, forward: 0, reverse: 0 });
      const cell = inside.get(cellKey);
      if (sk === a) cell.forward += 1;
      else cell.reverse += 1;
    }
    // cross-boundary and outside/outside edges are omitted from the canvas
  }
  return [...inside.values()];
}

// ============================================================
// Layout — adaptive, pan-friendly grid for arbitrary growth
// ============================================================
function layoutGrid(items, viewport, opts) {
  const n = items.length;
  if (n === 0) return [];
  const W_ = viewport.width;
  const H_ = viewport.height;
  const baseW = (opts && opts.width) || 240;
  const baseH = (opts && opts.height) || 140;
  const density = n > 60 ? 'tiny' : n > 30 ? 'compact' : n > 14 ? 'condensed' : 'normal';
  const boxW = density === 'tiny' ? 180 : density === 'compact' ? 200 : density === 'condensed' ? 220 : baseW;
  const boxH = density === 'tiny' ? 76 : density === 'compact' ? 92 : density === 'condensed' ? 112 : baseH;
  if (n === 1) {
    return items.map(item => ({ ...item, x: W_ / 2, y: H_ / 2, w: boxW, h: boxH, density }));
  }
  if (n === 2) {
    const cellW = Math.min(W_ / 2, 320);
    const y = H_ / 2;
    return items.map((item, i) => ({
      ...item,
      x: W_ / 2 - cellW / 2 + i * cellW,
      y,
      w: boxW,
      h: boxH,
      density,
    }));
  }
  const minCellW = boxW + 42;
  const minCellH = boxH + 34;
  const viewportCols = Math.max(2, Math.floor(W_ / minCellW));
  const aspectCols = Math.ceil(Math.sqrt(n * (W_ / Math.max(1, H_))));
  const maxCols = n > 12 ? 4 : 6;
  const cols = Math.min(n, Math.max(2, Math.min(maxCols, viewportCols, aspectCols)));
  const rows = Math.ceil(n / cols);
  const cellW = Math.max(minCellW, Math.min(320, W_ / Math.min(cols, viewportCols)));
  const cellH = Math.max(minCellH, density === 'normal' ? 178 : density === 'condensed' ? 146 : 118);
  const worldW = Math.max(W_, cols * cellW);
  const worldH = Math.max(H_, rows * cellH);
  return items.map((item, i) => {
    const col = i % cols;
    const row = Math.floor(i / cols);
    const rowItems = (row === rows - 1) ? (n - row * cols) : cols;
    const rowOffset = (cols - rowItems) * cellW / 2;
    return {
      ...item,
      x: W_ / 2 - worldW / 2 + cellW / 2 + col * cellW + rowOffset,
      y: H_ / 2 - worldH / 2 + cellH / 2 + row * cellH,
      w: boxW,
      h: boxH,
      density,
    };
  });
}

// ============================================================
// drawC4Box + drawAggregateEdge — visual primitives (preserved)
// ============================================================
function drawC4Box(item, opts) {
  opts = opts || {};
  const { x, y, w, h } = item;
  const density = item.density || 'normal';
  const compact = density === 'compact' || density === 'tiny';
  const tiny = density === 'tiny';
  const left = x - w / 2;
  const top = y - h / 2;
  ctx.save();
  if (item.dim) ctx.globalAlpha = 0.30;
  // Body
  ctx.fillStyle = 'rgba(12, 21, 44, 0.96)';
  ctx.beginPath();
  ctx.roundRect(left, top, w, h, tiny ? 10 : 14);
  ctx.fill();
  // Border
  ctx.lineWidth = opts.hover ? 1.6 : 1.0;
  ctx.strokeStyle = opts.hover
    ? 'rgba(123, 167, 255, 0.42)'
    : 'rgba(123, 167, 255, 0.20)';
  ctx.stroke();
  // Color bar (4px) at the left edge, inset 14px top/bottom.
  ctx.beginPath();
  if (item.deprecated) {
    ctx.setLineDash([3, 4]);
    ctx.strokeStyle = item.color;
    ctx.lineWidth = 4;
    ctx.moveTo(left + 2, top + 12);
    ctx.lineTo(left + 2, top + h - 12);
    ctx.stroke();
    ctx.setLineDash([]);
  } else {
    ctx.fillStyle = item.color;
    ctx.roundRect(left, top + 12, 4, h - 24, [0, 4, 4, 0]);
    ctx.fill();
  }
  // Kicker
  ctx.fillStyle = '#8ba7d1';
  ctx.font = 'bold ' + (tiny ? 8 : 10) + 'px "SFMono-Regular", monospace';
  ctx.textAlign = 'left';
  ctx.fillText(String(item.kicker || '').toUpperCase(), left + 14, top + (tiny ? 18 : 22));
  // Title
  ctx.fillStyle = '#e9f1ff';
  const titleFont = tiny ? 12 : compact ? 13 : 15;
  ctx.font = 'bold ' + titleFont + 'px "Avenir Next", "Segoe UI", sans-serif';
  const titleStr = String(item.title || '');
  const titleMax = w - 24;
  ctx.fillText(_truncate(titleStr, titleMax, titleFont + 'px "Avenir Next", "Segoe UI", sans-serif'),
    left + 14, top + (tiny ? 38 : 44));
  // Separator
  if (!tiny) {
    ctx.strokeStyle = 'rgba(123, 167, 255, 0.15)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(left + 14, top + 54);
    ctx.lineTo(left + w - 14, top + 54);
    ctx.stroke();
  }
  // Path line (mono, muted)
  if (item.statsLine) {
    ctx.fillStyle = 'rgba(139, 167, 209, 0.65)';
    const statsFont = tiny ? 8 : 10;
    ctx.font = statsFont + 'px "SFMono-Regular", monospace';
    ctx.fillText(_truncate(item.statsLine, w - 24, statsFont + 'px "SFMono-Regular", monospace'),
      left + 14, top + h - (tiny ? 10 : 14));
  }
  // Search dim halo (yellow)
  if (item.matchHalo) {
    ctx.strokeStyle = 'rgba(255, 220, 100, 0.78)';
    ctx.lineWidth = 2.4;
    ctx.beginPath();
    ctx.roundRect(left - 2, top - 2, w + 4, h + 4, tiny ? 12 : 16);
    ctx.stroke();
  }
  ctx.restore();
}

function _truncate(s, maxPx, font) {
  ctx.save();
  ctx.font = font;
  if (ctx.measureText(s).width <= maxPx) {
    ctx.restore();
    return s;
  }
  let lo = 0, hi = s.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    const cand = s.slice(0, mid) + '…';
    if (ctx.measureText(cand).width <= maxPx) lo = mid + 1;
    else hi = mid;
  }
  ctx.restore();
  return s.slice(0, Math.max(0, lo - 1)) + '…';
}

function drawAggregateEdge(cell, boxByKey, opts) {
  opts = opts || {};
  const ba = boxByKey.get(cell.a);
  const bb = boxByKey.get(cell.b);
  if (!ba || !bb) return;
  const dim = opts.dim || false;
  const total = cell.forward + cell.reverse;
  const thickness = Math.max(1, Math.min(6, Math.log2(total + 1) * 1.4));
  const sx = ba.x;
  const sy = ba.y;
  const tx = bb.x;
  const ty = bb.y;
  const midY = (sy + ty) / 2;
  ctx.save();
  ctx.globalAlpha = dim ? 0.18 : 0.78;
  ctx.strokeStyle = opts.highlight ? '#ffdc64' : 'rgba(139, 167, 209, 0.78)';
  ctx.lineWidth = opts.highlight ? thickness + 1 : thickness;
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  ctx.beginPath();
  ctx.moveTo(sx, sy);
  ctx.lineTo(sx, midY);
  ctx.lineTo(tx, midY);
  ctx.lineTo(tx, ty);
  ctx.stroke();
  ctx.restore();
  const hasBothDirections = cell.forward > 0 && cell.reverse > 0;
  const label = (hasBothDirections ? '↔ ' : '→ ') + total;
  const pillX = (sx + tx) / 2;
  const pillY = midY;
  ctx.save();
  ctx.font = 'bold 10px "SFMono-Regular", monospace';
  const m = ctx.measureText(label);
  const padX = 8;
  const pillW = m.width + padX * 2;
  const pillH = 18;
  const pillLeft = pillX - pillW / 2;
  const pillTop = pillY - pillH / 2;
  ctx.fillStyle = 'rgba(8, 15, 32, 0.92)';
  ctx.beginPath();
  ctx.roundRect(pillLeft, pillTop, pillW, pillH, 6);
  ctx.fill();
  ctx.strokeStyle = 'rgba(123, 167, 255, 0.30)';
  ctx.lineWidth = 1;
  ctx.stroke();
  ctx.fillStyle = '#e9f1ff';
  ctx.textAlign = 'center';
  ctx.fillText(label, pillX, pillY + 3);
  ctx.restore();
  cell._midX = pillX;
  cell._midY = pillY;
  cell._w = pillW;
  cell._h = pillH;
}

// ============================================================
// drawCurrent — single render path for any depth
// ============================================================
function buildCurrentItems() {
  const depth = currentPath.length;
  const children = childrenOf(currentPath);
  const items = children.map(child => {
    if (child.kind === 'folder') {
      const top = depth === 0 && uniqueLobes.includes(child.name) ? child.name : (currentPath[0] || child.name);
      const color = uniqueLobes.includes(top) ? lobeColor(top) : lobePalette[0];
      return {
        ...child,
        kicker: depth === 0 ? 'LOBE' : 'SUBLOBE',
        title: folderTitle(child.path),
        statsLine: child.path.join('/'),
        color,
      };
    }
    // Leaf
    const node = child.node;
    const top = currentPath[0] || node.lobe;
    const color = uniqueLobes.includes(top) ? lobeColor(top) : '#9ea9ff';
    let leafColor = color;
    if (node.file_type === 'yaml') leafColor = '#9ea9ff';
    if (node.type === 'glossary') leafColor = '#ffc6f4';
    return {
      ...child,
      kicker: kickerFor(child, depth),
      title: node.title,
      statsLine: node.path,
      color: leafColor,
      deprecated: node.status === 'deprecated',
      nodeId: node.id,
    };
  });
  const rect = stage.getBoundingClientRect();
  const viewport = { width: rect.width || 900, height: rect.height || 600 };
  const opts = depth === 0 ? { width: 260, height: 140 } : { width: 240, height: 130 };
  return { items: layoutGrid(items, viewport, opts), viewport };
}

function leafMatchText(item) {
  if (item.kind !== 'leaf') return '';
  const node = item.node;
  return [
    node.title, node.path, node.file_name, node.lobe, node.type,
    node.file_type || '', ...(node.tags || []), node.excerpt || '',
  ].join(' ').toLowerCase();
}

function folderMatchText(item) {
  return (item.name + ' ' + folderTitle(item.path)).toLowerCase();
}

function visibleAggregateEdges(edges, itemCount) {
  const maxEdges = itemCount > 10 ? 0 : itemCount > 6 ? 4 : 8;
  return edges
    .slice()
    .sort((a, b) => (b.forward + b.reverse) - (a.forward + a.reverse))
    .slice(0, maxEdges);
}

function drawCurrent() {
  const { items, viewport } = buildCurrentItems();
  currentBoxes = items;
  const itemByName = new Map(items.map(it => [it.name, it]));
  currentEdges = visibleAggregateEdges(aggregateEdgesAt(currentPath, items), items.length);

  const query = searchInput.value.trim().toLowerCase();
  const matches = (item) => {
    if (!query) return false;
    const hay = item.kind === 'folder' ? folderMatchText(item) : leafMatchText(item);
    return hay.includes(query);
  };
  // Boundary frame for any non-root path so the user sees the container.
  if (currentPath.length > 0 && items.length) {
    const minX = Math.min(...items.map(b => b.x - b.w / 2));
    const maxX = Math.max(...items.map(b => b.x + b.w / 2));
    const minY = Math.min(...items.map(b => b.y - b.h / 2));
    const maxY = Math.max(...items.map(b => b.y + b.h / 2));
    const pad = 40;
    ctx.save();
    ctx.strokeStyle = 'rgba(123, 167, 255, 0.18)';
    ctx.setLineDash([6, 6]);
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.roundRect(minX - pad, minY - pad - 28, maxX - minX + pad * 2, maxY - minY + pad * 2 + 28, 18);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.font = 'bold 10px "SFMono-Regular", monospace';
    ctx.fillStyle = '#8ba7d1';
    const containerKicker = currentPath.length === 1 ? 'LOBE' : 'SUBLOBE';
    const containerLabel = folderTitle(currentPath).toUpperCase();
    ctx.fillText(containerKicker + ' · ' + containerLabel, minX - pad + 14, minY - pad - 12);
    ctx.restore();
  }
  // Edges between siblings (behind boxes)
  for (const cell of currentEdges) {
    const dim = query && !matches(itemByName.get(cell.a)) && !matches(itemByName.get(cell.b));
    drawAggregateEdge(cell, new Map(items.map(b => [b.name, b])), { dim });
  }
  // Boxes
  for (const item of items) {
    const isMatch = matches(item);
    drawC4Box({
      ...item,
      dim: query && !isMatch,
      matchHalo: query && isMatch,
    }, { hover: hoveredId === item.name });
  }
}

function hitTestCurrent(worldX, worldY) {
  for (const box of currentBoxes) {
    if (worldX >= box.x - box.w / 2 && worldX <= box.x + box.w / 2 &&
        worldY >= box.y - box.h / 2 && worldY <= box.y + box.h / 2) {
      return { kind: 'item', item: box };
    }
  }
  for (const cell of currentEdges) {
    if (cell._midX == null) continue;
    if (Math.abs(worldX - cell._midX) <= cell._w / 2 &&
        Math.abs(worldY - cell._midY) <= cell._h / 2) {
      return { kind: 'edge', a: cell.a, b: cell.b };
    }
  }
  return null;
}

// ============================================================
// Navigation
// ============================================================
function _samePath(a, b) {
  return a.length === b.length && a.every((part, i) => part === b[i]);
}

function _recordPath(path) {
  if (_samePath(pathHistory[pathHistoryIndex] || [], path)) return;
  pathHistory = pathHistory.slice(0, pathHistoryIndex + 1);
  pathHistory.push(path.slice());
  pathHistoryIndex = pathHistory.length - 1;
}

function navigateTo(path, fromHistory) {
  if (!fromHistory) _recordPath(path);
  currentPath = path.slice();
  selectedId = null;
  renderBreadcrumb();
  syncNavButtons();
  fitCurrentToView(true);
}

function syncNavButtons() {
  if (backBtn) {
    const canBack = pathHistoryIndex > 0 || currentPath.length > 0;
    backBtn.disabled = !canBack;
    backBtn.setAttribute('aria-disabled', canBack ? 'false' : 'true');
  }
  if (forwardBtn) {
    const canForward = pathHistoryIndex < pathHistory.length - 1;
    forwardBtn.disabled = !canForward;
    forwardBtn.setAttribute('aria-disabled', canForward ? 'false' : 'true');
  }
}

function goPathBack() {
  if (pathHistoryIndex > 0) {
    pathHistoryIndex -= 1;
    navigateTo(pathHistory[pathHistoryIndex], true);
  } else if (currentPath.length > 0) {
    navigateTo(currentPath.slice(0, -1));
  }
}

function goPathForward() {
  if (pathHistoryIndex >= pathHistory.length - 1) return;
  pathHistoryIndex += 1;
  navigateTo(pathHistory[pathHistoryIndex], true);
}

function renderBreadcrumb() {
  breadcrumbEl.innerHTML = '';
  // Root crumb
  const rootBtn = document.createElement('button');
  rootBtn.type = 'button';
  rootBtn.className = 'crumb' + (currentPath.length === 0 ? ' active' : '');
  rootBtn.dataset.level = 'brain';
  rootBtn.textContent = BRAIN_NAME;
  rootBtn.addEventListener('click', () => {
    if (currentPath.length > 0) navigateTo([]);
  });
  breadcrumbEl.appendChild(rootBtn);
  for (let i = 0; i < currentPath.length; i++) {
    const sep = document.createElement('span');
    sep.className = 'sep';
    sep.textContent = '›';
    breadcrumbEl.appendChild(sep);
    const isLast = i === currentPath.length - 1;
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'crumb' + (isLast ? ' active' : '');
    btn.dataset.level = 'path';
    btn.dataset.depth = String(i + 1);
    btn.textContent = folderTitle(currentPath.slice(0, i + 1));
    if (!isLast) {
      btn.addEventListener('click', () => navigateTo(currentPath.slice(0, i + 1)));
    }
    breadcrumbEl.appendChild(btn);
  }
}

// ============================================================
// Camera / pointer plumbing
// ============================================================
let cameraAnim = null;
function animateCamera(tx, ty, ts, duration) {
  const sx = camera.x, sy = camera.y, ss = camera.scale;
  const start = performance.now();
  if (cameraAnim) cancelAnimationFrame(cameraAnim);
  function step(now) {
    const t = Math.min(1, (now - start) / duration);
    const ease = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
    camera.x = sx + (tx - sx) * ease;
    camera.y = sy + (ty - sy) * ease;
    camera.scale = ss + (ts - ss) * ease;
    requestDraw();
    if (t < 1) cameraAnim = requestAnimationFrame(step);
    else cameraAnim = null;
  }
  cameraAnim = requestAnimationFrame(step);
}

function resize() {
  const rect = stage.getBoundingClientRect();
  W = canvas.width = Math.floor(rect.width * devicePixelRatio);
  H = canvas.height = Math.floor(rect.height * devicePixelRatio);
  canvas.style.width = rect.width + 'px';
  canvas.style.height = rect.height + 'px';
  ctx.setTransform(devicePixelRatio, 0, 0, devicePixelRatio, 0, 0);
  requestDraw();
}

function toWorld(clientX, clientY) {
  const rect = canvas.getBoundingClientRect();
  return {
    x: (clientX - rect.left - camera.x) / camera.scale,
    y: (clientY - rect.top - camera.y) / camera.scale,
  };
}

function fitCurrentToView(instant) {
  const { items } = buildCurrentItems();
  const rect = stage.getBoundingClientRect();
  if (!items.length) {
    if (instant) {
      camera.x = 0;
      camera.y = 0;
      camera.scale = 1;
      requestDraw();
    } else {
      animateCamera(0, 0, 1, 220);
    }
    return;
  }
  const pad = 72;
  const minX = Math.min(...items.map(b => b.x - b.w / 2)) - pad;
  const maxX = Math.max(...items.map(b => b.x + b.w / 2)) + pad;
  const minY = Math.min(...items.map(b => b.y - b.h / 2)) - pad;
  const maxY = Math.max(...items.map(b => b.y + b.h / 2)) + pad;
  const boundsW = Math.max(1, maxX - minX);
  const boundsH = Math.max(1, maxY - minY);
  const scale = Math.min(rect.width / boundsW, rect.height / boundsH);
  const clampedScale = Math.min(1.12, Math.max(0.24, scale));
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  const tx = rect.width / 2 - cx * clampedScale;
  const ty = rect.height / 2 - cy * clampedScale;
  if (instant) {
    if (cameraAnim) { cancelAnimationFrame(cameraAnim); cameraAnim = null; }
    camera.x = tx;
    camera.y = ty;
    camera.scale = clampedScale;
    requestDraw();
  } else {
    animateCamera(tx, ty, clampedScale, 240);
  }
}

// ============================================================
// Master draw + animation loop
// ============================================================
function draw() {
  const rect = canvas.getBoundingClientRect();
  ctx.clearRect(0, 0, rect.width, rect.height);
  ctx.save();
  ctx.translate(camera.x, camera.y);
  ctx.scale(camera.scale, camera.scale);
  drawCurrent();
  ctx.restore();
}

function loop() {
  if (needsDraw) {
    draw();
    needsDraw = false;
  }
  requestAnimationFrame(loop);
}

// ============================================================
// Pointer events
// ============================================================
canvas.addEventListener('pointerdown', event => {
  canvas.setPointerCapture(event.pointerId);
  dragMoved = false;
  lastPointer = { x: event.clientX, y: event.clientY };
  isPanning = true;
  canvas.classList.add('dragging');
});

canvas.addEventListener('pointermove', event => {
  pointer = { x: event.clientX, y: event.clientY };
  const world = toWorld(event.clientX, event.clientY);
  const hit = hitTestCurrent(world.x, world.y);
  if (hit && hit.kind === 'item') {
    hoveredId = hit.item.name;
  } else {
    hoveredId = null;
  }
  const dx = event.clientX - lastPointer.x;
  const dy = event.clientY - lastPointer.y;
  if (Math.abs(dx) > 1 || Math.abs(dy) > 1) dragMoved = true;
  lastPointer = { x: event.clientX, y: event.clientY };
  if (isPanning) {
    camera.x += dx;
    camera.y += dy;
  }
  requestDraw();
});

canvas.addEventListener('pointerup', event => {
  isPanning = false;
  canvas.classList.remove('dragging');
  if (dragMoved) return;
  const world = toWorld(event.clientX, event.clientY);
  const hit = hitTestCurrent(world.x, world.y);
  if (!hit) return;
  if (hit.kind === 'item') {
    const item = hit.item;
    if (item.kind === 'folder') {
      navigateTo([...currentPath, item.name]);
    } else if (item.kind === 'leaf') {
      selectedId = item.node.id;
      openModal(item.node);
    }
  }
  // Edge clicks are no-ops in path-based navigation. Related files remain
  // reachable via search, recent, the file tree, and modal links.
});

canvas.addEventListener('pointerleave', () => {
  hoveredId = null;
  isPanning = false;
  canvas.classList.remove('dragging');
  requestDraw();
});

canvas.addEventListener('wheel', event => {
  event.preventDefault();
  const rect = canvas.getBoundingClientRect();
  const mouseX = event.clientX - rect.left;
  const mouseY = event.clientY - rect.top;
  const worldX = (mouseX - camera.x) / camera.scale;
  const worldY = (mouseY - camera.y) / camera.scale;
  const nextScale = Math.min(2.6, Math.max(0.35, camera.scale * (event.deltaY < 0 ? 1.08 : 0.92)));
  camera.x = mouseX - worldX * nextScale;
  camera.y = mouseY - worldY * nextScale;
  camera.scale = nextScale;
  requestDraw();
}, { passive: false });

// ============================================================
// Search / filter
// ============================================================
function refreshSearch() {
  const query = searchInput.value.trim().toLowerCase();
  resultsEl.innerHTML = '';
  if (!query) {
    resultCountEl.textContent = '';
    requestDraw();
    return;
  }
  const matches = { Neurons: [], Folders: [] };
  // Folders — walk all node paths to extract folder names matching query.
  const seenFolders = new Set();
  for (const node of graph.nodes) {
    const parts = node.path.split('/');
    for (let i = 0; i < parts.length - 1; i++) {
      const folderPath = parts.slice(0, i + 1);
      const key = folderPath.join('/');
      if (seenFolders.has(key)) continue;
      seenFolders.add(key);
      const name = folderPath[folderPath.length - 1];
      const title = folderTitle(folderPath);
      if (name.toLowerCase().includes(query) || title.toLowerCase().includes(query)) {
        matches.Folders.push({ kind: 'folder', path: folderPath, name, title });
      }
    }
  }
  for (const node of graph.nodes) {
    if (node.type !== 'neuron' && node.type !== 'glossary') continue;
    const hay = [
      node.title, node.path, node.file_name, node.lobe, node.type, node.file_type || '',
      ...(node.tags || []), node.excerpt || '',
    ].join(' ').toLowerCase();
    if (hay.includes(query)) matches.Neurons.push({ kind: 'neuron', node });
  }
  const total = matches.Neurons.length + matches.Folders.length;
  resultCountEl.textContent = total + ' result' + (total === 1 ? '' : 's');
  for (const group of ['Folders', 'Neurons']) {
    if (!matches[group].length) continue;
    const heading = document.createElement('div');
    heading.className = 'panel-section-header';
    heading.style.padding = '4px 4px';
    heading.innerHTML = '<h3>' + group + ' (' + matches[group].length + ')</h3>';
    resultsEl.appendChild(heading);
    for (const m of matches[group].slice(0, 12)) {
      const card = document.createElement('button');
      card.type = 'button';
      card.className = 'result-card';
      let title = '';
      let meta = '';
      let path = '';
      if (m.kind === 'folder') {
        title = m.title;
        meta = m.path.length === 1 ? 'lobe' : 'sublobe · ' + m.path.slice(0, -1).join('/');
        path = m.path.join('/');
      } else {
        title = m.node.title;
        meta = m.node.type + ' · ' + (m.node.sublobe || m.node.lobe);
        path = m.node.path;
      }
      card.innerHTML =
        '<div class="result-title">' + escapeHtml(title) + '</div>' +
        '<div class="result-meta">' + escapeHtml(meta) + '</div>' +
        (path ? '<div class="result-path">' + escapeHtml(path) + '</div>' : '');
      card.addEventListener('click', () => {
        if (m.kind === 'folder') {
          navigateTo(m.path);
        } else {
          // Drop into the leaf's parent folder, then open modal.
          const parts = m.node.path.split('/');
          navigateTo(parts.slice(0, -1));
          selectedId = m.node.id;
          openModal(m.node);
        }
      });
      resultsEl.appendChild(card);
    }
  }
  requestDraw();
}

function searchEnterJump() {
  const query = searchInput.value.trim().toLowerCase();
  if (!query) return;
  const cards = resultsEl.querySelectorAll('.result-card');
  if (cards.length === 1) cards[0].click();
}

function renderRecent() {
  recentListEl.innerHTML = '';
  const neurons = graph.nodes
    .filter(n => n.type === 'neuron' && n.updated)
    .sort((a, b) => String(b.updated).localeCompare(String(a.updated)))
    .slice(0, 5);
  for (const n of neurons) {
    const row = document.createElement('button');
    row.type = 'button';
    row.className = 'recent-row';
    row.innerHTML =
      '<span class="label">' + escapeHtml(n.title) + '</span>' +
      '<span class="ago">' + escapeHtml(_relAgo(n.updated)) + '</span>';
    row.addEventListener('click', () => {
      const parts = n.path.split('/');
      navigateTo(parts.slice(0, -1));
      selectedId = n.id;
      openModal(n);
    });
    recentListEl.appendChild(row);
  }
}

function _relAgo(dateStr) {
  if (!dateStr) return '';
  const t = Date.parse(dateStr);
  if (isNaN(t)) return '';
  const diff = Date.now() - t;
  const days = Math.round(diff / 86400000);
  if (days < 1) return 'today';
  if (days < 60) return days + 'd';
  const months = Math.round(days / 30);
  if (months < 24) return months + 'mo';
  return Math.round(days / 365) + 'y';
}

// ============================================================
// File tree (left sidebar) — clicking a leaf navigates to its parent
// folder and opens the modal.
// ============================================================
const treeRoot = { folders: new Map(), files: [] };
let treeBuilt = false;
function buildFileTree() {
  treeRoot.folders = new Map();
  treeRoot.files = [];
  const entries = graph.nodes
    .filter(n => (n.type === 'neuron' || n.type === 'glossary') && n.path)
    .sort((a, b) => a.path.localeCompare(b.path));
  for (const node of entries) {
    const parts = node.path.split('/');
    let cursor = treeRoot;
    for (let i = 0; i < parts.length - 1; i++) {
      const part = parts[i];
      if (!cursor.folders.has(part)) {
        cursor.folders.set(part, { folders: new Map(), files: [] });
      }
      cursor = cursor.folders.get(part);
    }
    cursor.files.push(node);
  }
  treeBuilt = true;
}
const treeCollapsed = {
  'modal-tree': new Set(),
  'panel-tree': new Set(),
};
function renderTreeFolder(name, folder, pathSoFar, targetId) {
  const full = pathSoFar ? pathSoFar + '/' + name : name;
  const collapsed = (treeCollapsed[targetId] || new Set()).has(full);
  const caret = collapsed ? '▸' : '▾';
  const subFolderHtml = [...folder.folders.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([n, f]) => renderTreeFolder(n, f, full, targetId))
    .join('');
  const fileHtml = folder.files
    .sort((a, b) => a.title.localeCompare(b.title))
    .map(node => '<div class="modal-tree-file" data-tree-node="' + node.id +
      '" title="' + escapeHtml(node.path) + '"><span class="icon">📄</span>' +
      escapeHtml(node.title) + '</div>')
    .join('');
  const topLobeColor = pathSoFar === '' && uniqueLobes.includes(name)
    ? ' style="--bar:' + lobeColor(name) + '"'
    : '';
  return (
    '<div class="modal-tree-folder' + (collapsed ? ' collapsed' : '') +
      '" data-tree-folder="' + escapeHtml(full) + '"' + topLobeColor + '>' +
    '<div class="modal-tree-folder-label"><span class="caret">' + caret +
      '</span><span class="icon">📁</span>' + escapeHtml(name) + '</div>' +
    '<div class="modal-tree-children">' + subFolderHtml + fileHtml + '</div>' +
    '</div>'
  );
}
function renderFileTree(activeNodeId, targetId) {
  if (!targetId) targetId = 'modal-tree';
  if (!treeBuilt) buildFileTree();
  const treeEl = document.getElementById(targetId);
  if (!treeEl) return;
  if (!treeCollapsed[targetId]) treeCollapsed[targetId] = new Set();
  const topFolders = [...treeRoot.folders.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([n, f]) => renderTreeFolder(n, f, '', targetId))
    .join('');
  const topFiles = treeRoot.files
    .sort((a, b) => a.title.localeCompare(b.title))
    .map(node => '<div class="modal-tree-file" data-tree-node="' + node.id +
      '" title="' + escapeHtml(node.path) + '"><span class="icon">📄</span>' +
      escapeHtml(node.title) + '</div>')
    .join('');
  treeEl.innerHTML = topFolders + topFiles;
  if (activeNodeId != null) {
    for (const el of treeEl.querySelectorAll('.modal-tree-file')) {
      if (Number(el.dataset.treeNode) === activeNodeId) el.classList.add('active');
    }
  }
  for (const el of treeEl.querySelectorAll('.modal-tree-folder-label')) {
    el.addEventListener('click', () => {
      const folder = el.closest('.modal-tree-folder');
      const path = folder?.dataset.treeFolder;
      if (!path) return;
      const collapsedSet = treeCollapsed[targetId];
      if (collapsedSet.has(path)) {
        collapsedSet.delete(path);
        folder.classList.remove('collapsed');
      } else {
        collapsedSet.add(path);
        folder.classList.add('collapsed');
      }
    });
  }
  for (const el of treeEl.querySelectorAll('.modal-tree-file')) {
    el.addEventListener('click', () => {
      const target = graph.nodes.find(n => n.id === Number(el.dataset.treeNode));
      if (target) {
        selectedId = target.id;
        const parts = target.path.split('/');
        navigateTo(parts.slice(0, -1));
        openModal(target);
      }
    });
  }
}

// ============================================================
// Helpers — escapeHtml, markdown / yaml renderers
// ============================================================
function escapeHtml(value) {
  return String(value || '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}

function renderInline(text) {
  let html = escapeHtml(text);
  html = html.replace(/`([^`]+)`/g, '<code>$1</code>');
  html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  html = html.replace(/(^|[^*])\*([^*]+)\*/g, '$1<em>$2</em>');
  html = html.replace(
    /\[([^\]]+)\]\(([^)\s]+)\)/g,
    '<a class="md-link" href="#" data-md-link="$2">$1</a>'
  );
  return html;
}
function _isTableRow(line) { return /^\s*\|.*\|\s*$/.test(line); }
function _isTableSeparator(line) {
  return /^\s*\|[\s:|\-]+\|\s*$/.test(line) && /-/.test(line);
}
function _parseRow(line) {
  let row = line.trim();
  if (row.startsWith('|')) row = row.slice(1);
  if (row.endsWith('|')) row = row.slice(0, -1);
  return row.split('|').map(c => c.trim());
}
function _renderTable(headerLine, bodyLines) {
  const headers = _parseRow(headerLine);
  let html = '<table><thead><tr>';
  for (const h of headers) html += '<th>' + renderInline(h) + '</th>';
  html += '</tr></thead><tbody>';
  for (const row of bodyLines) {
    const cells = _parseRow(row);
    html += '<tr>';
    for (const c of cells) html += '<td>' + renderInline(c) + '</td>';
    html += '</tr>';
  }
  html += '</tbody></table>';
  return html;
}
function renderMarkdown(md) {
  const lines = (md || '').split(/\r?\n/);
  const out = [];
  let inCode = false;
  let codeBuf = [];
  let listType = null;
  let inQuote = false;
  function flushList() {
    if (listType) { out.push('</' + listType + '>'); listType = null; }
  }
  function flushQuote() {
    if (inQuote) { out.push('</blockquote>'); inQuote = false; }
  }
  let i = 0;
  while (i < lines.length) {
    const raw = lines[i];
    if (inCode) {
      if (/^```/.test(raw)) {
        out.push('<pre><code>' + escapeHtml(codeBuf.join('\n')) + '</code></pre>');
        codeBuf = []; inCode = false;
      } else { codeBuf.push(raw); }
      i++; continue;
    }
    if (/^```/.test(raw)) {
      flushList(); flushQuote(); inCode = true; i++; continue;
    }
    if (
      _isTableRow(raw) &&
      i + 1 < lines.length &&
      _isTableSeparator(lines[i + 1])
    ) {
      flushList(); flushQuote();
      const header = raw;
      const body = [];
      i += 2;
      while (i < lines.length && _isTableRow(lines[i])) {
        body.push(lines[i]); i++;
      }
      out.push(_renderTable(header, body));
      continue;
    }
    const h = /^(#{1,6})\s+(.*)$/.exec(raw);
    if (h) {
      flushList(); flushQuote();
      const level = h[1].length;
      out.push('<h' + level + '>' + renderInline(h[2]) + '</h' + level + '>');
      i++; continue;
    }
    const ul = /^[-*]\s+(.*)$/.exec(raw);
    const ol = /^\d+\.\s+(.*)$/.exec(raw);
    if (ul) {
      flushQuote();
      if (listType !== 'ul') { flushList(); out.push('<ul>'); listType = 'ul'; }
      out.push('<li>' + renderInline(ul[1]) + '</li>');
      i++; continue;
    }
    if (ol) {
      flushQuote();
      if (listType !== 'ol') { flushList(); out.push('<ol>'); listType = 'ol'; }
      out.push('<li>' + renderInline(ol[1]) + '</li>');
      i++; continue;
    }
    const bq = /^>\s?(.*)$/.exec(raw);
    if (bq) {
      flushList();
      if (!inQuote) { out.push('<blockquote>'); inQuote = true; }
      out.push('<p>' + renderInline(bq[1]) + '</p>');
      i++; continue;
    }
    flushList(); flushQuote();
    if (/^\s*$/.test(raw)) { out.push(''); i++; continue; }
    if (/^[-=]{3,}\s*$/.test(raw)) { out.push('<hr>'); i++; continue; }
    out.push('<p>' + renderInline(raw) + '</p>');
    i++;
  }
  flushList(); flushQuote();
  if (inCode) {
    out.push('<pre><code>' + escapeHtml(codeBuf.join('\n')) + '</code></pre>');
  }
  return out.join('\n');
}
function _highlightYamlLine(escapedLine) {
  if (/^\s*#/.test(escapedLine)) {
    return '<span class="yaml-comment">' + escapedLine + '</span>';
  }
  let html = escapedLine;
  html = html.replace(/(\s)(#.*)$/, '$1<span class="yaml-comment">$2</span>');
  html = html.replace(
    /^(\s*-?\s*)('[^']*'|"[^"]*"|[\w./\-]+)(\s*:)/,
    '$1<span class="yaml-key">$2</span>$3'
  );
  html = html.replace(
    /(:\s|-\s)('[^']*'|"[^"]*")/g,
    '$1<span class="yaml-string">$2</span>'
  );
  html = html.replace(
    /(:\s|-\s)(true|false|null|yes|no)\b/g,
    '$1<span class="yaml-bool">$2</span>'
  );
  html = html.replace(
    /(:\s|-\s)(-?\d+(?:\.\d+)?)\b/g,
    '$1<span class="yaml-num">$2</span>'
  );
  return html;
}
function renderYaml(text) {
  const lines = (text || '').split(/\r?\n/);
  const highlighted = lines
    .map(l => _highlightYamlLine(escapeHtml(l)))
    .join('\n');
  return '<pre class="yaml-preview"><code>' + highlighted + '</code></pre>';
}

// ============================================================
// Modal — open neuron details (preserved from prior MRI)
// ============================================================
const navHistory = [];
let navIndex = -1;
function pushNav(id) {
  if (navHistory[navIndex] === id) return;
  navHistory.splice(navIndex + 1);
  navHistory.push(id);
  navIndex = navHistory.length - 1;
}
function openModal(node) {
  pushNav(node.id);
  const modal = document.getElementById('content-modal');
  const breadcrumb = node.path.split('/').map(p => p.replace(/\.(md|yml|yaml)$/, '')).join(' / ');
  document.getElementById('modal-kicker').textContent =
    node.type === 'glossary' ? 'GLOSSARY' :
    node.type === 'index' ? 'INDEX' :
    node.type === 'map' ? 'MAP' :
    node.file_type === 'yaml' ? 'YAML' : 'NEURON';
  document.getElementById('modal-title').textContent = node.title;
  const statsEl = document.getElementById('modal-stats');
  const parts = [];
  if (node.lobe && node.lobe !== 'root') parts.push(node.lobe);
  if (node.sublobe && node.sublobe !== node.lobe) parts.push(node.sublobe);
  if (node.tags && node.tags.length) parts.push('tags: ' + node.tags.join(', '));
  if (node.updated) parts.push('updated ' + node.updated);
  statsEl.innerHTML =
    '<span>' + escapeHtml(breadcrumb) + '</span>' +
    (parts.length ? '<span class="dot">·</span><span>' + escapeHtml(parts.join(' · ')) + '</span>' : '');
  renderFileTree(node.id, 'modal-tree');
  const raw = node.content_full || node.content_preview || 'No content.';
  const isYaml = node.file_type === 'yaml';
  const modalContent = document.getElementById('modal-content');
  modalContent.innerHTML = isYaml ? renderYaml(raw) : renderMarkdown(raw);
  if (!isYaml) {
    const nodePath = node.path.replace(/[^/]+$/, '');
    for (const a of modalContent.querySelectorAll('a.md-link[data-md-link]')) {
      const href = a.dataset.mdLink || '';
      const noAnchor = href.split('#')[0].split('?')[0];
      if (!noAnchor || /^https?:/.test(noAnchor)) continue;
      if (!/\.(md|ya?ml)$/i.test(noAnchor)) continue;
      const linkParts = (nodePath + noAnchor).split('/');
      const resolved = [];
      for (const p of linkParts) {
        if (p === '..') resolved.pop();
        else if (p && p !== '.') resolved.push(p);
      }
      const resolvedPath = resolved.join('/');
      const target = graph.nodes.find(n => n.path === resolvedPath);
      a.classList.remove('md-link');
      a.removeAttribute('href');
      if (target) {
        a.classList.add('content-link');
        a.dataset.modalNav = String(target.id);
        const anchor = href.includes('#') ? '#' + href.split('#')[1] : '';
        a.title = target.path + anchor;
      } else {
        a.classList.add('content-link-broken');
        a.title = 'broken link: ' + href;
      }
    }
    for (const btn of modalContent.querySelectorAll('[data-modal-nav]')) {
      btn.addEventListener('click', () => {
        const target = graph.nodes.find(n => n.id === Number(btn.dataset.modalNav));
        if (target) { openModal(target); }
      });
    }
  }
  const navEl = document.getElementById('modal-nav');
  const connected = [...(neighbors.get(node.id) || [])]
    .map(id => graph.nodes.find(n => n.id === id))
    .filter(n => n && n.type === 'neuron')
    .sort((a, b) => a.title.localeCompare(b.title));
  const NAV_COLLAPSE = 6;
  const currentParts = node.path.split('/');
  const currentParent = currentParts.length >= 2 ? currentParts[currentParts.length - 2] : '';
  const allNavHtml = connected.map(n => {
    const np = n.path.split('/');
    const parent = np.length >= 2 ? np[np.length - 2] : '';
    const label = parent && parent !== currentParent ? parent + ' / ' + n.title : n.title;
    return '<button type="button" class="modal-nav-btn" data-modal-nav="' + n.id +
      '" title="' + escapeHtml(n.path) + '">' + escapeHtml(label) + '</button>';
  });
  const hasOverflow = connected.length > NAV_COLLAPSE;
  function renderNav(expanded) {
    const btns = expanded ? allNavHtml.join('') : allNavHtml.slice(0, NAV_COLLAPSE).join('');
    const toggle = hasOverflow
      ? '<button type="button" class="modal-nav-toggle" id="modal-nav-toggle">' +
        (expanded ? 'show less' : '+' + (connected.length - NAV_COLLAPSE) + ' more') + '</button>'
      : '';
    navEl.innerHTML = btns + toggle;
    for (const btn of navEl.querySelectorAll('[data-modal-nav]')) {
      btn.addEventListener('click', () => {
        const target = graph.nodes.find(n => n.id === Number(btn.dataset.modalNav));
        if (target) { openModal(target); }
      });
    }
    const tog = document.getElementById('modal-nav-toggle');
    if (tog) tog.addEventListener('click', () => renderNav(!expanded));
  }
  renderNav(false);
  modal.style.display = 'flex';
}

// ============================================================
// Wire up controls
// ============================================================
searchInput.addEventListener('input', () => {
  refreshSearch();
  requestDraw();
});
searchInput.addEventListener('keydown', event => {
  if (event.key === 'Enter') {
    event.preventDefault();
    searchEnterJump();
  }
});
backBtn.addEventListener('click', () => {
  goPathBack();
});
forwardBtn.addEventListener('click', () => {
  goPathForward();
});
// Panel collapse — toggle a class on the body grid; show/hide expand button.
function togglePanel(side) {
  const body = document.querySelector('.body');
  const cls = side + '-collapsed';
  const collapsed = body.classList.toggle(cls);
  const expandBtn = document.getElementById('expand-' + side);
  if (expandBtn) expandBtn.classList.toggle('visible', collapsed);
  requestAnimationFrame(() => {
    resize();
    requestDraw();
  });
}
document.getElementById('collapse-left').addEventListener('click', () => togglePanel('left'));
document.getElementById('collapse-right').addEventListener('click', () => togglePanel('right'));
document.getElementById('expand-left').addEventListener('click', () => togglePanel('left'));
document.getElementById('expand-right').addEventListener('click', () => togglePanel('right'));

document.getElementById('modal-back').addEventListener('click', () => {
  if (navIndex > 0) {
    navIndex--;
    const node = graph.nodes.find(n => n.id === navHistory[navIndex]);
    if (node) { selectedId = node.id; openModal(node); }
  }
});
document.getElementById('modal-forward').addEventListener('click', () => {
  if (navIndex < navHistory.length - 1) {
    navIndex++;
    const node = graph.nodes.find(n => n.id === navHistory[navIndex]);
    if (node) { selectedId = node.id; openModal(node); }
  }
});
document.getElementById('modal-close').addEventListener('click', () => {
  document.getElementById('content-modal').style.display = 'none';
});
document.getElementById('content-modal').addEventListener('click', event => {
  if (event.target === event.currentTarget) event.currentTarget.style.display = 'none';
});
document.addEventListener('keydown', event => {
  if (event.key === 'Escape') {
    const modal = document.getElementById('content-modal');
    if (modal.style.display !== 'none') {
      modal.style.display = 'none';
    } else if (currentPath.length > 0) {
      goPathBack();
    }
  }
  if (event.key === '/' && event.target !== searchInput) {
    event.preventDefault();
    searchInput.focus();
    searchInput.select();
  }
});

// ============================================================
// Bootstrap
// ============================================================
resize();
addEventListener('resize', () => {
  resize();
  fitCurrentToView(true);
});
renderRecent();
renderFileTree(null, 'panel-tree');
renderBreadcrumb();
syncNavButtons();
fitCurrentToView(true);
loop();
