<script>
  import { telemetry, loadedInfo, activePanel, send } from './store.js'

  let state = $derived($telemetry?.state ?? 'estop')
  let prog = $derived($telemetry?.program)
  let progress = $derived(prog?.progress ?? 0)
  let canRun = $derived(state === 'idle' && !!$loadedInfo)
</script>

<footer class="panel">
  <button class="btn" onclick={() => activePanel.update((v) => (v === 'library' ? null : 'library'))}>
    ☰ Programs
  </button>
  <button class="btn" onclick={() => activePanel.update((v) => (v === 'digitize' ? null : 'digitize'))}>
    ⊹ Digitize
  </button>

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
    gap: 14px;
    padding: 12px 14px;
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
