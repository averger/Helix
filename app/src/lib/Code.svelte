<script>
  import { telemetry, loadedInfo, activePanel, send, loadProgram } from './store.js'

  let source = $state('')
  let fetchedFor = $state(null)
  let scroller = $state(null)

  let lines = $derived(source ? source.split('\n') : [])
  let activeLine = $derived($telemetry?.program?.line ?? 0)
  let running = $derived(['running', 'paused'].includes($telemetry?.state))
  let canRun = $derived($telemetry?.state === 'idle' && !!$loadedInfo)
  let blockDelete = $derived($telemetry?.block_delete ?? false)

  $effect(() => {
    const name = $loadedInfo?.name
    if (!name || name === fetchedFor) return
    fetchedFor = name
    fetch(`/api/programs/${encodeURIComponent(name)}`)
      .then((r) => r.json())
      .then((d) => (source = d.source ?? ''))
      .catch(() => (source = ''))
  })

  // keep the active line in view while running
  $effect(() => {
    if (!running || !scroller || !activeLine) return
    const el = scroller.querySelector(`[data-line="${activeLine}"]`)
    el?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  })

  async function toggleBlockDelete() {
    send({ cmd: 'block_delete', on: !blockDelete })
    // load-time switch: re-parse the program with the new setting
    if ($loadedInfo && !running) {
      setTimeout(() => loadProgram($loadedInfo.name), 150)
    }
  }
</script>

<div class="drawer panel">
  <div class="head">
    <span class="label">Program</span>
    <button
      class="chip"
      class:active={blockDelete}
      title="Block delete — skip lines starting with /"
      onclick={toggleBlockDelete}
    >
      /‒ skip
    </button>
    <button class="close" onclick={() => activePanel.set(null)}>✕</button>
  </div>

  {#if $loadedInfo}
    <div class="code" bind:this={scroller}>
      {#each lines as text, i}
        {@const n = i + 1}
        <div class="ln" class:active={running && n === activeLine} data-line={n}>
          <button
            class="run-from"
            title="Run from line {n}"
            disabled={!canRun}
            onclick={() => send({ cmd: 'run', line: n })}
          >
            ⏵
          </button>
          <span class="no">{n}</span>
          <span class="src">{text}</span>
        </div>
      {/each}
    </div>
  {:else}
    <span class="empty">No program loaded</span>
  {/if}
</div>

<style>
  .drawer {
    position: absolute;
    top: 14px;
    right: 14px;
    bottom: 14px;
    width: 340px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    background: rgba(13, 15, 18, 0.88);
    backdrop-filter: blur(18px);
    -webkit-backdrop-filter: blur(18px);
    animation: slide-in 220ms var(--ease);
    z-index: 10;
  }

  @keyframes slide-in {
    from {
      transform: translateX(16px);
      opacity: 0;
    }
  }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .head .label {
    flex: 1;
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
    color: var(--warn);
    border-color: rgba(251, 191, 36, 0.45);
    background: rgba(251, 191, 36, 0.12);
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

  .code {
    flex: 1;
    overflow-y: auto;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.7;
  }

  .ln {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 6px;
    border-radius: 6px;
    white-space: pre;
  }

  .ln.active {
    background: var(--accent-dim);
  }

  .ln.active .src {
    color: var(--accent);
  }

  .run-from {
    width: 18px;
    color: transparent;
    font-size: 10px;
  }

  .ln:hover .run-from:not(:disabled) {
    color: var(--accent);
  }

  .no {
    width: 32px;
    text-align: right;
    color: var(--text-faint);
    user-select: none;
  }

  .src {
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .empty {
    color: var(--text-faint);
    font-size: 13px;
    padding: 8px;
  }
</style>
