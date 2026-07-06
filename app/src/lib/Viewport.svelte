<script>
  import { onMount } from 'svelte'
  import * as THREE from 'three'
  import { OrbitControls } from 'three/addons/controls/OrbitControls.js'
  import { telemetry, toolpath, scanCloud } from './store.js'

  let canvas = $state(null)
  let ready = $state(false)

  let scene, camera, renderer, controls
  let pathGroup = null
  let tool = null
  let toolLight = null
  let segmentLines = [] // THREE.Line per segment, in program order
  let cloudGroup = null // digitized point cloud + trace line

  const COLOR_RAPID = new THREE.Color('#39414d')
  const COLOR_FEED = new THREE.Color('#aeb7c4')
  const COLOR_DONE = new THREE.Color('#27e0ff')

  onMount(() => {
    scene = new THREE.Scene()

    camera = new THREE.PerspectiveCamera(40, 1, 0.1, 5000)
    camera.up.set(0, 0, 1) // machine convention: Z is up
    camera.position.set(180, -220, 160)

    renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true })
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2))

    controls = new OrbitControls(camera, canvas)
    controls.enableDamping = true
    controls.dampingFactor = 0.08
    controls.maxPolarAngle = Math.PI * 0.95

    // bed grid on the XY plane
    const grid = new THREE.GridHelper(400, 40, 0x232a33, 0x161b21)
    grid.rotation.x = Math.PI / 2
    scene.add(grid)

    // axis triad at machine zero
    scene.add(axisLine([0, 0, 0], [30, 0, 0], 0xff5560))
    scene.add(axisLine([0, 0, 0], [0, 30, 0], 0x49e07c))
    scene.add(axisLine([0, 0, 0], [0, 0, 30], 0x4f8cff))

    // live tool: glowing tip + spindle axis hint
    tool = new THREE.Group()
    const tip = new THREE.Mesh(
      new THREE.SphereGeometry(1.6, 24, 24),
      new THREE.MeshBasicMaterial({ color: 0x27e0ff })
    )
    const halo = new THREE.Sprite(
      new THREE.SpriteMaterial({
        map: haloTexture(),
        color: 0x27e0ff,
        transparent: true,
        opacity: 0.85,
        depthWrite: false,
      })
    )
    halo.scale.set(14, 14, 1)
    const quill = new THREE.Mesh(
      new THREE.CylinderGeometry(0.7, 0.7, 36, 12),
      new THREE.MeshBasicMaterial({ color: 0x4d5562, transparent: true, opacity: 0.6 })
    )
    quill.rotation.x = Math.PI / 2
    quill.position.z = 19
    tool.add(tip, halo, quill)
    scene.add(tool)

    toolLight = new THREE.PointLight(0x27e0ff, 0, 60)
    scene.add(toolLight)

    const resize = () => {
      const { clientWidth: w, clientHeight: h } = canvas.parentElement
      renderer.setSize(w, h, false)
      camera.aspect = w / h
      camera.updateProjectionMatrix()
    }
    const observer = new ResizeObserver(resize)
    observer.observe(canvas.parentElement)
    resize()

    let raf
    const loop = () => {
      controls.update()
      renderer.render(scene, camera)
      raf = requestAnimationFrame(loop)
    }
    loop()
    ready = true

    return () => {
      cancelAnimationFrame(raf)
      observer.disconnect()
      controls.dispose()
      renderer.dispose()
    }
  })

  function axisLine(from, to, color) {
    const geo = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(...from),
      new THREE.Vector3(...to),
    ])
    return new THREE.Line(geo, new THREE.LineBasicMaterial({ color }))
  }

  function haloTexture() {
    const size = 64
    const c = document.createElement('canvas')
    c.width = c.height = size
    const ctx = c.getContext('2d')
    const g = ctx.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2)
    g.addColorStop(0, 'rgba(255,255,255,0.9)')
    g.addColorStop(0.25, 'rgba(255,255,255,0.25)')
    g.addColorStop(1, 'rgba(255,255,255,0)')
    ctx.fillStyle = g
    ctx.fillRect(0, 0, size, size)
    return new THREE.CanvasTexture(c)
  }

  // rebuild path lines when a program is loaded
  $effect(() => {
    const tp = $toolpath
    if (!ready) return
    if (pathGroup) {
      scene.remove(pathGroup)
      for (const line of segmentLines) {
        line.geometry.dispose()
        line.material.dispose()
      }
      segmentLines = []
      pathGroup = null
    }
    if (!tp?.segments?.length) return

    pathGroup = new THREE.Group()
    for (const seg of tp.segments) {
      const pts = seg.points.map((p) => new THREE.Vector3(p[0], p[1], p[2]))
      const geo = new THREE.BufferGeometry().setFromPoints(pts)
      let line
      if (seg.kind === 'rapid') {
        line = new THREE.Line(
          geo,
          new THREE.LineDashedMaterial({ color: COLOR_RAPID, dashSize: 3, gapSize: 3, transparent: true, opacity: 0.8 })
        )
        line.computeLineDistances()
      } else {
        line = new THREE.Line(geo, new THREE.LineBasicMaterial({ color: COLOR_FEED }))
      }
      line.userData.kind = seg.kind
      pathGroup.add(line)
      segmentLines.push(line)
    }
    scene.add(pathGroup)

    // frame the program extents
    const min = tp.extent.min
    const max = tp.extent.max
    const cx = (min[0] + max[0]) / 2
    const cy = (min[1] + max[1]) / 2
    const span = Math.max(max[0] - min[0], max[1] - min[1], max[2] - min[2], 40)
    controls.target.set(cx, cy, (min[2] + max[2]) / 2)
    camera.position.set(cx + span * 1.1, cy - span * 1.4, span * 1.1)
  })

  // digitized point cloud: dots colored by depth, plus the trace order
  // for manual scans
  $effect(() => {
    const cloud = $scanCloud
    if (!ready) return
    if (cloudGroup) {
      scene.remove(cloudGroup)
      cloudGroup.traverse((o) => {
        o.geometry?.dispose()
        o.material?.dispose()
      })
      cloudGroup = null
    }
    if (!cloud?.points?.length) return

    const pts = cloud.points
    let zMin = Infinity
    let zMax = -Infinity
    for (const p of pts) {
      if (p[2] < zMin) zMin = p[2]
      if (p[2] > zMax) zMax = p[2]
    }
    const span = Math.max(zMax - zMin, 1e-6)

    const positions = new Float32Array(pts.length * 3)
    const colors = new Float32Array(pts.length * 3)
    const deep = new THREE.Color('#1b4965')
    const high = new THREE.Color('#27e0ff')
    const c = new THREE.Color()
    for (let i = 0; i < pts.length; i++) {
      positions.set(pts[i], i * 3)
      c.lerpColors(deep, high, (pts[i][2] - zMin) / span)
      colors.set([c.r, c.g, c.b], i * 3)
    }
    const geo = new THREE.BufferGeometry()
    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geo.setAttribute('color', new THREE.BufferAttribute(colors, 3))

    cloudGroup = new THREE.Group()
    cloudGroup.add(
      new THREE.Points(geo, new THREE.PointsMaterial({ size: 2.2, vertexColors: true, sizeAttenuation: true }))
    )
    if (cloud.mode === 'manual' && pts.length > 1) {
      const lineGeo = new THREE.BufferGeometry().setFromPoints(pts.map((p) => new THREE.Vector3(...p)))
      cloudGroup.add(
        new THREE.Line(lineGeo, new THREE.LineBasicMaterial({ color: 0x27e0ff, transparent: true, opacity: 0.35 }))
      )
    }
    scene.add(cloudGroup)
  })

  // live updates: tool position + completed-path coloring
  $effect(() => {
    const t = $telemetry
    if (!ready || !t) return
    const { x, y, z } = t.position
    tool.position.set(x, y, z)
    toolLight.position.set(x, y, z + 4)
    toolLight.intensity = t.spindle?.on ? 220 : 0

    const done = t.program?.segment ?? -1
    for (let i = 0; i < segmentLines.length; i++) {
      const line = segmentLines[i]
      if (line.userData.kind === 'rapid') continue
      const target = done >= 0 && i < done ? COLOR_DONE : COLOR_FEED
      if (!line.material.color.equals(target)) {
        line.material.color.copy(target)
      }
    }
  })
</script>

<div class="viewport">
  <canvas bind:this={canvas}></canvas>
  <div class="hint">
    drag · orbit&nbsp;&nbsp;|&nbsp;&nbsp;scroll · zoom&nbsp;&nbsp;|&nbsp;&nbsp;⌨ arrows/PgUp/PgDn · jog&nbsp;&nbsp;·&nbsp;&nbsp;space · hold&nbsp;&nbsp;·&nbsp;&nbsp;esc · stop
  </div>
</div>

<style>
  .viewport,
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
  }

  .hint {
    position: absolute;
    right: 16px;
    bottom: 12px;
    font-size: 11px;
    color: var(--text-faint);
    letter-spacing: 0.04em;
    pointer-events: none;
  }
</style>
