<script>
  import { telemetry } from './store.js'

  const AXES = ['x', 'y', 'z']

  let pos = $derived($telemetry?.position ?? { x: 0, y: 0, z: 0 })
  let homed = $derived($telemetry?.homed ?? false)
  let feed = $derived($telemetry?.feed?.actual ?? 0)
  let rpm = $derived($telemetry?.spindle?.rpm ?? 0)
  let spindleOn = $derived($telemetry?.spindle?.on ?? false)

  function fmt(v) {
    const s = v.toFixed(3)
    return s === '-0.000' ? '0.000' : s
  }
</script>

<div class="panel dro">
  <div class="head">
    <span class="label">Position</span>
    <span class="homed" class:ok={homed}>{homed ? 'HOMED' : 'NOT HOMED'}</span>
  </div>

  {#each AXES as axis}
    <div class="axis">
      <span class="name">{axis.toUpperCase()}</span>
      <span class="value">{fmt(pos[axis])}</span>
      <span class="unit">mm</span>
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

  .homed {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--warn);
  }

  .homed.ok {
    color: var(--text-faint);
  }

  .axis {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 6px 12px;
    border-radius: var(--r-md);
    background: var(--panel-2);
    border: 1px solid var(--hairline);
  }

  .name {
    width: 18px;
    font-size: 14px;
    font-weight: 700;
    color: var(--accent);
  }

  .value {
    flex: 1;
    text-align: right;
    font-family: var(--font-mono);
    font-size: 30px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.01em;
  }

  .unit {
    font-size: 11px;
    color: var(--text-faint);
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
</style>
