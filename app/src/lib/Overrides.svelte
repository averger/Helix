<script>
  import { telemetry, send } from './store.js'

  let feed = $derived(Math.round(($telemetry?.feed?.override ?? 1) * 100))
  let rapid = $derived(Math.round(($telemetry?.rapid_override ?? 1) * 100))
  let spindle = $derived(Math.round(($telemetry?.spindle_override ?? 1) * 100))

  function set(kind, percent) {
    send({ cmd: 'override', kind, value: percent / 100 })
  }
</script>

<div class="panel ovr">
  <span class="label">Overrides</span>
  {#each [['feed', 'Feed', feed], ['rapid', 'Rapid', rapid], ['spindle', 'Spindle', spindle]] as [kind, name, value]}
    <div class="row">
      <span class="name">{name}</span>
      <input
        type="range"
        min="0"
        max="200"
        step="5"
        {value}
        oninput={(e) => set(kind, +e.currentTarget.value)}
        ondblclick={() => set(kind, 100)}
      />
      <button class="pct" class:off={value !== 100} onclick={() => set(kind, 100)}>{value}%</button>
    </div>
  {/each}
</div>

<style>
  .ovr {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .name {
    width: 56px;
    font-size: 12px;
    color: var(--text-dim);
  }

  .pct {
    width: 52px;
    text-align: right;
    font-family: var(--font-mono);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: var(--text-faint);
  }

  .pct.off {
    color: var(--warn);
  }
</style>
