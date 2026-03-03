import { useEffect, useRef, useCallback } from 'react'

// ─── Constants ─────────────────────────────────────────────────────────────
const STAR_COUNT  = 340
const PARTICLE_N  = 260
const RAY_N       = 26
const PHASE_DUR   = [3000, 5000, 6000]  // ms for phases 0,1,2

const easeOut   = t => 1 - (1 - t) * (1 - t)
const easeInOut = t => t < 0.5 ? 2 * t * t : -1 + (4 - 2 * t) * t
const lerp      = (a, b, t) => a + (b - a) * t

function hashId(str) {
  let h = 0
  for (let i = 0; i < str.length; i++) {
    h = (Math.imul(31, h) + str.charCodeAt(i)) >>> 0
  }
  return (h >>> 0) / 0xFFFFFFFF
}

// ─── Helpers ───────────────────────────────────────────────────────────────
function buildStars(W, H) {
  const arr = []
  for (let i = 0; i < STAR_COUNT; i++) {
    arr.push({
      x: Math.random(), y: Math.random(),
      r: 0.3 + Math.random() * 1.3,
      a: 0.12 + Math.random() * 0.65,
      tw: Math.random() * Math.PI * 2,
      ts: 0.0004 + Math.random() * 0.0014,
      hue: Math.random() < 0.3 ? 200 + Math.random() * 60 : 200 + Math.random() * 30,
    })
  }
  return arr
}

function buildNebula(W, H) {
  const neb = document.createElement('canvas')
  neb.width = W; neb.height = H
  const nc = neb.getContext('2d')
  const cx = W / 2, cy = H / 2

  const bg = nc.createRadialGradient(cx, cy * 0.75, 0, cx, cy, Math.max(W, H) * 0.9)
  bg.addColorStop(0,    '#08081a')
  bg.addColorStop(0.35, '#050510')
  bg.addColorStop(0.65, '#030308')
  bg.addColorStop(1,    '#000000')
  nc.fillStyle = bg
  nc.fillRect(0, 0, W, H)

  const cloud = (x, y, r, a, h) => {
    const g = nc.createRadialGradient(x, y, 0, x, y, r)
    g.addColorStop(0,   `hsla(${h},40%,30%,${a})`)
    g.addColorStop(0.5, `hsla(${h},35%,20%,${a * 0.4})`)
    g.addColorStop(1,   'transparent')
    nc.fillStyle = g; nc.fillRect(0, 0, W, H)
  }
  cloud(W * 0.12, H * 0.28, W * 0.32, 0.18, 240)
  cloud(W * 0.88, H * 0.22, W * 0.28, 0.14, 230)
  cloud(W * 0.55, H * 0.75, W * 0.22, 0.10, 220)
  cloud(cx, cy * 1.1, Math.min(W, H) * 0.44, 0.12, 210)
  return neb
}

function freshParticle(cx, cy, W, H) {
  const ang = Math.random() * Math.PI * 2
  const r   = (0.2 + Math.random() * 0.8) * Math.min(W, H) * 0.40
  return {
    x: cx + Math.cos(ang) * r, y: cy + Math.sin(ang) * r,
    vx: (Math.random() - 0.5) * 0.52,
    vy: (Math.random() - 0.5) * 0.52,
    life: 0, maxLife: 220 + Math.random() * 320,
    size: 0.5 + Math.random() * 1.4,
    active: false,
  }
}

function freshRay(cx, cy, W, H, upward) {
  const srcAng = upward ? Math.PI + (Math.random() - 0.5) * 0.9 : Math.random() * Math.PI * 2
  const srcR   = Math.random() * Math.min(W, H) * 0.15
  return {
    sx: cx + Math.cos(srcAng) * srcR,
    sy: cy + Math.sin(srcAng) * srcR,
    angle: upward
      ? -Math.PI * 0.5 + (Math.random() - 0.5) * Math.PI * 1.3
      : Math.random() * Math.PI * 2,
    length: 90 + Math.random() * Math.min(W, H) * 0.38,
    width:  0.8 + Math.random() * 2.4,
    alpha:  upward ? 0.06 + Math.random() * 0.10 : 0.02 + Math.random() * 0.05,
    speed:  (Math.random() - 0.5) * 0.00035,
    life: 0, maxLife: 260 + Math.random() * 460,
    upward,
  }
}

