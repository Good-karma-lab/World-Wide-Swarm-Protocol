# Cosmic Canvas Topology Redesign

**Date:** 2026-03-02
**Branch:** WWS
**Status:** Approved

## Overview

Replace the vis-network main swarm topology view with a custom canvas renderer that matches the space/cosmic aesthetic from the example design. Apply a platinum color scheme throughout the UI. Keep vis-network for the holon detail panel (task-scoped view).

## Goals

- Topology view: nebula background, starfield, animated platinum-glow nodes, travel-light edges, startup zoom animation, slow idle rotation
- Self node (is_self: true) rendered larger + brighter with outer pulsing halo
- Click on any node opens the appropriate detail panel
- Platinum/silver color scheme replaces current blue/teal throughout
- All existing functionality preserved (bottom tray, slide panels, modals, holon detail)

## Color Scheme

Primary: platinum `#e8e8f0` (cool silver-white with blue tint)
Node core: `hsla(220, 30%, 92%, ...)` — cool platinum/white
Edge travel-light: platinum shimmer
Labels: platinum text on dark pill background
CSS vars to add: `--platinum`, `--platinum-dim`, `--platinum-glow`

## Architecture

### LiveGraph.jsx — two modes

**Mode A: Main swarm canvas (default)**
Activated when `taskHolon` is null.

Canvas rendering pipeline (per frame):
1. Offscreen nebula gradient (painted once on resize)
2. Starfield — 340 twinkling stars, unaffected by zoom/rotation
3. Zoomed transform layer: rays → particles → animated edges → burst effects → glow nodes
4. Screen-space labels (pill badges, always crisp, not zoomed)
5. Central glow + vignette overlay

Startup timeline:
- Phase 0 (0–3s): nebula + starfield only, camera at 7% zoom
- Phase 1 (3–8s): nodes spawn one-by-one with burst; camera zooms 14%→56%
- Phase 2 (8–14s): camera 56%→100%, edges draw, labels reveal, UI fades in
- Phase 3 (14s+): idle — slow rotation (0.000028 rad/frame), all elements at full opacity

Node layout:
- FG nodes (agents from topology API, `is_self` first): outer ring, labeled
- BG fill nodes (holons from holons API): smaller inner nodes, no label
- Self node: 2× size, first in outer ring, always-visible label, outer pulsing halo ring

Click detection:
- `mousedown` event on canvas
- Hit-test all nodes: check if click falls within node circle (world→screen coordinate transform)
- Agent node hit → `onNodeClick({ type: 'agent', data: { agent } })`
- Holon node hit → `onNodeClick({ type: 'holon', data: holon })`
- Hover → show cursor: pointer

**Mode B: Holon detail (vis-network)**
Activated when `taskHolon` is set. Unchanged from current implementation.

### styles.css

- Replace `--teal` primary accent with `--platinum: #e8e8f0`
- `--platinum-dim: #8888a8`, `--platinum-glow: rgba(232,232,240,0.6)`
- Background stays `#000` (pure black, matching example)
- Header: transparent/blur, platinum brand text with text-shadow glow
- Buttons: platinum border/text; primary button: platinum bg with dark text
- Badges: platinum variants
- Health dots: keep red/yellow, replace green with platinum

### Header.jsx

- Brand "WWS" in Palatino serif with platinum glow text-shadow
- Watermark div "WWS · Agent Protocol" positioned bottom-left (fixed, z-index 10)

## Files to Modify

1. `webapp/src/components/LiveGraph.jsx` — full canvas implementation + keep vis-network holon mode
2. `webapp/src/styles.css` — platinum color scheme
3. `webapp/src/components/Header.jsx` — serif brand, glow, watermark

## Out of Scope

- No changes to slide panels content
- No changes to bottom tray data/logic
- No changes to API client or polling
- No changes to backend
