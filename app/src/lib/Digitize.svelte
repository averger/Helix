<script>
  import { onMount, onDestroy } from 'svelte'
  import {
    telemetry, scans, activePanel,
    refreshScans, createScan, capturePoint, undoPoint, finishScan, discardScan,
    startGridScan, cancelGridScan, deleteScan, exportScan,
    showScanCloud, scanCloud, refreshPrograms,
  } from './store.js'

  let newName = $state('')
  let autoCapture = $state(false)
  let spacing = $state(1)
  let exportFeed = $state(800)

  let grid = $state({ x0: -60, y0: -40, x1: 60, y1: 40, step: 5, z_safe: 5, z_min: -22, refine_dz: 0.8 })

  let session = $derived($telemetry?.scan?.session ?? null)
  let job = $derived($telemetry?.scan?.job ?? { active: false, done: 0, total: 0 })
  let canCapture = $derived(['idle', 'jog'].includes($telemetry?.state))
  let canGrid = $derived($telemetry?.state === 'idle' && !job.active)

  onMount(refreshScans)

  // auto-capture while jogging: record a point whenever the spindle has
  // moved more than `spacing` from the last captured position
  let lastAuto = null
  $effect(() => {
    const t = $telemetry
    if (!autoCapture || !session || !t || !['idle', 'jog'].includes(t.state)) return
    const p = [t.position.x, t.position.y, t.position.z]
    if (!lastAuto || Math.hypot(p[0] - lastAuto[0], p[1] - lastAuto[1], p[2] - lastAuto[2]) >= spacing) {
      lastAuto = p
      capturePoint().then(() => showScanCloud(null))
    }
  })

  // live point cloud while a grid scan runs
  let pollTimer = null
  $effect(() => {
    if (job.active && !pollTimer) {
      pollTimer = setInterval(() => showScanCloud(null), 500)
    } else if (!job.active && pollTimer) {
      clearInterval(pollTimer)
      pollTimer = null
      if (session) showScanCloud(null)
    }
  })
  onDestroy(() => clearInterval(pollTimer))

  async function onCreate(e) {
    e.preventDefault()
    const name = newName.trim()
    if (!name) return
    if (await createScan(name)) {
      newName = ''
      lastAuto = null
    }
  }

  async function onCapture() {
    if (await capturePoint()) showScanCloud(null)
  }

  async function onUndo() {
    await undoPoint()
    showScanCloud(null)
  }

  async function onFinish() {
    if (await finishScan()) {
      autoCapture = false
      await refreshScans()
    }
  }

  async function onDiscard() {
    await discardScan()
    autoCapture = false
    scanCloud.set(null)
  }

  async function onGrid() {
    await startGridScan({
      x0: +grid.x0, y0: +grid.y0, x1: +grid.x1, y1: +grid.y1,
      step: +grid.step, z_safe: +grid.z_safe, z_min: +grid.z_min, refine_dz: +grid.refine_dz,
    })
  }

  async function onExport(name, mode) {
    const result = await exportScan(name, { mode, feed: +exportFeed, z_safe: 5 })
    if (result) await refreshPrograms()
  }

  async function onDelete(name) {
    await deleteScan(name)
    await refreshScans()
  }
</script>