// ─── Connection builder ─────────────────────────────────────────────────────
// Build topology-based connections when edges are available; nearest-neighbor fallback.
function buildConnections(nodes, topoEdges) {
  const idToIdx = {}
  nodes.forEach((n, i) => { idToIdx[n.id] = i })

  const hasTopology = topoEdges.length > 0

  nodes.forEach((n, i) => {
    if (n.type === 'agent' && hasTopology) {
      const connected = topoEdges
        .filter(e => e.from === n.id || e.to === n.id)
        .map(e => idToIdx[e.from === n.id ? e.to : e.from])
        .filter(j => j !== undefined && j !== i)
      // Deduplicate
      n.connections = [...new Set(connected)]
    } else {
      // Nearest-neighbor for holons or when no topology edges exist
      const dists = nodes
        .map((m, j) => ({ j, d: Math.hypot(n.ox - m.ox, n.oy - m.oy) }))
        .filter(e => e.j !== i)
        .sort((a, b) => a.d - b.d)
        .slice(0, 2 + Math.floor(Math.random() * 3))
      n.connections = dists.map(e => e.j)
    }
  })
}

// ─── Node builder ──────────────────────────────────────────────────────────
function buildNodes(W, H, apiAgents, apiHolons, apiTopology) {
  const cx = W / 2, cy = H / 2
  const nodes = []

  const topoNodes  = apiTopology?.nodes  || []
  const topoEdges  = apiTopology?.edges  || []
  const agentList  = apiAgents?.agents   || []
  const holonList  = apiHolons           || []

  const agentMap = {}
  agentList.forEach(a => { agentMap[a.agent_id] = a })

  const selfTopo  = topoNodes.find(n => n.is_self)
  const otherTopo = topoNodes.filter(n => !n.is_self)

  const fgSrc = selfTopo ? [selfTopo, ...otherTopo] : otherTopo
  const FG_COUNT = fgSrc.length

  // Elliptical orbit: fill the canvas regardless of aspect ratio
  const fgRadiusX = W * 0.38
  const fgRadiusY = H * 0.38

  fgSrc.forEach((tn, i) => {
    const agent = agentMap[tn.id] || {}
    const isSelf = tn.is_self || false
    const jitter = isSelf ? 0 : (hashId(tn.id + 'j') - 0.5) * 0.18
    const ang = (i / Math.max(FG_COUNT, 1)) * Math.PI * 2 - Math.PI * 0.5 + jitter
    const rJitterX = isSelf ? 0 : (hashId(tn.id + 'rx') - 0.5) * fgRadiusX * 0.12
    const rJitterY = isSelf ? 0 : (hashId(tn.id + 'ry') - 0.5) * fgRadiusY * 0.12
    const name = tn.name || agent.name || (tn.id || '').slice(-12)
    nodes.push({
      id: tn.id,
      type: 'agent',
      agentData: agent,
      ox: cx + Math.cos(ang) * (fgRadiusX + rJitterX),
      oy: cy + Math.sin(ang) * (fgRadiusY + rJitterY),
      x: cx, y: cy,
      phase: hashId(tn.id + 'ph') * Math.PI * 2,
      freq: 0.00032 + hashId(tn.id + 'fq') * 0.00020,
      size: isSelf ? 11 : 5.5 + hashId(tn.id + 'sz') * 3.0,
      brightness: isSelf ? 1.0 : 0.75 + hashId(tn.id + 'br') * 0.25,
      pulseFreq: 0.00022 + hashId(tn.id + 'pf') * 0.00018,
      pulsePhase: hashId(tn.id + 'pp') * Math.PI * 2,
      pulse: 1,
      depth: 1.0,
      spawnAt: isSelf ? 0.05 : 0.08 + (i / Math.max(FG_COUNT, 1)) * 0.60 + (hashId(tn.id + 'sp') - 0.5) * 0.04,
      born: false,
      bornAlpha: 0,
      labelAlpha: 0,
      label: name.length > 20 ? name.slice(0, 19) + '…' : name,
      connections: [],
      fg: true,
      isSelf,
    })
  })

  const bgSrc = holonList
  bgSrc.forEach((h, i) => {
    const ring  = Math.floor(i / 12)
    const maxR  = Math.max(1, Math.floor(bgSrc.length / 12))
    const baseR = ((ring + 0.5) / (maxR + 1)) * Math.min(W, H) * 0.22
    const ang   = (i % 12) * (Math.PI * 2 / 12) + ring * 0.6 + (hashId(h.task_id + 'a') - 0.5) * 0.9
    const r     = baseR + (hashId(h.task_id + 'r') - 0.5) * baseR * 0.45
    nodes.push({
      id: `holon:${h.task_id}`,
      type: 'holon',
      holonData: h,
      ox: cx + Math.cos(ang) * r,
      oy: cy + Math.sin(ang) * r,
      x: cx, y: cy,
      phase: hashId(h.task_id + 'ph') * Math.PI * 2,
      freq: 0.00040 + hashId(h.task_id + 'fq') * 0.00030,
      size: 2.0 + hashId(h.task_id + 'sz') * 3.0,
      brightness: 0.35 + hashId(h.task_id + 'br') * 0.35,
      pulseFreq: 0.00025 + hashId(h.task_id + 'pf') * 0.00030,
      pulsePhase: hashId(h.task_id + 'pp') * Math.PI * 2,
      pulse: 1,
      depth: 0.3 + Math.random() * 0.5,
      spawnAt: 0.12 + (i / Math.max(bgSrc.length, 1)) * 0.80 + (hashId(h.task_id + 'sp') - 0.5) * 0.04,
      born: false,
      bornAlpha: 0,
      labelAlpha: 0,
      label: null,
      connections: [],
      fg: false,
      isSelf: false,
    })
  })

  buildConnections(nodes, topoEdges)
  return nodes
}

