<script>
  import { telemetry, send } from './store.js'

  const STEPS = [0.01, 0.1, 1, 10]
  let step = $state(1)
  let continuous = $state(false)
  let velocity = $state(3000)
  let mdiText = $state('')

  let canJog = $derived(['idle', 'jog'].includes($telemetry?.state))
  let canMdi = $derived($telemetry?.state === 'idle')

  function press(axis, dir) {
    if (continuous) {
      send({ cmd: 'jog', axis, dir, velocity })
    } else {
      send({ cmd: 'jog_step', axis, dir, step })
    }
  }

  function release(axis) {
    if (continuous) send({ cmd: 'jog', axis, dir: 0 })
  }

  function submitMdi(e) {
    e.preventDefault()
    const text = mdiText.trim()
    if (text) {
      send({ cmd: 'mdi', text })
      mdiText = ''
    }
  }
</script>

<div class="panel jog">
  <div class="head">
    <span class="label">Jog</span>
    <div class="steps">
      {#each STEPS as s}
        <button class="chip" class:active={!continuous && step === s} onclick={() => { step = s; continuous = false }}>
          {s}
        </button>
      {/each}
      <button class="chip" class:active={continuous} onclick={() => (continuous = true)}>∞</button>
    </div>
  </div>

  <div class="pad">
    <div class="xy">
      <button class="key up" disabled={!canJog} onpointerdown={() => press('y', 1)} onpointerup={() => release('y')} onpointerleave={() => release('y')}>Y+</button>
      <button class="key left" disabled={!canJog} onpointerdown={() => press('x', -1)} onpointerup={() => release('x')} onpointerleave={() => release('x')}>X−</button>
      <div class="hub"></div>
      <button class="key right" disabled={!canJog} onpointerdown={() => press('x', 1)} onpointerup={() => release('x')} onpointerleave={() => release('x')}>X+</button>
      <button class="key down" disabled={!canJog} onpointerdown={() => press('y', -1)} onpointerup={() => release('y')} onpointerleave={() => release('y')}>Y−</button>
    </div>
    <div class="zcol">
      <button class="key" disabled={!canJog} onpointerdown={() => press('z', 1)} onpointerup={() => release('z')} onpointerleave={() => release('z')}>Z+</button>
      <button class="key" disabled={!canJog} onpointerdown={() => press('z', -1)} onpointerup={() => release('z')} onpointerleave={() => release('z')}>Z−</button>
    </div>
  </div>

  {#if continuous}
    <div class="vel">
      <span class="label">Speed</span>
      <input type="range" min="100" max="8000" step="100" bind:value={velocity} />
      <span class="mono">{velocity}</span>
    </div>
  {/if}

  <form class="mdi" onsubmit={submitMdi}>
    <input
      type="text"
      placeholder="MDI — G0 X0 Y0"
      bind:value={mdiText}
      disabled={!canMdi}
      spellcheck="false"
      autocomplete="off"
    />
  </form>
</div>

<style>
  .jog {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
  }

  .steps {
    display: flex;
    gap: 4px;
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

  .pad {
    display: flex;
    gap: 14px;
    justify-content: center;
  }

  .xy {
    display: grid;
    grid-template-columns: repeat(3, 56px);
    grid-template-rows: repeat(3, 56px);
    gap: 6px;
  }

  .up { grid-area: 1 / 2; }
  .left { grid-area: 2 / 1; }
  .hub { grid-area: 2 / 2; }
  .right { grid-area: 2 / 3; }
  .down { grid-area: 3 / 2; }

  .hub {
    border-radius: 50%;
    border: 1px dashed var(--hairline-2);
    margin: 14px;
  }

  .zcol {
    display: flex;
    flex-direction: column;
    gap: 6px;
    justify-content: center;
  }

  .zcol .key {
    width: 56px;
    height: 56px;
  }

  .key {
    border-radius: var(--r-md);
    background: var(--panel-2);
    border: 1px solid var(--hairline);
    font-weight: 700;
    font-size: 14px;
    color: var(--text);
    touch-action: none;
    transition: background 120ms var(--ease), border-color 120ms var(--ease), transform 80ms var(--ease);
  }

  .key:active:not(:disabled) {
    background: var(--accent-dim);
    border-color: rgba(39, 224, 255, 0.5);
    color: var(--accent);
    transform: scale(0.95);
  }

  .key:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .vel {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .vel .mono {
    font-family: var(--font-mono);
    font-size: 12px;
    width: 42px;
    text-align: right;
    color: var(--text-dim);
  }

  .mdi input {
    width: 100%;
    padding: 10px 14px;
    border-radius: var(--r-md);
    background: var(--panel-2);
    border: 1px solid var(--hairline);
    font-family: var(--font-mono);
    font-size: 13px;
    outline: none;
    transition: border-color 150ms var(--ease);
  }

  .mdi input:focus {
    border-color: rgba(39, 224, 255, 0.5);
  }

  .mdi input:disabled {
    opacity: 0.4;
  }
</style>