<div class="drawer panel">
  <div class="head">
    <span class="label">Digitize</span>
    <button class="close" onclick={() => activePanel.set(null)}>✕</button>
  </div>

  {#if session}
    <div class="session">
      <div class="meta">
        <span class="sname">{session.name}</span>
        <span class="count">{session.points} pts</span>
      </div>

      {#if job.active}
        <div class="jobrow">
          <div class="track">
            <div class="fill" style="width: {job.total ? (job.done / job.total) * 100 : 0}%"></div>
          </div>
          <span class="jobtext">{job.done} / {job.total}</span>
        </div>
        <button class="btn danger" onclick={cancelGridScan}>Cancel scan</button>
      {:else}
        <button class="btn primary big" disabled={!canCapture} onclick={onCapture}>
          ⊕ Capture point
        </button>
        <div class="auto">
          <button class="chip" class:active={autoCapture} onclick={() => { autoCapture = !autoCapture; lastAuto = null }}>
            auto
          </button>
          <span class="dim">every</span>
          {#each [0.5, 1, 2, 5] as s}
            <button class="chip" class:active={spacing === s} onclick={() => (spacing = s)}>{s}</button>
          {/each}
          <span class="dim">mm</span>
        </div>

        <div class="sep"></div>
        <span class="label">Grid scan</span>
        <div class="gridform">
          <label>X <input type="number" bind:value={grid.x0} /> → <input type="number" bind:value={grid.x1} /></label>
          <label>Y <input type="number" bind:value={grid.y0} /> → <input type="number" bind:value={grid.y1} /></label>
          <label>step <input type="number" step="0.5" bind:value={grid.step} /> refine Δz <input type="number" step="0.1" bind:value={grid.refine_dz} /></label>
          <label>safe Z <input type="number" bind:value={grid.z_safe} /> min Z <input type="number" bind:value={grid.z_min} /></label>
        </div>
        <button class="btn ghost-accent" disabled={!canGrid} onclick={onGrid}>▦ Run grid scan</button>

        <div class="sep"></div>
        <div class="row">
          <button class="btn" disabled={!session.points} onclick={onUndo}>↩ Undo</button>
          <button class="btn primary" disabled={!session.points} onclick={onFinish}>✓ Save</button>
          <button class="btn" onclick={onDiscard}>✕ Discard</button>
        </div>
      {/if}
    </div>
  {:else}
    <form class="newscan" onsubmit={onCreate}>
      <input type="text" placeholder="New scan name…" bind:value={newName} spellcheck="false" />
      <button class="btn primary" disabled={!newName.trim()}>Start</button>
    </form>
  {/if}

  <div class="sep"></div>
  <span class="label">Saved scans</span>
  <div class="list">
    {#each $scans as s (s.name)}
      <div class="item" class:active={$scanCloud?.name === s.name}>
        <button class="iname" onclick={() => showScanCloud(s.name)}>
          <span class="pname">{s.name}</span>
          <span class="size">{s.points} pts · {s.mode}</span>
        </button>
        <div class="actions">
          <button title="Reproduce as contour G-code" onclick={() => onExport(s.name, 'contour')}>↯</button>
          {#if s.mode === 'grid'}
            <button title="Reproduce as raster G-code" onclick={() => onExport(s.name, 'raster')}>▦</button>
          {/if}
          <button title="Delete" class="del" onclick={() => onDelete(s.name)}>🗑</button>
        </div>
      </div>
    {:else}
      <span class="empty">No saved scans</span>
    {/each}
  </div>

  <div class="exportfeed">
    <span class="dim">export feed</span>
    <input type="number" step="50" bind:value={exportFeed} />
    <span class="dim">mm/min</span>
  </div>
</div>

<style>
  .drawer {
    position: absolute;
    top: 14px;
    left: 14px;
    bottom: 14px;
    width: 320px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    background: rgba(13, 15, 18, 0.88);
    backdrop-filter: blur(18px);
    -webkit-backdrop-filter: blur(18px);
    animation: slide-in 220ms var(--ease);
    z-index: 10;
    overflow-y: auto;
  }

  @keyframes slide-in {
    from {
      transform: translateX(-16px);
      opacity: 0;
    }
  }

  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .close {
    color: var(--text-faint);
    font-size: 14px;
    padding: 4px 8px;
    border-radius: var(--r-sm);
  }

  .close:hover {
    color: var(--text);
  }

  .session {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .meta {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .sname {
    font-weight: 600;
    font-family: var(--font-mono);
    font-size: 13px;
  }

  .count {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--accent);
  }

  .big {
    padding: 14px;
    font-size: 14px;
  }

  .auto {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .dim {
    font-size: 11px;
    color: var(--text-faint);
  }

  .chip {
    padding: 4px 9px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    font-family: var(--font-mono);
    color: var(--text-dim);
    border: 1px solid var(--hairline);
    transition: all 150ms var(--ease);
  }

  .chip.active {
    color: #00191f;
    background: var(--accent);
    border-color: var(--accent);
  }

  .sep {
    height: 1px;
    background: var(--hairline);
  }

  .gridform {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .gridform label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--text-dim);
  }

  .gridform input,
  .exportfeed input {
    width: 58px;
    padding: 5px 8px;
    border-radius: var(--r-sm);
    background: var(--panel-2);
    border: 1px solid var(--hairline);
    font-family: var(--font-mono);
    font-size: 12px;
    outline: none;
  }

  .gridform input:focus {
    border-color: rgba(39, 224, 255, 0.5);
  }

  .jobrow {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .track {
    flex: 1;
    height: 4px;
    border-radius: 2px;
    background: var(--hairline);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    box-shadow: 0 0 10px rgba(39, 224, 255, 0.6);
    transition: width 200ms linear;
  }

  .jobtext {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-dim);
    white-space: nowrap;
  }

  .row {
    display: flex;
    gap: 8px;
  }

  .row .btn {
    flex: 1;
    padding: 8px 6px;
    font-size: 12px;
  }

  .newscan {
    display: flex;
    gap: 8px;
  }

  .newscan input {
    flex: 1;
    padding: 10px 14px;
    border-radius: var(--r-md);
    background: var(--panel-2);
    border: 1px solid var(--hairline);
    font-size: 13px;
    outline: none;
  }

  .newscan input:focus {
    border-color: rgba(39, 224, 255, 0.5);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--r-md);
    border: 1px solid var(--hairline);
    background: var(--panel-2);
  }

  .item.active {
    border-color: rgba(39, 224, 255, 0.5);
    background: var(--accent-dim);
  }

  .iname {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    text-align: left;
  }

  .pname {
    font-family: var(--font-mono);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size {
    font-size: 10px;
    color: var(--text-faint);
  }

  .actions {
    display: flex;
    gap: 2px;
  }

  .actions button {
    padding: 5px 7px;
    border-radius: var(--r-sm);
    font-size: 13px;
    color: var(--text-dim);
  }

  .actions button:hover {
    color: var(--accent);
    background: var(--accent-dim);
  }

  .actions .del:hover {
    color: var(--danger);
    background: var(--danger-dim);
  }

  .empty {
    color: var(--text-faint);
    font-size: 13px;
    padding: 8px;
  }

  .exportfeed {
    display: flex;
    align-items: center;
    gap: 8px;
  }
</style>
