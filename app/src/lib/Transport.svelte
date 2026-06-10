<script>
  import { telemetry, loadedInfo, activePanel, send } from './store.js'

  let state = $derived($telemetry?.state ?? 'estop')
  let prog = $derived($telemetry?.program)
  let progress = $derived(prog?.progress ?? 0)
  let canRun = $derived(state === 'idle' && !!$loadedInfo)
  let singleBlock = $derived($telemetry?.single_block ?? false)
  let optionalStop = $derived($telemetry?.optional_stop ?? false)

  const togglePanel = (name) => activePanel.update((v) => (v === name ? null : name))
</script>

<footer class="panel">
  <button class="btn" onclick={() => togglePanel('library')}>☰ Programs</button>
  <button class="btn" onclick={() => togglePanel('code')}>⌘ Code</button>
  <button class="btn" onclick={() => togglePanel('tools')}>⛭ Tools</button>
  <button class="btn" onclick={() => togglePanel('digitize')}>⊹ Digitize</button>

  <div class="modes">
    <button
      class="chip"
      class:active={singleBlock}
      title="Single block — pause after every block"
      onclick={() => send({ cmd: 'single_block', on: !singleBlock })}
    >
      SBL
    </button>
    <button
      class="chip"
      class:active={optionalStop}
      title="Optional stop — honour M1"
      onclick={() => send({ cmd: 'optional_stop', on: !optionalStop })}
    >
      M1
    </button>
  </div>

  <div class="prog">
    {#if $loadedInfo}
      <div class="meta">
        <span class="name">{$loadedInfo.name}</span>
        <span class="detail">
          {#if state === 'running' || state === 'paused'}
            line {prog?.line ?? 0} / {$loadedInfo.lines} · {Math.round(progress * 100)}%
          {:else}
            {$loadedInfo.lines} lines · {$loadedInfo.length_mm} mm of travel
          {/if}
        </span>
      </div>
      <div class="track">
        <div class="fill" style="width: {progress * 100}%"></div>
      </div>
    {:else}
      <span class="empty">No program loaded — open the library</span>
    {/if}
  </div>

  {#if state === 'running'}
    <button class="btn" onclick={() => send({ cmd: 'pause' })}>⏸ Hold</button>
    <button class="btn danger" onclick={() => send({ cmd: 'stop' })}>⏹ Stop</button>
  {:else if state === 'paused'}
    <button class="btn primary" onclick={() => send({ cmd: 'resume' })}>⏵ Resume</button>
    <button class="btn danger" onclick={() => send({ cmd: 'stop' })}>⏹ Stop</button>
  {:else}
    <button class="btn primary run" disabled={!canRun} onclick={() => send({ cmd: 'run' })}>
      ⏵ Run
    </button>
  {/if}
</footer>

<style>
  footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
  }

  .modes {
    display: flex;
    gap: 4px;
  }

  .chip {
    padding: 6px 10px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 700;
    font-family: var(--font-mono);
    letter-spacing: 0.06em;
    color: var(--text-faint);
    border: 1px solid var(--hairline);
    transition: all 150ms var(--ease);
  }

  .chip.active {
    color: var(--warn);
    border-color: rgba(251, 191, 36, 0.45);
    background: rgba(251, 191, 36, 0.12);
  }

  .prog {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .meta {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
  }

  .name {
    font-weight: 600;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    white-space: nowrap;
  }

  .track {
    height: 4px;
    border-radius: 2px;
    background: var(--hairline);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    border-radius: 2px;
    box-shadow: 0 0 10px rgba(39, 224, 255, 0.6);
    transition: width 120ms linear;
  }

  .empty {
    font-size: 13px;
    color: var(--text-faint);
  }

  .run {
    min-width: 110px;
  }
</style>
