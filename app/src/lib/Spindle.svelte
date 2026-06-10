<script>
  import { telemetry, tools, send } from './store.js'

  let rpm = $state(8000)

  let spindle = $derived($telemetry?.spindle ?? { on: false, reverse: false })
  let coolant = $derived($telemetry?.coolant ?? false)
  let toolNo = $derived($telemetry?.tool?.number ?? 0)
  let canControl = $derived(['idle', 'paused', 'jog'].includes($telemetry?.state))
  let canTool = $derived(['idle', 'paused'].includes($telemetry?.state))
</script>

<div class="panel sp">
  <span class="label">Spindle · Coolant · Tool</span>

  <div class="row">
    <button
      class="btn dir"
      class:running={spindle.on && !spindle.reverse}
      disabled={!canControl}
      onclick={() => send({ cmd: 'spindle', on: !(spindle.on && !spindle.reverse), rpm, reverse: false })}
      title="M3 — spindle clockwise"
    >
      ⟳ M3
    </button>
    <button
      class="btn dir"
      class:running={spindle.on && spindle.reverse}
      disabled={!canControl}
      onclick={() => send({ cmd: 'spindle', on: !(spindle.on && spindle.reverse), rpm, reverse: true })}
      title="M4 — spindle counter-clockwise"
    >
      ⟲ M4
    </button>
    <button
      class="btn"
      disabled={!canControl || !spindle.on}
      onclick={() => send({ cmd: 'spindle', on: false, rpm: 0, reverse: false })}
      title="M5 — spindle stop"
    >
      ■ M5
    </button>
  </div>

  <div class="row">
    <span class="dim">S</span>
    <input type="range" min="500" max="24000" step="500" bind:value={rpm} />
    <span class="mono">{rpm}</span>
  </div>

  <div class="row">
    <button
      class="btn"
      class:running={coolant}
      disabled={!canControl}
      onclick={() => send({ cmd: 'coolant', on: !coolant })}
      title="M8 / M9"
    >
      ❄ Coolant
    </button>
    <select
      class="toolsel"
      disabled={!canTool}
      value={toolNo}
      onchange={(e) => send({ cmd: 'tool', number: +e.currentTarget.value })}
      title="Manual tool change (M6)"
    >
      <option value={0}>T—</option>
      {#each $tools as t (t.number)}
        <option value={t.number}>T{t.number} · Ø{t.diameter}</option>
      {/each}
    </select>
  </div>
</div>

<style>
  .sp {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .row .btn {
    flex: 1;
    padding: 8px 6px;
    font-size: 12px;
  }

  .btn.running {
    border-color: rgba(39, 224, 255, 0.5);
    color: var(--accent);
    background: var(--accent-dim);
  }

  .dim {
    font-size: 12px;
    color: var(--text-faint);
    font-family: var(--font-mono);
  }

  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
    width: 44px;
    text-align: right;
    color: var(--text-dim);
  }

  .toolsel {
    flex: 1;
    appearance: none;
    padding: 8px 12px;
    border-radius: var(--r-md);
    background: var(--panel-2);
    border: 1px solid var(--hairline);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 12px;
    cursor: pointer;
    outline: none;
  }

  .toolsel:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
