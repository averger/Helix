<script>
  import { telemetry, send } from './store.js'

  const AXES = ['x', 'y', 'z']
  const WCS = ['G54', 'G55', 'G56', 'G57', 'G58', 'G59', 'G59.1', 'G59.2', 'G59.3']

  let showMachine = $state(false)

  let pos = $derived($telemetry?.position ?? { x: 0, y: 0, z: 0 })
  let mpos = $derived($telemetry?.machine_position ?? { x: 0, y: 0, z: 0 })
  let homedAxes = $derived($telemetry?.homed_axes ?? { x: false, y: false, z: false })
  let wcs = $derived($telemetry?.wcs ?? { index: 0, name: 'G54' })
  let tool = $derived($telemetry?.tool ?? { number: 0, length: 0 })
  let feed = $derived($telemetry?.feed?.actual ?? 0)
  let rpm = $derived($telemetry?.spindle?.rpm ?? 0)
  let spindleOn = $derived($telemetry?.spindle?.on ?? false)
  let canSetup = $derived(['idle', 'jog'].includes($telemetry?.state))

  function fmt(v) {
    const s = v.toFixed(3)
    return s === '-0.000' ? '0.000' : s
  }
</script>

<div class="panel dro">
  <div class="head">
    <span class="label">Position</span>
    <div class="opts">
      <select
        class="wcs"
        value={wcs.index}
        disabled={!canSetup}
        onchange={(e) => send({ cmd: 'wcs', index: +e.currentTarget.value })}
      >
        {#each WCS as name, i}
          <option value={i}>{name}</option>
        {/each}
      </select>
      <button class="mode" class:on={showMachine} onclick={() => (showMachine = !showMachine)}>
        {showMachine ? 'G53' : 'WORK'}
      </button>
    </div>
  </div>

  {#each AXES as axis}
    <div class="axis">
      <button
        class="name"
        class:unhomed={!homedAxes[axis]}
        title={homedAxes[axis] ? `${axis.toUpperCase()} homed` : `Home ${axis.toUpperCase()}`}
        disabled={homedAxes[axis] || !['on', 'idle'].includes($telemetry?.state)}
        onclick={() => send({ cmd: 'home', axis })}
      >
        {axis.toUpperCase()}
      </button>
      <span class="value">{fmt(showMachine ? mpos[axis] : pos[axis])}</span>
      <button
        class="zero"
        title="Touch off — current position becomes {axis.toUpperCase()}0 in {wcs.name}"
        disabled={!canSetup || showMachine}
        onclick={() => send({ cmd: 'touch_off', axis, value: 0 })}
      >
        ⌀
      </button>
    </div>
  {/each}

  <div class="meters">
    <div class="meter">
      <span class="label">Feed</span>
      <span class="mono">{Math.round(feed)}</span>
      <span class="unit">mm/min</span>
    </div>
    <div class="meter" class:live={spindleOn}>
      <span class="label">Spindle</span>
      <span class="mono">{Math.round(rpm)}</span>
      <span class="unit">rpm</span>
    </div>
  </div>

  <div class="toolrow">
    <span class="label">Tool</span>
    <span class="mono">{tool.number > 0 ? `T${tool.number}` : '—'}</span>
    {#if tool.number > 0}
      <span class="unit">L {tool.length.toFixed(2)} mm</span>
    {/if}
  </div>
</div>

<style>
  .dro {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .opts {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .wcs {
    appearance: none;
    padding: 4px 10px;
    border-radius: 999px;
    background: var(--accent-dim);
    border: 1px solid rgba(39, 224, 255, 0.4);
    color: var(--accent);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 700;
    cursor: pointer;
    outline: none;
  }

  .wcs:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .mode {
    padding: 4px 10px;
    border-radius: 999px;
    border: 1px solid var(--hairline);
    color: var(--text-faint);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
  }

  .mode.on {
    color: var(--warn);
    border-color: rgba(251, 191, 36, 0.4);
  }

  .axis {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 10px;
    border-radius: var(--r-md);
    background: var(--panel-2);
    border: 1px solid var(--hairline);
  }

  .name {
    width: 26px;
    height: 26px;
    border-radius: var(--r-sm);
    font-size: 14px;
    font-weight: 700;
    color: var(--accent);
  }

  .name.unhomed:not(:disabled) {
    color: var(--warn);
    border: 1px dashed rgba(251, 191, 36, 0.5);
  }

  .name:disabled {
    cursor: default;
  }

  .value {
    flex: 1;
    text-align: right;
    font-family: var(--font-mono);
    font-size: 28px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.01em;
  }

  .zero {
    width: 28px;
    height: 28px;
    border-radius: var(--r-sm);
    border: 1px solid var(--hairline);
    color: var(--text-faint);
    font-size: 13px;
    transition: all 150ms var(--ease);
  }

  .zero:hover:not(:disabled) {
    color: var(--accent);
    border-color: rgba(39, 224, 255, 0.5);
  }

  .zero:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .meters {
    display: flex;
    gap: 10px;
  }

  .meter {
    flex: 1;
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--r-md);
    border: 1px solid var(--hairline);
    transition: border-color 200ms var(--ease);
  }

  .meter.live {
    border-color: rgba(39, 224, 255, 0.4);
  }

  .meter .mono {
    flex: 1;
    text-align: right;
    font-family: var(--font-mono);
    font-size: 16px;
    font-variant-numeric: tabular-nums;
  }

  .toolrow {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 0 4px;
  }

  .toolrow .mono {
    font-family: var(--font-mono);
    font-size: 13px;
    color: var(--text);
  }

  .unit {
    font-size: 11px;
    color: var(--text-faint);
  }
</style>