// ─── Node status hue ───────────────────────────────────────────────────────
// healthy → 220 (blue-white), degraded → 42 (amber), offline → 0 (coral-red)
function nodeHue(n) {
  if (n.type !== 'agent') return 220
  const a = n.agentData
  if (!a || typeof a.connected === 'undefined') return 220
  if (a.connected === false) return 0
  if (a.loop_active === false) return 42
  return 220
}

// ─── Component ─────────────────────────────────────────────────────────────
export default function CosmicCanvas({ agents, holons, topology, onNodeClick }) {
  const canvasRef = useRef(null)
  const stateRef  = useRef(null)

  // Keep latest prop values accessible inside the one-time effect without
  // causing it to re-run (and restart the intro animation).
  const agentsRef   = useRef(agents)
  const holonsRef   = useRef(holons)
  const topologyRef = useRef(topology)
  agentsRef.current   = agents
  holonsRef.current   = holons
  topologyRef.current = topology

  // ─── Screen-space position accounting for zoom+rotation ─────────────────
  function screenPos(n, cx, cy, cameraZoom, rotAngle) {
    const dx = n.x - cx
    const dy = n.y - cy
    const cosA = Math.cos(rotAngle)
    const sinA = Math.sin(rotAngle)
    return {
      sx: cx + (cosA * dx - sinA * dy) * cameraZoom,
      sy: cy + (sinA * dx + cosA * dy) * cameraZoom,
    }
  }

  const handleClick = useCallback((e) => {
    const st = stateRef.current
    if (!st) return
    const rect = e.currentTarget.getBoundingClientRect()
    const mx = e.clientX - rect.left
    const my = e.clientY - rect.top
    const { nodes, cameraZoom, W, H, rotAngle } = st
    const cx = W / 2, cy = H / 2

    for (const n of nodes) {
      if (n.bornAlpha < 0.3) continue
      const { sx, sy } = screenPos(n, cx, cy, cameraZoom, rotAngle)
      const sr = n.size * n.pulse * cameraZoom
      if (Math.hypot(mx - sx, my - sy) < Math.max(sr * 2.5, 14)) {
        if (n.type === 'agent' && onNodeClick) {
          onNodeClick({ type: 'agent', data: { agent: n.agentData } })
        } else if (n.type === 'holon' && onNodeClick) {
          onNodeClick({ type: 'holon', data: n.holonData })
        }
        return
      }
    }
  }, [onNodeClick])

  const handleMouseMove = useCallback((e) => {
    const st = stateRef.current
    if (!st) return
    const rect = e.currentTarget.getBoundingClientRect()
    const mx = e.clientX - rect.left
    const my = e.clientY - rect.top
    const { nodes, cameraZoom, W, H, rotAngle } = st
    const cx = W / 2, cy = H / 2
    let hit = false
    for (const n of nodes) {
      if (n.bornAlpha < 0.3) continue
      const { sx, sy } = screenPos(n, cx, cy, cameraZoom, rotAngle)
      const sr = n.size * n.pulse * cameraZoom
      if (Math.hypot(mx - sx, my - sy) < Math.max(sr * 2.5, 14)) { hit = true; break }
    }
    e.currentTarget.style.cursor = hit ? 'pointer' : 'default'
  }, [])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')

    let W = canvas.width  = canvas.offsetWidth
    let H = canvas.height = canvas.offsetHeight
    let cx = W / 2, cy = H / 2

    let stars   = buildStars(W, H)
    let neb     = buildNebula(W, H)
    let nodes   = buildNodes(W, H, agentsRef.current, holonsRef.current, topologyRef.current)
    const ptcls = Array.from({ length: PARTICLE_N }, () => freshParticle(cx, cy, W, H))
    const rays  = Array.from({ length: RAY_N }, (_, i) => freshRay(cx, cy, W, H, i < 14))
    const bursts = []

    let cameraZoom = 0.07, cameraTargetZoom = 0.07
    let frameT = 0, lastTs = null, startTs = null
    let rafId  = null

    stateRef.current = { nodes, cameraZoom, W, H, rotAngle: 0 }

    function drawStars() {
      for (const s of stars) {
        const tw = 0.5 + 0.5 * Math.sin(frameT * s.ts + s.tw)
        ctx.fillStyle = `hsla(${s.hue},40%,90%,${s.a * tw})`
        ctx.beginPath()
        ctx.arc(s.x * W, s.y * H, s.r, 0, Math.PI * 2)
        ctx.fill()
      }
    }

    function drawRays(ga) {
      for (const r of rays) {
        const tl   = r.life / r.maxLife
        const fade = tl < 0.15 ? tl / 0.15 : tl > 0.75 ? (1 - tl) / 0.25 : 1
        const a    = r.alpha * fade * ga
        if (a < 0.005) {
          r.life++
          if (r.life > r.maxLife) Object.assign(r, freshRay(cx, cy, W, H, r.upward))
          continue
        }
        const ex = r.sx + Math.cos(r.angle) * r.length
        const ey = r.sy + Math.sin(r.angle) * r.length
        const g  = ctx.createLinearGradient(r.sx, r.sy, ex, ey)
        g.addColorStop(0,    `hsla(220,60%,88%,${a})`)
        g.addColorStop(0.35, `hsla(215,55%,72%,${a * 0.5})`)
        g.addColorStop(1,    `hsla(210,50%,60%,0)`)
        ctx.save()
        ctx.lineWidth = r.width
        ctx.strokeStyle = g
        ctx.beginPath()
        ctx.moveTo(r.sx, r.sy); ctx.lineTo(ex, ey)
        ctx.stroke()
        ctx.restore()
        r.angle += r.speed; r.life++
        if (r.life > r.maxLife) Object.assign(r, freshRay(cx, cy, W, H, r.upward))
      }
    }

    function drawBursts() {
      for (let i = bursts.length - 1; i >= 0; i--) {
        const b = bursts[i]; b.t++
        const p    = b.t / b.maxT
        if (p >= 1) { bursts.splice(i, 1); continue }
        const fade = p < 0.15 ? p / 0.15 : 1 - p
        const rad  = p * (b.big ? 90 : 60)
        const g = ctx.createRadialGradient(b.x, b.y, 0, b.x, b.y, rad)
        g.addColorStop(0,    `hsla(220,60%,98%,${fade * 0.92})`)
        g.addColorStop(0.25, `hsla(215,60%,85%,${fade * 0.55})`)
        g.addColorStop(0.65, `hsla(210,50%,70%,${fade * 0.20})`)
        g.addColorStop(1,    'transparent')
        ctx.fillStyle = g
        ctx.beginPath(); ctx.arc(b.x, b.y, rad, 0, Math.PI * 2); ctx.fill()
        const spokes = b.big ? 12 : 8
        ctx.save(); ctx.globalAlpha = fade * 0.45
        ctx.strokeStyle = 'hsla(220,80%,92%,1)'; ctx.lineWidth = 0.7
        for (let s = 0; s < spokes; s++) {
          const ang = (s / spokes) * Math.PI * 2 + p * 0.4
          ctx.beginPath()
          ctx.moveTo(b.x, b.y)
          ctx.lineTo(b.x + Math.cos(ang) * rad * 2.8, b.y + Math.sin(ang) * rad * 2.8)
          ctx.stroke()
        }
        ctx.restore()
      }
    }

    function drawEdges(connP) {
      // Max alpha distance — canvas diagonal (no hard cutoff, alpha fades)
      const maxDist = Math.hypot(W, H)
      const drawn = new Set()
      for (let i = 0; i < nodes.length; i++) {
        const n = nodes[i]
        if (!n.born) continue
        for (const j of n.connections) {
          const key = i < j ? `${i}-${j}` : `${j}-${i}`
          if (drawn.has(key)) continue; drawn.add(key)
          const m = nodes[j]
          if (!m.born) continue
          const dist = Math.hypot(n.x - m.x, n.y - m.y)
          const str   = Math.max(0, 1 - dist / maxDist)
          const pulse = 0.5 + 0.5 * Math.sin(frameT * 0.0008 + i * 0.7 + j * 0.4)
          const ba    = Math.min(n.bornAlpha, m.bornAlpha)
          const a     = str * pulse * 0.65 * connP * ba
          if (a < 0.008) continue
          const tv    = (frameT * 0.0012 + i * 0.3) % 1
          const g     = ctx.createLinearGradient(n.x, n.y, m.x, m.y)
          g.addColorStop(0,                       `hsla(220,60%,80%,${a * 0.8})`)
          g.addColorStop(Math.max(0, tv - 0.12),  `hsla(220,60%,80%,${a * 0.3})`)
          g.addColorStop(tv,                      `hsla(220,80%,98%,${a * 1.8})`)
          g.addColorStop(Math.min(1, tv + 0.12),  `hsla(220,60%,80%,${a * 0.3})`)
          g.addColorStop(1,                       `hsla(215,55%,75%,${a * 0.8})`)
          ctx.save()
          ctx.lineWidth   = 0.65 + str * 1.5
          ctx.strokeStyle = g
          ctx.shadowColor = `hsla(220,70%,80%,${a * 0.6})`
          ctx.shadowBlur  = 5
          ctx.beginPath(); ctx.moveTo(n.x, n.y); ctx.lineTo(m.x, m.y); ctx.stroke()
          ctx.restore()
        }
      }
    }

    function drawNodes() {
      for (const n of nodes) {
        if (n.bornAlpha <= 0.01) continue
        const a = n.bornAlpha
        const s = n.size * n.pulse
        const h = nodeHue(n)
        const saturation = h === 220 ? 60 : 80  // more saturated for alert states

        if (n.isSelf) {
          const haloR  = s * 9 + 4 * Math.sin(frameT * 0.0015)
          const haloA  = 0.25 + 0.15 * Math.sin(frameT * 0.0012)
          ctx.save()
          ctx.strokeStyle = `hsla(${h},70%,92%,${a * haloA})`
          ctx.lineWidth   = 1.5
          ctx.shadowColor = `hsla(${h},80%,95%,${a * 0.5})`
          ctx.shadowBlur  = 12
          ctx.beginPath(); ctx.arc(n.x, n.y, haloR, 0, Math.PI * 2); ctx.stroke()
          ctx.restore()
        }

        const g1 = ctx.createRadialGradient(n.x, n.y, 0, n.x, n.y, s * 6)
        g1.addColorStop(0,   `hsla(${h},${saturation}%,92%,${a * n.brightness * 0.9})`)
        g1.addColorStop(0.3, `hsla(${h - 5},55%,75%,${a * n.brightness * 0.38})`)
        g1.addColorStop(0.7, `hsla(${h - 10},50%,60%,${a * n.brightness * 0.10})`)
        g1.addColorStop(1,   'transparent')
        ctx.fillStyle = g1
        ctx.beginPath(); ctx.arc(n.x, n.y, s * 6, 0, Math.PI * 2); ctx.fill()

        const g2 = ctx.createRadialGradient(n.x, n.y, 0, n.x, n.y, s)
        g2.addColorStop(0,   `hsla(${h + 10},40%,98%,${a})`)
        g2.addColorStop(0.5, `hsla(${h},50%,85%,${a * 0.8})`)
        g2.addColorStop(1,   `hsla(${h - 5},45%,70%,0)`)
        ctx.fillStyle = g2
        ctx.beginPath(); ctx.arc(n.x, n.y, s, 0, Math.PI * 2); ctx.fill()
      }
    }

    function drawLabels(labelP, rotAngle) {
      if (labelP <= 0) return
      const FS    = 11
      const PAD_X = 7
      const PAD_Y = 4
      ctx.font = `${FS}px 'Courier New', monospace`

      for (const n of nodes) {
        if (!n.fg || !n.label || n.bornAlpha < 0.05) continue
        const la = (n.isSelf ? 1 : n.labelAlpha) * labelP
        if (la < 0.03) continue

        const dx = (n.x - cx) * cameraZoom
        const dy = (n.y - cy) * cameraZoom
        const cosA = Math.cos(rotAngle), sinA = Math.sin(rotAngle)
        const sx = cx + cosA * dx - sinA * dy
        const sy = cy + sinA * dx + cosA * dy
        const sr = n.size * n.pulse * cameraZoom

        const tw    = ctx.measureText(n.label).width
        const pillW = tw + PAD_X * 2
        const pillH = FS + PAD_Y * 2
        const px    = sx + sr + 8
        const py    = sy - pillH / 2

        ctx.save()
        ctx.globalAlpha = la * 0.55
        ctx.strokeStyle = n.isSelf ? 'rgba(200,210,240,1)' : 'rgba(160,170,210,1)'
        ctx.lineWidth   = 0.8
        ctx.setLineDash([3, 3])
        ctx.beginPath()
        ctx.moveTo(sx + sr * 0.9, sy); ctx.lineTo(px, sy)
        ctx.stroke(); ctx.setLineDash([])
        ctx.restore()

        ctx.save()
        ctx.globalAlpha = la * 0.82
        ctx.fillStyle   = n.isSelf ? 'rgba(4, 4, 18, 0.90)' : 'rgba(4, 4, 18, 0.75)'
        ctx.strokeStyle = n.isSelf
          ? `rgba(200,210,240,${la * 0.9})`
          : `rgba(150,160,200,${la * 0.5})`
        ctx.lineWidth = n.isSelf ? 1.2 : 0.8
        const rx = 3
        ctx.beginPath()
        ctx.moveTo(px + rx, py)
        ctx.lineTo(px + pillW - rx, py)
        ctx.quadraticCurveTo(px + pillW, py, px + pillW, py + rx)
        ctx.lineTo(px + pillW, py + pillH - rx)
        ctx.quadraticCurveTo(px + pillW, py + pillH, px + pillW - rx, py + pillH)
        ctx.lineTo(px + rx, py + pillH)
        ctx.quadraticCurveTo(px, py + pillH, px, py + pillH - rx)
        ctx.lineTo(px, py + rx)
        ctx.quadraticCurveTo(px, py, px + rx, py)
        ctx.closePath()
        ctx.fill(); ctx.stroke()
        ctx.restore()

        ctx.save()
        ctx.globalAlpha  = la
        ctx.font         = `${FS}px 'Courier New', monospace`
        ctx.fillStyle    = n.isSelf ? 'rgba(230,235,255,1)' : 'rgba(200,210,240,1)'
        ctx.shadowColor  = n.isSelf ? 'rgba(200,210,255,0.9)' : 'rgba(180,190,230,0.6)'
        ctx.shadowBlur   = n.isSelf ? 8 : 5
        ctx.textAlign    = 'left'
        ctx.textBaseline = 'middle'
        ctx.fillText(n.label, px + PAD_X, py + pillH / 2)
        ctx.restore()
      }
    }

    function drawParticles(ga) {
      for (const p of ptcls) {
        if (!p.active) continue
        p.x += p.vx; p.y += p.vy; p.life++
        if (p.life > p.maxLife) { Object.assign(p, freshParticle(cx, cy, W, H)); continue }
        const tl   = p.life / p.maxLife
        const fade = tl < 0.12 ? tl / 0.12 : tl > 0.8 ? (1 - tl) / 0.2 : 1
        ctx.fillStyle = `hsla(220,60%,85%,${fade * 0.55 * ga})`
        ctx.beginPath(); ctx.arc(p.x, p.y, p.size, 0, Math.PI * 2); ctx.fill()
      }
    }

    function drawCentralGlow(a) {
      if (a < 0.01) return
      const pulse  = 0.85 + 0.15 * Math.sin(frameT * 0.0005)
      const radius = Math.min(W, H) * 0.32 * pulse
      const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, radius)
      g.addColorStop(0,   `hsla(220,60%,70%,${0.07 * a})`)
      g.addColorStop(0.3, `hsla(215,55%,60%,${0.04 * a})`)
      g.addColorStop(0.7, `hsla(210,50%,50%,${0.02 * a})`)
      g.addColorStop(1,   'transparent')
      ctx.fillStyle = g
      ctx.beginPath(); ctx.arc(cx, cy, radius, 0, Math.PI * 2); ctx.fill()
    }

    function loop(ts) {
      rafId = requestAnimationFrame(loop)
      if (!lastTs) { lastTs = ts; startTs = ts }
      const dt      = Math.min(ts - lastTs, 32)
      lastTs = ts
      frameT += dt
      const elapsed = ts - startTs

      const p0end = PHASE_DUR[0]
      const p1end = p0end + PHASE_DUR[1]
      const p2end = p1end + PHASE_DUR[2]

      let connP = 0, labelP = 0, rayGA = 0, ptclGA = 0, phaseNum = 0

      if (elapsed < p0end) {
        phaseNum = 0
        const phaseT = elapsed / p0end
        cameraTargetZoom = lerp(0.06, 0.14, phaseT)
        rayGA = phaseT * 0.18
      } else if (elapsed < p1end) {
        phaseNum = 1
        const phaseT = (elapsed - p0end) / PHASE_DUR[1]
        cameraTargetZoom = lerp(0.14, 0.56, easeOut(phaseT))
        rayGA  = 0.18 + phaseT * 0.82
        ptclGA = easeOut(phaseT)
        for (const n of nodes) {
          if (!n.born && phaseT >= n.spawnAt) {
            n.born = true
            bursts.push({ x: n.ox, y: n.oy, t: 0, maxT: 42, big: n.isSelf })
            const idle = ptcls.find(p => !p.active)
            if (idle) { Object.assign(idle, freshParticle(cx, cy, W, H)); idle.active = true }
          }
        }
      } else if (elapsed < p2end) {
        phaseNum = 2
        const phaseT = (elapsed - p1end) / PHASE_DUR[2]
        cameraTargetZoom = lerp(0.56, 1.0, easeInOut(phaseT))
        connP  = easeOut(phaseT)
        rayGA  = 1
        ptclGA = 1
        labelP = phaseT > 0.45 ? easeOut((phaseT - 0.45) / 0.55) : 0
      } else {
        phaseNum = 3
        cameraTargetZoom = 1
        connP = labelP = rayGA = ptclGA = 1
      }

      cameraZoom += (cameraTargetZoom - cameraZoom) * 0.055

      const rotAngle = phaseNum === 3 ? frameT * 0.000028 : 0
      stateRef.current = { nodes, cameraZoom, W, H, rotAngle }

      for (const n of nodes) {
        if (n.born) {
          n.bornAlpha  = Math.min(1, n.bornAlpha + 0.038)
          if (n.label) n.labelAlpha = Math.min(1, n.labelAlpha + 0.022)
        }
        const d  = Math.sin(frameT * n.freq + n.phase)
        const d2 = Math.cos(frameT * n.freq * 1.3 + n.phase + 1)
        n.x = n.ox + d * 18 + d2 * 8
        n.y = n.oy + d2 * 18 + d * 8
        n.pulse = 0.62 + 0.38 * Math.sin(frameT * n.pulseFreq + n.pulsePhase)
      }

      if (phaseNum >= 1 && Math.random() < 0.14) {
        const idle = ptcls.find(p => !p.active)
        if (idle) { Object.assign(idle, freshParticle(cx, cy, W, H)); idle.active = true }
      }

      ctx.clearRect(0, 0, W, H)
      ctx.drawImage(neb, 0, 0)
      drawStars()

      ctx.save()
      ctx.translate(cx, cy)
      ctx.scale(cameraZoom, cameraZoom)
      if (phaseNum === 3) ctx.rotate(rotAngle)
      ctx.translate(-cx, -cy)
      drawRays(rayGA)
      drawParticles(ptclGA)
      drawEdges(connP)
      drawBursts()
      drawNodes()
      ctx.restore()

      drawLabels(labelP, rotAngle)
      drawCentralGlow(cameraZoom)

      const vig = ctx.createRadialGradient(cx, cy, Math.min(W, H) * 0.28, cx, cy, Math.max(W, H) * 0.72)
      vig.addColorStop(0, 'transparent')
      vig.addColorStop(1, 'rgba(0,0,0,0.72)')
      ctx.fillStyle = vig
      ctx.fillRect(0, 0, W, H)
    }

    rafId = requestAnimationFrame(loop)

    const onResize = () => {
      // Release old nebula canvas GPU memory before replacing
      neb.width = 0
      W = canvas.width  = canvas.offsetWidth
      H = canvas.height = canvas.offsetHeight
      cx = W / 2; cy = H / 2
      neb   = buildNebula(W, H)
      stars = buildStars(W, H)
      nodes = buildNodes(W, H, agentsRef.current, holonsRef.current, topologyRef.current)
      stateRef.current = { nodes, cameraZoom, W, H, rotAngle: 0 }
    }
    window.addEventListener('resize', onResize)

    return () => {
      cancelAnimationFrame(rafId)
      window.removeEventListener('resize', onResize)
    }
  }, []) // ← runs once; prop changes handled by the incremental effect below

  // ─── Incremental node update ────────────────────────────────────────────
  // When agents/holons/topology change, add new nodes (with spawn animation)
  // and remove departed ones — without restarting the intro animation.
  useEffect(() => {
    const st = stateRef.current
    if (!st) return
    const { W, H, nodes } = st

    const newNodes = buildNodes(W, H, agents, holons, topology)
    const existingMap = new Map(nodes.map(n => [n.id, n]))
    const incomingIds  = new Set(newNodes.map(n => n.id))
    let changed = false

    // Update existing nodes' data; add genuinely new nodes
    for (const n of newNodes) {
      const existing = existingMap.get(n.id)
      if (existing) {
        existing.agentData = n.agentData
        existing.holonData = n.holonData
        existing.label     = n.label
      } else {
        // New node: start invisible and fade in naturally
        n.born      = true
        n.bornAlpha = 0
        n.labelAlpha = 0
        nodes.push(n)
        changed = true
      }
    }

    // Remove nodes no longer in the swarm
    for (let i = nodes.length - 1; i >= 0; i--) {
      if (!incomingIds.has(nodes[i].id)) {
        nodes.splice(i, 1)
        changed = true
      }
    }

    // Always rebuild connections so topology edges stay current
    buildConnections(nodes, topology?.edges || [])
  }, [agents, holons, topology])

  return (
    <canvas
      ref={canvasRef}
      style={{ width: '100%', height: '100%', display: 'block' }}
      onClick={handleClick}
      onMouseMove={handleMouseMove}
    />
  )
}
